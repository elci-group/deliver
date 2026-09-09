//! Fixture-driven conformance harness for Ogma.
//!
//! Discovers `fixtures/*/fixture.toml`, audits each fixture repository with
//! `ogma_core::run_audit` and compares the `AuditResult` against the
//! declarative expectations in the fixture's TOML file: baseline language and
//! confidence, the canonical locale set, per-locale native flags, the exact
//! violation-code sets per scope (global + each locale), the exact set of
//! files cited per global violation code, the framework list, and strings
//! that must not appear in any violation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ogma_model::{AuditResult, Confidence, OverallStatus, Policy};
use serde::Deserialize;

/// One loaded fixture: its directory, parsed policy and expectations.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub name: String,
    pub dir: PathBuf,
    pub policy: Policy,
    pub expect: Expect,
}

/// Declarative expectations from `fixture.toml`'s `[expect]` tables.
#[derive(Debug, Clone)]
pub struct Expect {
    /// Expected baseline language code, or `None` for "none" (undetected).
    pub baseline: Option<String>,
    pub baseline_confidence: Option<Confidence>,
    pub locales: Vec<String>,
    pub overall: OverallStatus,
    /// Locales expected to be native; every reported locale not listed here
    /// is expected to be non-native.
    pub native: BTreeSet<String>,
    /// Exact violation-code sets per scope: "global" or a locale canonical.
    pub violations: BTreeMap<String, BTreeSet<String>>,
    /// Exact set of repository-relative files cited per global code.
    pub violation_files: BTreeMap<String, BTreeSet<String>>,
    pub frameworks: Option<Vec<String>>,
    /// Text that must not appear in any violation message or detail.
    pub absent_text: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawFixture {
    #[allow(dead_code)]
    description: Option<String>,
    policy: Option<RawPolicy>,
    expect: Option<RawExpect>,
}

#[derive(Deserialize, Default)]
struct RawPolicy {
    baseline: Option<String>,
    required: Option<Vec<String>>,
    translation_coverage: Option<f64>,
    allow_fallback: Option<bool>,
    require_pluralisation: Option<bool>,
    require_locale_formatting: Option<bool>,
    require_accessibility: Option<bool>,
    allow_unknown: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawExpect {
    baseline: Option<String>,
    baseline_confidence: Option<String>,
    locales: Option<Vec<String>>,
    overall: Option<String>,
    native: Option<BTreeMap<String, bool>>,
    violations: Option<BTreeMap<String, Vec<String>>>,
    violation_files: Option<BTreeMap<String, Vec<String>>>,
    frameworks: Option<Vec<String>>,
    absent_text: Option<Vec<String>>,
}

/// The workspace `fixtures/` directory (two levels up from this crate).
pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Load every fixture directory containing a `fixture.toml`.
pub fn discover_fixtures() -> Vec<Fixture> {
    let root = fixtures_root();
    let mut out = Vec::new();
    let entries: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("fixture.toml").is_file())
        .collect();
    for dir in entries {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(load_fixture(&name, &dir));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Parse one fixture directory (must contain `fixture.toml`).
pub fn load_fixture(name: &str, dir: &Path) -> Fixture {
    let text = std::fs::read_to_string(dir.join("fixture.toml"))
        .unwrap_or_else(|e| panic!("cannot read {}/fixture.toml: {e}", dir.display()));
    let raw: RawFixture = toml::from_str(&text)
        .unwrap_or_else(|e| panic!("invalid TOML in {}/fixture.toml: {e}", dir.display()));

    let policy = build_policy(name, raw.policy);
    let expect = build_expect(name, raw.expect.unwrap_or_default());
    Fixture { name: name.to_string(), dir: dir.to_path_buf(), policy, expect }
}

/// Materialise a [`Policy`] from the optional `[policy]` overrides. Unset keys
/// keep `Policy::default()` by emitting no TOML for them and reusing
/// `ogma_policy::parse_policy` (an absent key never overrides the default).
fn build_policy(name: &str, raw: Option<RawPolicy>) -> Policy {
    let Some(raw) = raw else { return Policy::default() };
    let mut text = String::new();
    if let Some(baseline) = &raw.baseline {
        text.push_str("[application]\nbaseline = ");
        text.push_str(&serde_json::to_string(baseline).unwrap());
        text.push('\n');
    }
    if let Some(required) = &raw.required {
        text.push_str("[locales]\nrequired = ");
        text.push_str(&serde_json::to_string(required).unwrap());
        text.push('\n');
    }
    let mut reqs = String::new();
    if let Some(v) = raw.translation_coverage {
        reqs.push_str(&format!("translation_coverage = {v}\n"));
    }
    if let Some(v) = raw.allow_fallback {
        reqs.push_str(&format!("allow_fallback = {v}\n"));
    }
    if let Some(v) = raw.require_pluralisation {
        reqs.push_str(&format!("require_pluralisation = {v}\n"));
    }
    if let Some(v) = raw.require_locale_formatting {
        reqs.push_str(&format!("require_locale_formatting = {v}\n"));
    }
    if let Some(v) = raw.require_accessibility {
        reqs.push_str(&format!("require_accessibility = {v}\n"));
    }
    if let Some(v) = raw.allow_unknown {
        reqs.push_str(&format!("allow_unknown = {v}\n"));
    }
    if !reqs.is_empty() {
        text.push_str("[requirements]\n");
        text.push_str(&reqs);
    }
    ogma_policy::parse_policy(&text)
        .unwrap_or_else(|e| panic!("fixture {name}: invalid [policy]: {e}"))
}

fn parse_confidence(name: &str, raw: &str) -> Confidence {
    match raw.to_ascii_uppercase().as_str() {
        "UNKNOWN" => Confidence::Unknown,
        "LIKELY" => Confidence::Likely,
        "PROVEN" => Confidence::Proven,
        other => panic!("fixture {name}: unknown baseline_confidence {other:?}"),
    }
}

fn build_expect(name: &str, raw: RawExpect) -> Expect {
    let mut violations: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (scope, codes) in raw.violations.unwrap_or_default() {
        violations.insert(scope, codes.into_iter().collect());
    }
    let mut violation_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (code, files) in raw.violation_files.unwrap_or_default() {
        violation_files.insert(code, files.into_iter().collect());
    }
    Expect {
        baseline: match raw.baseline.as_deref() {
            None => panic!("fixture {name}: [expect].baseline is required"),
            Some("none") => None,
            Some(code) => Some(code.to_string()),
        },
        baseline_confidence: raw
            .baseline_confidence
            .map(|c| parse_confidence(name, &c)),
        locales: raw.locales.unwrap_or_default(),
        overall: match raw.overall.as_deref() {
            Some("PASS") => OverallStatus::Pass,
            Some("FAIL") => OverallStatus::Fail,
            Some("UNKNOWN") => OverallStatus::Unknown,
            other => panic!("fixture {name}: [expect].overall must be PASS|FAIL|UNKNOWN, got {other:?}"),
        },
        native: raw
            .native
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| k)
            .collect(),
        violations,
        violation_files,
        frameworks: raw.frameworks,
        absent_text: raw.absent_text.unwrap_or_default(),
    }
}

/// Audit a fixture with its declared policy and `incremental: false`.
pub fn audit_fixture(fx: &Fixture) -> AuditResult {
    ogma_core::run_audit(
        &fx.dir,
        &ogma_core::AuditConfig { policy: fx.policy.clone(), incremental: false },
    )
    .unwrap_or_else(|e| panic!("fixture {}: audit failed: {e}", fx.name))
}

fn codes(violations: &[ogma_model::Violation]) -> BTreeSet<String> {
    violations.iter().map(|v| v.code.clone()).collect()
}

fn fmt_set(set: &BTreeSet<String>) -> String {
    let mut v: Vec<_> = set.iter().cloned().collect();
    v.sort();
    format!("[{}]", v.join(", "))
}

/// Compare an audit result against a fixture's expectations. Returns the list
/// of mismatch descriptions; empty means full conformance.
fn push(out: &mut Vec<String>, what: &str, expected: &str, actual: &str) {
    out.push(format!("{what}: expected {expected}, actual {actual}"));
}

pub fn check(result: &AuditResult, fx: &Fixture) -> Vec<String> {
    let mut out = Vec::new();

    // Baseline language.
    let actual_baseline = result
        .baseline
        .language
        .as_ref()
        .map(|l| l.code().to_string());
    match (&fx.expect.baseline, &actual_baseline) {
        (None, None) => {}
        (Some(e), Some(a)) if e == a => {}
        (e, a) => push(
            &mut out,
            "baseline language",
            &e.clone().unwrap_or_else(|| "none".to_string()),
            &a.clone().unwrap_or_else(|| "none".to_string()),
        ),
    }
    if let Some(expected) = fx.expect.baseline_confidence {
        if result.baseline.confidence != expected {
            push(
                &mut out,
                "baseline confidence",
                &format!("{expected:?}"),
                &format!("{:?}", result.baseline.confidence),
            );
        }
    }

    // Canonical locale set.
    let actual_locales: BTreeSet<String> = result
        .locales
        .iter()
        .map(|l| l.locale.canonical.clone())
        .collect();
    let expected_locales: BTreeSet<String> = fx.expect.locales.iter().cloned().collect();
    if actual_locales != expected_locales {
        push(
            &mut out,
            "locale set",
            &fmt_set(&expected_locales),
            &fmt_set(&actual_locales),
        );
    }

    // Overall status.
    if result.overall != fx.expect.overall {
        push(
            &mut out,
            "overall status",
            &format!("{:?}", fx.expect.overall),
            &format!("{:?}", result.overall),
        );
    }

    // Per-locale native flags (native table lists the native ones; all other
    // reported locales must be non-native).
    for report in &result.locales {
        let canonical = report.locale.canonical.as_str();
        let expected_native = fx.expect.native.contains(canonical);
        if report.native != expected_native {
            push(
                &mut out,
                &format!("native[{canonical}]"),
                &expected_native.to_string(),
                &report.native.to_string(),
            );
        }
    }

    // Exact violation-code sets: "global" plus every reported locale scope.
    let mut actual_scopes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    actual_scopes.insert("global".to_string(), codes(&result.violations));
    for report in &result.locales {
        actual_scopes.insert(report.locale.canonical.clone(), codes(&report.violations));
    }
    let mut expected_scopes = fx.expect.violations.clone();
    expected_scopes.entry("global".to_string()).or_default();
    if expected_scopes.keys().cloned().collect::<BTreeSet<_>>() != actual_scopes.keys().cloned().collect() {
        push(
            &mut out,
            "violation scopes",
            &format!("{:?}", expected_scopes.keys().collect::<Vec<_>>()),
            &format!("{:?}", actual_scopes.keys().collect::<Vec<_>>()),
        );
    }
    for (scope, expected) in &expected_scopes {
        let actual = actual_scopes.get(scope).cloned().unwrap_or_default();
        if *expected != actual {
            push(&mut out, &format!("violations[{scope}]"), &fmt_set(expected), &fmt_set(&actual));
        }
    }

    // Exact file sets per global violation code.
    for (code, expected) in &fx.expect.violation_files {
        let actual: BTreeSet<String> = result
            .violations
            .iter()
            .filter(|v| v.code == *code)
            .filter_map(|v| v.file.as_ref())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        if *expected != actual {
            push(
                &mut out,
                &format!("violation files[{code}]"),
                &fmt_set(expected),
                &fmt_set(&actual),
            );
        }
    }

    // Frameworks.
    if let Some(expected) = &fx.expect.frameworks {
        let actual = result.frameworks.clone();
        if *expected != actual {
            push(
                &mut out,
                "frameworks",
                &format!("{expected:?}"),
                &format!("{actual:?}"),
            );
        }
    }

    // Forbidden text anywhere in violation messages or details.
    for needle in &fx.expect.absent_text {
        let mut found = Vec::new();
        let mut all: Vec<&ogma_model::Violation> = result.violations.iter().collect();
        for report in &result.locales {
            all.extend(report.violations.iter());
        }
        for v in all {
            if v.message.contains(needle.as_str())
                || v.details.iter().any(|d| d.contains(needle.as_str()))
            {
                found.push(format!("{} {}", v.code, v.message));
            }
        }
        if !found.is_empty() {
            push(
                &mut out,
                &format!("absent_text[{needle}]"),
                "absent from all violations",
                &format!("present in {}", found.join("; ")),
            );
        }
    }

    out
}

/// Render the deterministic JSON of an audit result (used for determinism
/// and golden-output comparisons).
pub fn render_json(result: &AuditResult) -> String {
    ogma_report::render_json(result)
}

/// Render the human-readable text report for an audit result.
pub fn render_text(result: &AuditResult, fx: &Fixture) -> String {
    ogma_report::render_text(result, &fx.policy)
}
