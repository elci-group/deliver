//! Policy parsing and evaluation for Ogma.
//!
//! A policy file is TOML describing the conformance bar an application must
//! meet. Every section and key is optional; missing keys take
//! [`Policy::default`] values. Parsing never fails on unknown keys or unknown
//! top-level tables (forward compatibility); it fails with a
//! [`PolicyError`] (`invalid_config: true`) only on malformed values, which
//! callers map to exit code 3.

use std::fmt;
use std::path::Path;

use ogma_model::{BaselinePolicy, LocaleId, Policy};
use serde::Deserialize;

/// Configuration/policy error. `invalid_config` distinguishes exit-code-3
/// conditions (bad policy file) from analysis errors.
#[derive(Clone, Debug)]
pub struct PolicyError {
    pub message: String,
    pub invalid_config: bool,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PolicyError {}

impl PolicyError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            invalid_config: true,
        }
    }
}

/// Deserialisation shape. Unknown keys and unknown top-level tables are
/// ignored by serde by default, which is exactly the forward-compat behaviour
/// the policy format requires.
#[derive(Deserialize, Default)]
struct RawPolicy {
    #[serde(default)]
    application: RawApplication,
    #[serde(default)]
    locales: RawLocales,
    #[serde(default)]
    requirements: RawRequirements,
}

#[derive(Deserialize, Default)]
struct RawApplication {
    baseline: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawLocales {
    required: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct RawRequirements {
    translation_coverage: Option<f64>,
    allow_fallback: Option<bool>,
    require_pluralisation: Option<bool>,
    require_locale_formatting: Option<bool>,
    require_accessibility: Option<bool>,
    allow_unknown: Option<bool>,
}

/// Parse a policy from TOML text. An empty document yields
/// [`Policy::default`].
pub fn parse_policy(text: &str) -> Result<Policy, PolicyError> {
    let raw: RawPolicy = toml::from_str(text)
        .map_err(|e| PolicyError::invalid(format!("invalid policy TOML: {e}")))?;

    let mut policy = Policy::default();

    if let Some(baseline) = raw.application.baseline {
        if baseline.eq_ignore_ascii_case("auto") {
            policy.baseline = BaselinePolicy::Auto;
        } else {
            policy.baseline = BaselinePolicy::Explicit(
                ogma_locale::parse_locale(&baseline).ok_or_else(|| {
                    PolicyError::invalid(format!(
                        "application.baseline: unparseable locale {baseline:?}"
                    ))
                })?,
            );
        }
    }

    if let Some(required) = raw.locales.required {
        let mut locales: Vec<LocaleId> = Vec::new();
        for entry in &required {
            locales.push(ogma_locale::parse_locale(entry).ok_or_else(|| {
                PolicyError::invalid(format!(
                    "locales.required: unparseable locale entry {entry:?}"
                ))
            })?);
        }
        // Deterministic order; deduplicate by canonical form.
        locales.sort();
        locales.dedup_by(|a, b| a.canonical == b.canonical);
        policy.required_locales = locales;
    }

    if let Some(coverage) = raw.requirements.translation_coverage {
        if coverage.is_nan() || !(0.0..=1.0).contains(&coverage) {
            return Err(PolicyError::invalid(format!(
                "requirements.translation_coverage: {coverage} is outside [0.0, 1.0]"
            )));
        }
        policy.translation_coverage = coverage;
    }

    if let Some(allow_fallback) = raw.requirements.allow_fallback {
        policy.allow_fallback = allow_fallback;
    }
    if let Some(require_pluralisation) = raw.requirements.require_pluralisation {
        policy.require_pluralisation = require_pluralisation;
    }
    if let Some(require_locale_formatting) = raw.requirements.require_locale_formatting {
        policy.require_locale_formatting = require_locale_formatting;
    }
    if let Some(require_accessibility) = raw.requirements.require_accessibility {
        policy.require_accessibility = require_accessibility;
    }
    if let Some(allow_unknown) = raw.requirements.allow_unknown {
        policy.allow_unknown = allow_unknown;
    }

    Ok(policy)
}

/// Load and parse a policy file.
pub fn load_policy(path: &Path) -> Result<Policy, PolicyError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        PolicyError::invalid(format!(
            "cannot read policy file {}: {e}",
            path.display()
        ))
    })?;
    parse_policy(&text)
}

