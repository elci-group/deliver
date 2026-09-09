//! Audit pipeline orchestration for Ogma.
//!
//! [`run_audit`] wires discovery, source analysis, catalogue analysis and
//! conformance evaluation into a single [`ogma_model::AuditResult`]. Every
//! stage output is deterministically ordered; identical (tree, config, Ogma
//! version) inputs produce identical results.
//!
//! When `AuditConfig::incremental` is set, string-extraction results are
//! cached in `root/.ogma/` keyed by content hash, so unchanged files are not
//! re-analysed. Cache failures never fail an audit.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ogma_discovery::{read_file_limited, FileClass, IndexedFile, RepositoryIndex};
use ogma_model::{
    codes, AuditResult, BaselinePolicy, CatalogueFormat, Confidence, EvidenceKind,
    EvidenceStrength, LanguageEvidence, LinguisticSurface, LocaleId, OverallStatus, Policy,
    Severity, StringKind, StringOccurrence, StringResource, Violation,
};
use ogma_parser::{analyse_semantics, detect_frameworks, detect_locale_config, extract_strings,
    guess_text_language};
use ogma_strings::{discover_catalogues, units_by_key};
use serde::{Deserialize, Serialize};

const CACHE_DIR: &str = ".ogma";
const CACHE_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 1_048_576;

/// Configuration for one audit run.
#[derive(Clone, Debug)]
pub struct AuditConfig {
    pub policy: Policy,
    /// Use the .ogma/ incremental cache (default true).
    pub incremental: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self { policy: Policy::default(), incremental: true }
    }
}

/// Failure modes of the audit pipeline.
#[derive(Clone, Debug)]
pub enum AuditError {
    /// Unreadable/invalid repository or unrecoverable analysis failure.
    Analysis(String),
    Io(String),
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditError::Analysis(m) => write!(f, "analysis error: {m}"),
            AuditError::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// Run the full deterministic audit pipeline. Results are deterministic for
/// identical (tree, config, Ogma version).
pub fn run_audit(root: &Path, config: &AuditConfig) -> Result<AuditResult, AuditError> {
    let index = ogma_discovery::scan(root)
        .map_err(|e| AuditError::Analysis(e.to_string()))?;

    let frameworks = detect_frameworks(&index, root);
    let (occurrences, cache) = extract_with_cache(&index, root, config.incremental);
    let locale_hints = detect_locale_config(&index, root);
    let catalogues = discover_catalogues(&index, root);

    // Supported locales: union of catalogue effective locales, config-hint
    // locales and policy-required locales, deduplicated by canonical form.
    let mut supported: BTreeSet<LocaleId> = BTreeSet::new();
    for c in &catalogues {
        if let Some(l) = c.effective_locale() {
            supported.insert(l.clone());
        }
    }
    for h in &locale_hints {
        supported.insert(h.locale.clone());
    }
    for l in &config.policy.required_locales {
        supported.insert(l.clone());
    }

    let evidence = build_evidence(&occurrences, &locale_hints, &catalogues);
    let detection = ogma_conformance::detect_baseline(&evidence, &config.policy);

    // The working baseline used for coverage math. When automatic detection
    // is ambiguous the fallback `en` locale is used here only; the ambiguity
    // remains visible in the returned BaselineDetection.
    let baseline_locale = match &config.policy.baseline {
        BaselinePolicy::Explicit(loc) => loc.clone(),
        BaselinePolicy::Auto => match &detection.language {
            Some(lang) => ogma_locale::parse_locale(lang.code())
                .unwrap_or_else(|| LocaleId::canonical_only(lang.code())),
            None => LocaleId::canonical_only("en"),
        },
    };

    // The baseline locale always participates in conformance evaluation so
    // the coverage math has a surface even for apps with no catalogues.
    supported.insert(baseline_locale.clone());
    let supported_locales: Vec<LocaleId> = supported.into_iter().collect();

    let rtl_required = supported_locales
        .iter()
        .any(|l| ogma_locale::profile_for(&l.language).direction == ogma_locale::TextDirection::Rtl);
    let semantic = analyse_semantics(&index, root, rtl_required);

    let baseline_catalogue = select_baseline_catalogue(&catalogues, &baseline_locale);

    let source_keys: BTreeSet<String> = occurrences
        .iter()
        .filter(|o| o.translated)
        .map(|o| o.text.clone())
        .collect();
    let hard_coded_aria = occurrences
        .iter()
        .filter(|o| o.rule == "ui/aria-label" && !o.translated)
        .count();

    let conformance_in = ogma_conformance::ConformanceInput {
        baseline_locale: baseline_locale.clone(),
        baseline_catalogue: &baseline_catalogue,
        catalogues: &catalogues,
        source_keys: &source_keys,
        supported_locales: &supported_locales,
        semantic_violations: &semantic,
        hard_coded_aria_labels: hard_coded_aria,
        policy: &config.policy,
    };
    let output = ogma_conformance::evaluate(&conformance_in);

    let application_name = detect_application_name(&index, root);

    let user_facing: Vec<&StringOccurrence> = occurrences
        .iter()
        .filter(|o| o.kind == StringKind::UserFacing)
        .collect();
    let hard_coded: Vec<&StringOccurrence> =
        user_facing.iter().copied().filter(|o| !o.translated).collect();

    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for o in &occurrences {
        *by_kind.entry(kind_name(o.kind).to_string()).or_insert(0) += 1;
    }

    let linguistic_surface = LinguisticSurface {
        total_units: units_by_key(&baseline_catalogue).len(),
        user_facing_occurrences: user_facing.len(),
        hard_coded_occurrences: hard_coded.len(),
        by_kind,
    };

    let global_violations = aggregate_hard_coded(&occurrences);

    let overall = if output.locales.iter().any(|r| !r.native)
        || global_violations.iter().any(|v| v.severity == Severity::Error)
    {
        OverallStatus::Fail
    } else {
        OverallStatus::Pass
    };

    let result = AuditResult {
        application_name,
        baseline: detection,
        frameworks: frameworks.iter().map(|f| f.name.clone()).collect(),
        locales: output.locales,
        linguistic_surface,
        violations: global_violations,
        overall,
    };

    if let Some(cache) = cache {
        write_cache(root, &cache);
    }

    Ok(result)
}

/// Deterministic snake_case name of a [`StringKind`], matching its serde form.
fn kind_name(kind: StringKind) -> &'static str {
    match kind {
        StringKind::UserFacing => "user_facing",
        StringKind::DeveloperFacing => "developer_facing",
        StringKind::Internal => "internal",
        StringKind::TestOnly => "test_only",
        StringKind::Debug => "debug",
        StringKind::Generated => "generated",
        StringKind::Unknown => "unknown",
    }
}

