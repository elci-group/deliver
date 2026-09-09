//! Canonical domain types for Ogma.
//!
//! These types are the stable contract between every Ogma crate. All
//! collections exposed in analysis results use ordered containers so that
//! serialized output is deterministic for identical inputs.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A human language identifier (ISO 639-1 two-letter code, lower-case).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Language(pub String);

impl Language {
    pub fn new(code: &str) -> Self {
        Self(code.to_ascii_lowercase())
    }
    pub fn code(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A locale identifier in its normalised canonical form (`en-GB`) together
/// with the original declaration as found in the repository (`en_GB`, `en-gb`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LocaleId {
    /// The declaration exactly as discovered.
    pub original: String,
    /// Canonical BCP-47-like form: lowercase language, Title script,
    /// UPPER region, all joined with `-`.
    pub canonical: String,
    /// The primary language subtag.
    pub language: Language,
    /// ISO 15924 script subtag, if present.
    pub script: Option<String>,
    /// ISO 3166-1 / UN M.49 region subtag, if present.
    pub region: Option<String>,
}

impl LocaleId {
    pub fn canonical_only(canonical: &str) -> Self {
        Self {
            original: canonical.to_string(),
            canonical: canonical.to_string(),
            language: Language::new(canonical.split('-').next().unwrap_or(canonical)),
            script: None,
            region: None,
        }
    }
}

impl Ord for LocaleId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical.cmp(&other.canonical)
    }
}
impl PartialOrd for LocaleId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical)
    }
}

/// Deterministic confidence levels. `Unknown` must never be silently
/// promoted to a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Confidence {
    #[default]
    Unknown,
    Likely,
    Proven,
}

/// Weight class for baseline-language evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceStrength {
    Weak,
    Medium,
    Strong,
}

/// Where a piece of baseline-language evidence came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    LocaleCatalogue,
    DefaultLocaleDeclaration,
    FallbackConfiguration,
    HtmlLangAttribute,
    SourceStrings,
    Readme,
    ResourceNaming,
    FrameworkConfiguration,
    ApplicationMetadata,
}

/// One piece of deterministic evidence contributing to baseline detection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageEvidence {
    /// Repository-relative path or configuration key the evidence came from.
    pub source: String,
    pub kind: EvidenceKind,
    pub strength: EvidenceStrength,
    pub language: Language,
    /// Human-readable detail, e.g. "1,482 source strings".
    pub detail: String,
}

/// The result of baseline (application-language) detection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BaselineDetection {
    pub language: Option<Language>,
    pub confidence: Confidence,
    pub evidence: Vec<LanguageEvidence>,
}

/// Classification of a string literal's audience.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StringKind {
    UserFacing,
    DeveloperFacing,
    Internal,
    TestOnly,
    Debug,
    Generated,
    Unknown,
}

/// A single occurrence of a string literal in source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StringOccurrence {
    /// Repository-relative path.
    pub file: PathBuf,
    pub line: usize,
    pub text: String,
    pub kind: StringKind,
    pub confidence: Confidence,
    /// True when the string flows through the application's i18n lookup
    /// (`t("key")`, `gettext`, `tr!`, ...) rather than being hard-coded.
    pub translated: bool,
    /// Identifier of the extraction rule that produced this occurrence.
    pub rule: String,
}

/// Supported translation resource formats.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogueFormat {
    Json,
    Yaml,
    Toml,
    Po,
    Pot,
    Xliff,
    Arb,
    Resx,
    AppleStrings,
    AppleStringsdict,
    Properties,
    Csv,
    Custom(String),
}

/// A single translation unit inside a catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationUnit {
    pub key: String,
    /// Raw value; `None` when the unit is a key-only declaration.
    pub value: Option<String>,
    /// Locale the unit belongs to, if the catalogue is locale-specific.
    pub locale: Option<LocaleId>,
    /// CLDR plural category (`one`, `few`, `many`, `other`) when this unit
    /// is one arm of a plural form.
    pub plural_form: Option<String>,
    /// Interpolation placeholders used by the value (`{name}`, `%s`, `{{v}}`).
    pub placeholders: Vec<String>,
}

/// A parsed translation catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StringResource {
    /// Repository-relative path.
    pub path: PathBuf,
    pub format: CatalogueFormat,
    /// Locale declared by file naming/convention.
    pub declared_locale: Option<LocaleId>,
    /// Locale declared inside the file (`"@@locale": "fr"` in ARB).
    pub embedded_locale: Option<LocaleId>,
    pub units: Vec<TranslationUnit>,
}

impl StringResource {
    /// The effective locale for this catalogue.
    pub fn effective_locale(&self) -> Option<&LocaleId> {
        self.embedded_locale.as_ref().or(self.declared_locale.as_ref())
    }
}

/// Why a fallback occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    /// `fr-CA` falling back to `fr`: acceptable regional fallback.
    Regional,
    /// A defined chain of increasingly general locales.
    LinguisticChain,
    /// A key exists in the fallback locale but not the requested one.
    MissingTranslation,
    /// Fall back to the application's primary language.
    PrimaryLanguage,
}

/// A fallback relationship discovered statically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackRule {
    pub requested: LocaleId,
    pub target: LocaleId,
    pub kind: FallbackKind,
}