/// Compose two policies: `overlay` wins for scalars/enums; required_locales
/// are unioned (deduplicated, sorted); allow_unknown uses OR.
///
/// Because missing keys are materialised as `Policy::default()` during
/// parsing, an overlay value equal to the default is indistinguishable from
/// an absent key: merging always takes the overlay's scalar/enum values
/// outright, so a layer that explicitly writes a default value will reset the
/// composed value to the default. Layers that want to inherit a base value
/// must omit the key entirely.
pub fn merge(base: Policy, overlay: Policy) -> Policy {
    let mut required_locales = base.required_locales;
    required_locales.extend(overlay.required_locales);
    required_locales.sort();
    required_locales.dedup_by(|a, b| a.canonical == b.canonical);

    Policy {
        baseline: overlay.baseline,
        required_locales,
        translation_coverage: overlay.translation_coverage,
        allow_fallback: overlay.allow_fallback,
        require_pluralisation: overlay.require_pluralisation,
        require_locale_formatting: overlay.require_locale_formatting,
        require_accessibility: overlay.require_accessibility,
        allow_unknown: base.allow_unknown || overlay.allow_unknown,
    }
}

/// The default policy, serialised — used by `ogma init`.
pub const DEFAULT_POLICY_TOML: &str = r#"# Ogma policy file.
# Every key is optional; omitting a key keeps Ogma's default for it.

[application]
# The application's baseline (native) language. "auto" lets Ogma detect it
# deterministically from repository evidence; give an explicit locale (e.g.
# "en-GB") to pin it.
baseline = "auto"

[locales]
# Locales the application must natively support for the audit to pass.
required = []

