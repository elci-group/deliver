//! Catalogue discovery: candidate selection, locale inference, and parsing.

use std::path::Path;

use ogma_discovery::{FileClass, RepositoryIndex};
use ogma_model::{CatalogueFormat, LocaleId, StringResource};

use crate::parsers::{parse_catalogue, PLURAL_CATEGORIES};

/// Catalogues larger than this are skipped entirely (nothing recorded).
pub(crate) const MAX_CATALOGUE_BYTES: u64 = 2 * 1024 * 1024;

/// Extensions recognised as translation catalogues.
pub(crate) const CATALOGUE_EXTS: &[&str] = &[
    "json", "yaml", "yml", "toml", "po", "pot", "arb", "resx", "strings", "stringsdict",
    "properties", "csv", "xliff", "xlf",
];

/// Manifest names the indexer reports as [`FileClass::Resource`] but which
/// must not be treated as catalogues unless they live under an i18n-ish
/// directory.
pub(crate) const MANIFEST_EXCLUSIONS: &[&str] = &[
    "package.json",
    "package-lock.json",
    "cargo.toml",
    "cargo.lock",
    "pubspec.yaml",
    "pubspec.lock",
    "gemfile",
    "gemfile.lock",
    "composer.json",
    "composer.lock",
    "go.mod",
    "go.sum",
    "pom.xml",
    "build.gradle",
    "settings.gradle",
    "tsconfig.json",
    ".eslintrc.json",
];

/// Directory segments that mark an i18n location.
pub(crate) const I18N_SEGMENTS: &[&str] = &[
    "locales", "locale", "i18n", "l10n", "translations", "translation", "langs", "lang", "intl",
];

/// Discover and parse every translation catalogue in the index.
///
/// Only [`FileClass::Resource`] files with catalogue extensions are
/// candidates. Files over 2 MiB and manifest/dotfile names outside i18n
/// directories are skipped. Results are sorted by path; identical indexes
/// produce byte-identical output. Unparseable catalogue files are skipped
/// silently at this level (documented); per-unit structural problems are
/// reported by [`crate::compare_catalogues`] instead.
pub fn discover_catalogues(index: &RepositoryIndex, root: &Path) -> Vec<StringResource> {
    let mut resources = Vec::new();

    for file in &index.files {
        if file.class != FileClass::Resource {
            continue;
        }
        let name = file
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(ext) = file.path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if !CATALOGUE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if file.size > MAX_CATALOGUE_BYTES {
            continue;
        }
        if is_excluded_manifest(&name, &file.path) {
            continue;
        }

        let content = match ogma_discovery::read_file_limited(&root.join(&file.path), MAX_CATALOGUE_BYTES as usize) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let format = format_for_ext(&ext);
        let Some(mut parsed) = parse_catalogue(format.clone(), &content) else {
            continue;
        };

        let declared_locale = infer_declared_locale(&file.path);
        let effective = parsed.embedded_locale.clone().or(declared_locale.clone());
        for unit in &mut parsed.units {
            unit.locale = effective.clone();
        }

        resources.push(StringResource {
            path: file.path.clone(),
            format,
            declared_locale,
            embedded_locale: parsed.embedded_locale,
            units: parsed.units,
        });
    }

    resources.sort_by(|a, b| a.path.cmp(&b.path));
    resources
}

fn format_for_ext(ext: &str) -> CatalogueFormat {
    match ext {
        "json" => CatalogueFormat::Json,
        "yaml" | "yml" => CatalogueFormat::Yaml,
        "toml" => CatalogueFormat::Toml,
        "po" => CatalogueFormat::Po,
        "pot" => CatalogueFormat::Pot,
        "arb" => CatalogueFormat::Arb,
        "resx" => CatalogueFormat::Resx,
        "strings" => CatalogueFormat::AppleStrings,
        "stringsdict" => CatalogueFormat::AppleStringsdict,
        "properties" => CatalogueFormat::Properties,
        "csv" => CatalogueFormat::Csv,
        "xliff" | "xlf" => CatalogueFormat::Xliff,
        other => CatalogueFormat::Custom(other.to_string()),
    }
}

/// Manifest names and hidden dotfiles are excluded unless some parent
/// directory segment is i18n-ish.
fn is_excluded_manifest(name: &str, path: &Path) -> bool {
    let lower = name.to_ascii_lowercase();
    let excluded = MANIFEST_EXCLUSIONS.contains(&lower.as_str()) || lower.starts_with('.');
    if !excluded {
        return false;
    }
    !path
        .parent()
        .map(has_i18n_segment)
        .unwrap_or(false)
}

fn has_i18n_segment(dir: &Path) -> bool {
    dir.iter()
        .any(|s| I18N_SEGMENTS.contains(&s.to_string_lossy().to_ascii_lowercase().as_str()))
}

/// Locale inference from the repository-relative path, in priority order:
/// filename stem (whole, then expanding suffix tokens, then first token),
/// then parent directory (`.lproj` convention, or a locale-named directory
/// under an i18n-ish segment).
pub(crate) fn infer_declared_locale(path: &Path) -> Option<LocaleId> {
    let segments: Vec<String> = path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let filename = segments.last()?.clone();

    // 1. Filename stem.
    let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(&filename).to_string();
    if let Some(locale) = ogma_locale::parse_locale(&stem) {
        return Some(locale);
    }
    let tokens: Vec<&str> = stem.split(['_', '-']).collect();
    if tokens.len() > 1 {
        // Expanding suffixes from the last token leftwards (app-en-US → en-US).
        for k in 1..tokens.len() {
            let candidate = tokens[tokens.len() - k..].join("-");
            if let Some(locale) = ogma_locale::parse_locale(&candidate) {
                return Some(locale);
            }
        }
        if let Some(locale) = ogma_locale::parse_locale(tokens[0]) {
            return Some(locale);
        }
    }

    // 2. Parent directory, nearest first.
    for idx in (0..segments.len() - 1).rev() {
        let dir = &segments[idx];
        if let Some(base) = dir.strip_suffix(".lproj") {
            if let Some(locale) = ogma_locale::parse_locale(base) {
                return Some(locale);
            }
        }
        if let Some(locale) = ogma_locale::parse_locale(dir) {
            let under_i18n = segments[..idx]
                .iter()
                .any(|s| I18N_SEGMENTS.contains(&s.to_ascii_lowercase().as_str()));
            if under_i18n {
                return Some(locale);
            }
        }
    }

    None
}

/// True when `unit` carries a known CLDR plural category as its form.
pub(crate) fn is_named_plural(form: &Option<String>) -> bool {
    form.as_deref()
        .is_some_and(|f| PLURAL_CATEGORIES.contains(&f))
}