impl FallbackRule {
    /// Whether this fallback crosses into another primary language.
    pub fn crosses_language(&self) -> bool {
        self.requested.language != self.target.language
    }
}

/// Severity of a conformance finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A deterministic conformance violation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    /// Stable machine code, e.g. `OGMA-I18N-001`.
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub locale: Option<LocaleId>,
    pub confidence: Confidence,
    /// Optional extra lines rendered beneath the finding by `ogma explain`.
    pub details: Vec<String>,
}

/// Canonical violation codes used by Ogma engines.
pub mod codes {
    /// Hard-coded user-facing string bypassing the i18n system.
    pub const HARD_CODED_STRING: &str = "OGMA-I18N-001";
    /// Missing translation for a required key.
    pub const MISSING_TRANSLATION: &str = "OGMA-I18N-002";
    /// Orphaned catalogue key with no source reference.
    pub const ORPHANED_KEY: &str = "OGMA-I18N-003";
    /// Missing plural form for a locale that requires it.
    pub const MISSING_PLURAL_FORM: &str = "OGMA-I18N-004";
    /// Placeholder set differs between baseline and target locale.
    pub const INCONSISTENT_PLACEHOLDERS: &str = "OGMA-I18N-005";
    /// Target value is identical to the baseline value (untranslated).
    pub const UNTRANSLATED_VALUE: &str = "OGMA-I18N-006";
    /// Malformed or unparseable plural rule.
    pub const MALFORMED_PLURAL: &str = "OGMA-I18N-007";
    /// Locale-insensitive date/time formatting API used on user-facing data.
    pub const LOCALE_INSENSITIVE_DATE: &str = "OGMA-I18N-008";
    /// Hard-coded number/decimal/grouping formatting.
    pub const LOCALE_INSENSITIVE_NUMBER: &str = "OGMA-I18N-009";
    /// RTL locale supported without direction-aware implementation evidence.
    pub const RTL_BIDI: &str = "OGMA-I18N-010";
    /// Unacceptable fallback to another primary language.
    pub const LANGUAGE_FALLBACK: &str = "OGMA-I18N-011";
    /// Translation coverage below policy threshold.
    pub const COVERAGE_BELOW_THRESHOLD: &str = "OGMA-I18N-012";
    /// Duplicate key inside a catalogue.
    pub const DUPLICATE_KEY: &str = "OGMA-I18N-013";
}

/// Pass/fail/unknown status of one conformance check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CheckStatus {
    Fail,
    Unknown,
    Pass,
}

/// Outcome of one named dimension of native-support analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub detail: String,
}

/// Per-locale conformance report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocaleReport {
    pub locale: LocaleId,
    /// Fraction of the baseline linguistic surface covered [0.0, 1.0].
    pub coverage: f64,
    pub native: bool,
    pub fallback_dependencies: Vec<FallbackRule>,
    pub checks: BTreeMap<String, CheckOutcome>,
    pub violations: Vec<Violation>,
}

/// Repository-wide summary of the linguistic surface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinguisticSurface {
    /// Total distinct user-facing translation keys in the baseline surface.
    pub total_units: usize,
    /// User-facing string occurrences found in source.
    pub user_facing_occurrences: usize,
    /// Hard-coded (non-translated) user-facing occurrences.
    pub hard_coded_occurrences: usize,
    pub by_kind: BTreeMap<String, usize>,
}

/// Overall audit verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OverallStatus {
    Fail,
    Unknown,
    Pass,
}

/// The complete, serialisable result of an Ogma audit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditResult {
    pub application_name: Option<String>,
    pub baseline: BaselineDetection,
    /// Frameworks detected (React, Django, ...), deterministically ordered.
    pub frameworks: Vec<String>,
    pub locales: Vec<LocaleReport>,
    pub linguistic_surface: LinguisticSurface,
    /// Repository-global violations not attributable to a single locale.
    pub violations: Vec<Violation>,
    pub overall: OverallStatus,
}

/// Baseline policy: explicit or automatic detection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaselinePolicy {
    Auto,
    Explicit(LocaleId),
}

/// The evaluated conformance policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub baseline: BaselinePolicy,
    pub required_locales: Vec<LocaleId>,
    /// Required translation coverage in [0.0, 1.0].
    pub translation_coverage: f64,
    pub allow_fallback: bool,
    pub require_pluralisation: bool,
    pub require_locale_formatting: bool,
    pub require_accessibility: bool,
    /// Whether checks with `Confidence::Unknown` may count as passes.
    pub allow_unknown: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            baseline: BaselinePolicy::Auto,
            required_locales: Vec::new(),
            translation_coverage: 1.0,
            allow_fallback: false,
            require_pluralisation: true,
            require_locale_formatting: true,
            require_accessibility: false,
            allow_unknown: false,
        }
    }
}

impl Policy {
    /// Whether a check outcome satisfies this policy.
    pub fn outcome_satisfies(&self, outcome: &CheckOutcome, confidence: Confidence) -> bool {
        match (outcome.status, confidence) {
            (CheckStatus::Fail, _) => false,
            // UNKNOWN must never silently pass: it satisfies the policy only
            // when the policy explicitly permits unknown conditions.
            (CheckStatus::Unknown, _) => self.allow_unknown,
            (CheckStatus::Pass, Confidence::Unknown) => self.allow_unknown,
            (CheckStatus::Pass, _) => true,
        }
    }
}