[requirements]
# Fraction of the baseline linguistic surface each required locale must cover
# (0.0 to 1.0). 1.0 demands complete translation of every baseline key.
translation_coverage = 1.0
# Whether falling back to another language (instead of native support) is
# acceptable.
allow_fallback = false
# Whether each required locale must provide all plural forms its language
# requires.
require_pluralisation = true
# Whether user-facing dates, numbers and formatting must go through
# locale-aware APIs.
require_locale_formatting = true
# Whether accessibility-related i18n checks (a11y strings, accessible names)
# must pass.
require_accessibility = false
# Whether checks whose confidence is Unknown may count as passes. Defaults to
# false: unknown is never silently promoted to a pass.
allow_unknown = false
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use ogma_model::Language;

    fn loc(s: &str) -> LocaleId {
        ogma_locale::parse_locale(s).unwrap()
    }

    #[test]
    fn default_policy_toml_roundtrips_to_default() {
        assert_eq!(parse_policy(DEFAULT_POLICY_TOML).unwrap(), Policy::default());
    }

    #[test]
    fn empty_document_is_default() {
        assert_eq!(parse_policy("").unwrap(), Policy::default());
    }

    #[test]
    fn unknown_keys_and_tables_are_ignored() {
        let text = r#"
[application]
baseline = "auto"
some_future_key = 42

[locales]
required = ["en"]
other = "ignored"

[requirements]
allow_unknown = true
frobnicate = "yes"

[totally_new_table]
whatever = true
"#;
        let policy = parse_policy(text).unwrap();
        assert_eq!(policy.baseline, BaselinePolicy::Auto);
        assert_eq!(policy.required_locales, vec![loc("en")]);
        assert!(policy.allow_unknown);
        assert_eq!(policy.translation_coverage, 1.0);
    }

    #[test]
    fn wrong_value_types_are_invalid_config() {
        let err = parse_policy("[requirements]\ntranslation_coverage = \"high\"\n").unwrap_err();
        assert!(err.invalid_config);

        let err = parse_policy("[requirements]\nallow_fallback = \"yes\"\n").unwrap_err();
        assert!(err.invalid_config);

        let err = parse_policy("[application]\nbaseline = 42\n").unwrap_err();
        assert!(err.invalid_config);

        let err = parse_policy("[locales]\nrequired = \"en-GB\"\n").unwrap_err();
        assert!(err.invalid_config);
    }

    #[test]
    fn malformed_toml_is_invalid_config() {
        let err = parse_policy("[requirements\nbroken = ").unwrap_err();
        assert!(err.invalid_config);
    }

    #[test]
    fn baseline_auto_is_case_insensitive() {
        for text in ["[application]\nbaseline = \"auto\"\n", "[application]\nbaseline = \"AUTO\"\n"] {
            assert_eq!(parse_policy(text).unwrap().baseline, BaselinePolicy::Auto);
        }
    }

    #[test]
    fn baseline_explicit_parses_and_normalises() {
        let policy = parse_policy("[application]\nbaseline = \"EN_gb\"\n").unwrap();
        match &policy.baseline {
            BaselinePolicy::Explicit(l) => {
                assert_eq!(l.canonical, "en-GB");
                assert_eq!(l.language, Language::new("en"));
                assert_eq!(l.region.as_deref(), Some("GB"));
            }
            other => panic!("expected explicit baseline, got {other:?}"),
        }
    }

    #[test]
    fn baseline_unparseable_is_invalid_config() {
        let err = parse_policy("[application]\nbaseline = \"xx-nope\"\n").unwrap_err();
        assert!(err.invalid_config);
        assert!(err.message.contains("xx-nope"));
    }

    #[test]
    fn required_locales_sorted_and_deduplicated() {
        let text = "[locales]\nrequired = [\"fr-FR\", \"en-GB\", \"fr_fr\", \"en\"]\n";
        let policy = parse_policy(text).unwrap();
        let canonicals: Vec<&str> = policy
            .required_locales
            .iter()
            .map(|l| l.canonical.as_str())
            .collect();
        assert_eq!(canonicals, ["en", "en-GB", "fr-FR"]);
    }

    #[test]
    fn required_locale_entry_unparseable_lists_the_entry() {
        let text = "[locales]\nrequired = [\"en-GB\", \"not-a-locale\"]\n";
        let err = parse_policy(text).unwrap_err();
        assert!(err.invalid_config);
        assert!(err.message.contains("not-a-locale"));
    }

    #[test]
    fn coverage_out_of_bounds_is_invalid_config() {
        for text in [
            "[requirements]\ntranslation_coverage = 1.5\n",
            "[requirements]\ntranslation_coverage = -0.1\n",
        ] {
            assert!(parse_policy(text).unwrap_err().invalid_config);
        }
    }

    #[test]
    fn coverage_boundary_values_accepted() {
        for (text, expected) in [
            ("[requirements]\ntranslation_coverage = 0.0\n", 0.0),
            ("[requirements]\ntranslation_coverage = 1.0\n", 1.0),
        ] {
            assert_eq!(parse_policy(text).unwrap().translation_coverage, expected);
        }
    }

    #[test]
    fn coverage_in_range_accepted() {
        let policy = parse_policy("[requirements]\ntranslation_coverage = 0.75\n").unwrap();
        assert_eq!(policy.translation_coverage, 0.75);
    }

    #[test]
    fn load_policy_missing_file_is_invalid_config_with_path() {
        let path = Path::new("/nonexistent/definitely-missing-policy.toml");
        let err = load_policy(path).unwrap_err();
        assert!(err.invalid_config);
        assert!(err.message.contains(path.display().to_string().as_str()));
    }

    #[test]
    fn load_policy_reads_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ogma-policy-test-{}.toml", std::process::id()));
        std::fs::write(&path, "[locales]\nrequired = [\"de-DE\"]\n")?;
        let policy = load_policy(&path)?;
        std::fs::remove_file(&path)?;
        assert_eq!(policy.required_locales, vec![loc("de-DE")]);
        assert_eq!(policy, Policy {
            required_locales: vec![loc("de-DE")],
            ..Policy::default()
        });
        Ok(())
    }

    #[test]
    fn merge_scalar_overlay_wins() {
        let mut base = Policy::default();
        base.translation_coverage = 0.5;
        base.allow_fallback = true;
        base.require_pluralisation = false;

        let mut overlay = Policy::default();
        overlay.translation_coverage = 0.9;
        overlay.require_accessibility = true;

        let merged = merge(base.clone(), overlay.clone());
        assert_eq!(merged.translation_coverage, 0.9);
        assert!(merged.require_accessibility);
        // Overlay wins outright for scalars, even where it holds the default.
        assert!(!merged.allow_fallback);
        assert!(merged.require_pluralisation);
    }

    #[test]
    fn merge_unions_and_dedupes_required_locales() {
        let mut base = Policy::default();
        base.required_locales = vec![loc("fr-FR"), loc("en-GB")];
        let mut overlay = Policy::default();
        overlay.required_locales = vec![loc("de-DE"), loc("fr_fr")];

        let merged = merge(base, overlay);
        let canonicals: Vec<&str> = merged
            .required_locales
            .iter()
            .map(|l| l.canonical.as_str())
            .collect();
        assert_eq!(canonicals, ["de-DE", "en-GB", "fr-FR"]);
    }

    #[test]
    fn merge_allow_unknown_uses_or() {
        let mut base = Policy::default();
        base.allow_unknown = true;
        let overlay = Policy::default();
        assert!(merge(base.clone(), overlay).allow_unknown);
        assert!(merge(Policy::default(), base).allow_unknown);
        assert!(!merge(Policy::default(), Policy::default()).allow_unknown);
    }

    #[test]
    fn merge_baseline_overlay_wins() {
        let mut overlay = Policy::default();
        overlay.baseline = BaselinePolicy::Explicit(loc("en-GB"));
        let merged = merge(Policy::default(), overlay);
        assert_eq!(merged.baseline, BaselinePolicy::Explicit(loc("en-GB")));
    }

    #[test]
    fn error_display_and_std_error() {
        let err = PolicyError {
            message: "boom".to_string(),
            invalid_config: true,
        };
        assert_eq!(err.to_string(), "boom");
        let _: &dyn std::error::Error = &err;
    }
}