/// Build the deterministic, sorted baseline-language evidence vector.
fn build_evidence(
    occurrences: &[StringOccurrence],
    hints: &[ogma_parser::LocaleConfigHint],
    catalogues: &[StringResource],
) -> Vec<LanguageEvidence> {
    let mut evidence = Vec::new();

    for h in hints {
        let (kind, strength) = if h.is_default {
            (EvidenceKind::DefaultLocaleDeclaration, EvidenceStrength::Strong)
        } else {
            (EvidenceKind::FallbackConfiguration, EvidenceStrength::Medium)
        };
        evidence.push(LanguageEvidence {
            source: format!("{}:{}", h.path.display(), h.key),
            kind,
            strength,
            language: h.locale.language.clone(),
            detail: format!("default locale declared as {}", h.locale.canonical),
        });
    }

    // Script guess over hard-coded user-facing source strings.
    let mut texts: Vec<&str> = occurrences
        .iter()
        .filter(|o| o.kind == StringKind::UserFacing && !o.translated)
        .map(|o| o.text.as_str())
        .collect();
    texts.sort();
    texts.truncate(200);
    if !texts.is_empty() {
        let joined = texts.join("\n");
        if let Some(lang) = guess_text_language(&joined) {
            let n = texts.len();
            evidence.push(LanguageEvidence {
                source: format!("{n} user-facing source strings"),
                kind: EvidenceKind::SourceStrings,
                strength: EvidenceStrength::Medium,
                language: lang.clone(),
                detail: format!("script analysis of {n} hard-coded user-facing strings"),
            });
        }
    }

    let mut seen: BTreeSet<(&PathBuf, &str)> = BTreeSet::new();
    for c in catalogues {
        let Some(l) = c.effective_locale() else { continue };
        if seen.insert((&c.path, l.canonical.as_str())) {
            evidence.push(LanguageEvidence {
                source: c.path.display().to_string(),
                kind: EvidenceKind::ResourceNaming,
                strength: EvidenceStrength::Weak,
                language: l.language.clone(),
                detail: format!("catalogue declared for {}", l.canonical),
            });
        }
    }

    evidence.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| evidence_kind_name(&a.kind).cmp(evidence_kind_name(&b.kind)))
    });
    evidence
}

fn evidence_kind_name(kind: &EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::LocaleCatalogue => "locale_catalogue",
        EvidenceKind::DefaultLocaleDeclaration => "default_locale_declaration",
        EvidenceKind::FallbackConfiguration => "fallback_configuration",
        EvidenceKind::HtmlLangAttribute => "html_lang_attribute",
        EvidenceKind::SourceStrings => "source_strings",
        EvidenceKind::Readme => "readme",
        EvidenceKind::ResourceNaming => "resource_naming",
        EvidenceKind::FrameworkConfiguration => "framework_configuration",
        EvidenceKind::ApplicationMetadata => "application_metadata",
    }
}

/// Choose the catalogue that defines the baseline linguistic surface:
/// exact canonical match, then same language, then largest, then empty.
fn select_baseline_catalogue(catalogues: &[StringResource], baseline: &LocaleId) -> StringResource {
    if let Some(c) = catalogues.iter().find(|c| {
        c.effective_locale().is_some_and(|l| l.canonical == baseline.canonical)
    }) {
        return c.clone();
    }
    if let Some(c) = catalogues.iter().find(|c| {
        c.effective_locale().is_some_and(|l| l.language == baseline.language)
    }) {
        return c.clone();
    }
    if let Some(c) = catalogues.iter().max_by_key(|c| c.units.len()) {
        return c.clone();
    }
    StringResource {
        path: PathBuf::from("<none>"),
        format: CatalogueFormat::Custom("none".into()),
        declared_locale: None,
        embedded_locale: None,
        units: vec![],
    }
}

/// Application name from manifests, in Cargo.toml → package.json →
/// pubspec.yaml priority order; first match wins.
fn detect_application_name(index: &RepositoryIndex, root: &Path) -> Option<String> {
    let manifests = index.manifests();
    let mut cargo: Vec<_> = manifests
        .iter()
        .filter(|m| file_name(&m.path) == "Cargo.toml")
        .collect();
    let mut pkg: Vec<_> = manifests
        .iter()
        .filter(|m| file_name(&m.path) == "package.json")
        .collect();
    let mut pubspec: Vec<_> = manifests
        .iter()
        .filter(|m| file_name(&m.path) == "pubspec.yaml")
        .collect();
    cargo.sort_by_key(|m| m.path.clone());
    pkg.sort_by_key(|m| m.path.clone());
    pubspec.sort_by_key(|m| m.path.clone());

    for m in &cargo {
        if let Some(name) = cargo_package_name(root, &m.path) {
            return Some(name);
        }
    }
    for m in &pkg {
        if let Some(content) = read_bounded(root, &m.path) {
            if let Some(name) = json_string_field(&content, "name") {
                return Some(name);
            }
        }
    }
    for m in &pubspec {
        if let Some(content) = read_bounded(root, &m.path) {
            for line in content.lines() {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix("name:") {
                    let v = rest.trim().trim_matches(['"', '\'']);
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// `name = "..."` inside the `[package]` table of a Cargo.toml.
fn cargo_package_name(root: &Path, rel: &Path) -> Option<String> {
    let content = read_bounded(root, rel)?;
    let mut in_package = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let v = rest.trim().trim_matches('"').trim_matches('\'');
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// First `"key": "value"` string field in JSON content (line scanning).
fn json_string_field(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in content.lines() {
        let mut rest = line.trim_start();
        while let Some(p) = rest.find(&needle) {
            let after = rest[p + needle.len()..].trim_start();
            if let Some(after) = after.strip_prefix(':') {
                let v = after.trim().trim_start_matches('"');
                let end = v.find('"').unwrap_or(v.len());
                let v = &v[..end];
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
            rest = &rest[p + needle.len()..];
        }
    }
    None
}

/// Aggregate hard-coded user-facing occurrences into one violation per file.
fn aggregate_hard_coded(occurrences: &[StringOccurrence]) -> Vec<Violation> {
    let mut by_file: BTreeMap<&Path, Vec<&StringOccurrence>> = BTreeMap::new();
    for o in occurrences {
        if o.kind == StringKind::UserFacing && !o.translated {
            by_file.entry(&o.file).or_default().push(o);
        }
    }

    let mut out = Vec::new();
    for (file, mut occs) in by_file {
        occs.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.text.cmp(&b.text)));
        let first_line = occs[0].line;
        let total = occs.len();
        let mut details: Vec<String> = occs
            .iter()
            .take(20)
            .map(|o| format!("line {}: {}", o.line, truncate(&o.text, 48)))
            .collect();
        if total > 20 {
            details.push(format!("... {} more", total - 20));
        }
        out.push(Violation {
            code: codes::HARD_CODED_STRING.to_string(),
            severity: Severity::Warning,
            message: format!("{total} hard-coded user-facing strings"),
            file: Some(file.to_path_buf()),
            line: Some(first_line),
            locale: None,
            confidence: Confidence::Likely,
            details,
        });
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn read_bounded(root: &Path, rel: &Path) -> Option<String> {
    read_file_limited(&root.join(rel), MAX_MANIFEST_BYTES).ok()
}

// ---------------------------------------------------------------------------
// Incremental cache
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CacheState {
    version: u32,
    files: BTreeMap<String, u64>,
}

type OccurrenceCache = BTreeMap<String, Vec<StringOccurrence>>;

/// Extract string occurrences, reusing `root/.ogma/` cached results for
/// unchanged Source files when `incremental` is set. Returns the merged,
/// deterministically sorted occurrences plus the new cache state to persist
/// (None when caching is disabled or unavailable).
fn extract_with_cache(
    index: &RepositoryIndex,
    root: &Path,
    incremental: bool,
) -> (Vec<StringOccurrence>, Option<(CacheState, OccurrenceCache)>) {
    let source_files: Vec<&IndexedFile> = index.by_class(FileClass::Source);

    if !incremental {
        return (extract_strings(index, root), None);
    }

    let cache_dir = root.join(CACHE_DIR);
    if fs::create_dir_all(&cache_dir).is_err() {
        return (extract_strings(index, root), None);
    }

    let (hashes, cached_occurrences) = load_cache(&cache_dir);

    let mut uncached_files: Vec<IndexedFile> = Vec::new();
    let mut new_hashes: BTreeMap<String, u64> = BTreeMap::new();
    let mut new_occurrences: OccurrenceCache = BTreeMap::new();

    for f in &source_files {
        let key = f.path.to_string_lossy().replace('\\', "/");
        new_hashes.insert(key.clone(), f.hash);
        let hit = hashes
            .get(&key)
            .is_some_and(|h| *h == f.hash)
            && cached_occurrences.contains_key(&key);
        if hit {
            new_occurrences.insert(key.clone(), cached_occurrences[&key].clone());
        } else {
            // Re-extract; any previous entry under this key is replaced.
            uncached_files.push((*f).clone());
        }
    }
    // Cached entries for files no longer in the index are dropped: they are
    // never re-inserted into `new_occurrences`.

    if !uncached_files.is_empty() {
        let partial = RepositoryIndex { root: index.root.clone(), files: uncached_files };
        for o in extract_strings(&partial, root) {
            new_occurrences
                .entry(o.file.to_string_lossy().replace('\\', "/"))
                .or_default()
                .push(o);
        }
    }

    let mut merged: Vec<StringOccurrence> = Vec::new();
    for (_, occs) in new_occurrences.iter_mut() {
        // Re-sort so cached and freshly extracted vectors share one order.
        occs.sort_by(|a, b| {
            (&a.file, a.line, &a.text, &a.rule).cmp(&(&b.file, b.line, &b.text, &b.rule))
        });
        merged.extend(occs.iter().cloned());
    }
    merged.sort_by(|a, b| {
        (&a.file, a.line, &a.text, &a.rule).cmp(&(&b.file, b.line, &b.text, &b.rule))
    });

    (
        merged,
        Some((CacheState { version: CACHE_VERSION, files: new_hashes }, new_occurrences)),
    )
}

fn load_cache(cache_dir: &Path) -> (BTreeMap<String, u64>, OccurrenceCache) {
    let hashes = read_file_limited(&cache_dir.join("cache.json"), MAX_MANIFEST_BYTES)
        .ok()
        .and_then(|s| serde_json::from_str::<CacheState>(&s).ok())
        .filter(|c| c.version == CACHE_VERSION)
        .map(|c| c.files)
        .unwrap_or_default();
    let occurrences = read_file_limited(&cache_dir.join("occurrences.json"), MAX_MANIFEST_BYTES)
        .ok()
        .and_then(|s| serde_json::from_str::<OccurrenceCache>(&s).ok())
        .unwrap_or_default();
    (hashes, occurrences)
}

fn write_cache(root: &Path, cache: &(CacheState, OccurrenceCache)) {
    let cache_dir = root.join(CACHE_DIR);
    let Ok(cache_json) = serde_json::to_string_pretty(&cache.0) else { return };
    let Ok(occ_json) = serde_json::to_string_pretty(&cache.1) else { return };
    let _ = fs::write(cache_dir.join("cache.json"), cache_json);
    let _ = fs::write(cache_dir.join("occurrences.json"), occ_json);
}
