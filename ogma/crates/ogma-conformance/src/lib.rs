//! Deterministic conformance evaluation for Ogma.
//!
//! [`evaluate`] produces one [`ogma_model::LocaleReport`] per supported or
//! policy-required locale across eight named check dimensions, and
//! [`detect_baseline`] determines the application's baseline language from
//! weighted, deterministic evidence. No randomness, timestamps or
//! hash-ordered containers are used anywhere: identical inputs always
//! produce identical output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ogma_locale::{fallback_chain, profile_for, TextDirection};
use ogma_model::{
    codes, BaselineDetection, BaselinePolicy, CheckOutcome, CheckStatus, Confidence, Language,
    LocaleId, LocaleReport, Severity, StringResource, Violation,
};
use ogma_strings::{compare_catalogues, coverage, units_by_key};

/// Inputs for one conformance evaluation.
pub struct ConformanceInput<'a> {
    /// The detected or policy-mandated baseline locale.
    pub baseline_locale: ogma_model::LocaleId,
    /// The catalogue chosen as the baseline linguistic surface
    /// (may have zero units when the app has no catalogues).
    pub baseline_catalogue: &'a ogma_model::StringResource,
    /// Every discovered catalogue (including the baseline catalogue).
    pub catalogues: &'a [ogma_model::StringResource],
    /// Translation keys referenced from source code (translated occurrences).
    pub source_keys: &'a std::collections::BTreeSet<String>,
    /// All locales the application declares/supports (catalogue locales,
    /// config hints, policy-required union), including the baseline.
    pub supported_locales: &'a [ogma_model::LocaleId],
    /// Locale-semantics findings from source analysis (OGMA-I18N-008/009/010).
    pub semantic_violations: &'a [ogma_model::Violation],
    /// Number of hard-coded user-facing aria-label strings in source.
    pub hard_coded_aria_labels: usize,
    /// The evaluated policy.
    pub policy: &'a ogma_model::Policy,
}

#[derive(Debug)]
pub struct ConformanceOutput {
    /// Per-locale reports, sorted by locale.
    pub locales: Vec<ogma_model::LocaleReport>,
    /// Repository-global violations not attributable to one locale.
    pub global_violations: Vec<ogma_model::Violation>,
}

const CHECK_TRANSLATION_COVERAGE: &str = "translation_coverage";
const CHECK_STRUCTURAL_INTEGRITY: &str = "structural_integrity";
const CHECK_PLURALISATION: &str = "pluralisation";
const CHECK_FALLBACK_INDEPENDENCE: &str = "fallback_independence";
const CHECK_LOCALE_FORMATTING: &str = "locale_formatting";
const CHECK_TEXT_DIRECTION: &str = "text_direction";
const CHECK_ACCESSIBILITY: &str = "accessibility";
const CHECK_SURFACE_COVERAGE: &str = "surface_coverage";

/// Evaluate conformance for every supported/required locale.
pub fn evaluate(input: &ConformanceInput) -> ConformanceOutput {
    let mut locale_set: BTreeSet<LocaleId> = input.supported_locales.iter().cloned().collect();
    locale_set.extend(input.policy.required_locales.iter().cloned());

    let mut reports = Vec::new();
    for locale in &locale_set {
        let report = if is_required_missing(locale, input) {
            required_missing_report(locale, input)
        } else {
            let target: &StringResource = if locale.canonical == input.baseline_locale.canonical {
                input.baseline_catalogue
            } else {
                match find_catalogue(locale, input.catalogues) {
                    Some(c) => c,
                    None => {
                        reports.push(evaluate_locale(
                            locale,
                            &synthetic_surface(locale),
                            false,
                            input,
                        ));
                        continue;
                    }
                }
            };
            evaluate_locale(
                locale,
                target,
                locale.canonical == input.baseline_locale.canonical,
                input,
            )
        };
        reports.push(report);
    }

    ConformanceOutput {
        locales: reports,
        global_violations: Vec::new(),
    }
}

/// Determine the application baseline language from weighted evidence.
/// Policy baseline Explicit wins outright (confidence Proven).
/// Weights: Strong=100, Medium=50, Weak=10, summed per language.
///  - clear leader (strictly greater than runner-up) with total >= 100 → Proven
///  - clear leader with total < 100 → Likely
///  - tie for the lead → language None, confidence Unknown (ambiguity is
///    reported, never hidden)
pub fn detect_baseline(
    evidence: &[ogma_model::LanguageEvidence],
    policy: &ogma_model::Policy,
) -> ogma_model::BaselineDetection {
    if let BaselinePolicy::Explicit(loc) = &policy.baseline {
        return BaselineDetection {
            language: Some(loc.language.clone()),
            confidence: Confidence::Proven,
            evidence: evidence.to_vec(),
        };
    }

    let mut totals: BTreeMap<&Language, u64> = BTreeMap::new();
    for e in evidence {
        let weight = match e.strength {
            ogma_model::EvidenceStrength::Strong => 100,
            ogma_model::EvidenceStrength::Medium => 50,
            ogma_model::EvidenceStrength::Weak => 10,
        };
        *totals.entry(&e.language).or_insert(0) += weight;
    }

    let mut best: Option<(&Language, u64)> = None;
    let mut tied = false;
    for (language, total) in &totals {
        match best {
            None => {
                best = Some((language, *total));
                tied = false;
            }
            Some((_, best_total)) => {
                if *total > best_total {
                    best = Some((language, *total));
                    tied = false;
                } else if *total == best_total {
                    tied = true;
                }
            }
        }
    }

    let (language, confidence) = match best {
        Some((language, total)) if !tied => (
            Some(language.clone()),
            if total >= 100 {
                Confidence::Proven
            } else {
                Confidence::Likely
            },
        ),
        _ => (None, Confidence::Unknown),
    };

    BaselineDetection {
        language,
        confidence,
        evidence: evidence.to_vec(),
    }
}

fn find_catalogue<'a>(locale: &LocaleId, catalogues: &'a [StringResource]) -> Option<&'a StringResource> {
    catalogues
        .iter()
        .find(|c| {
            c.effective_locale()
                .is_some_and(|l| l.canonical == locale.canonical)
        })
        .or_else(|| {
            catalogues.iter().find(|c| {
                c.effective_locale()
                    .is_some_and(|l| l.language == locale.language)
            })
        })
}

fn is_required_missing(locale: &LocaleId, input: &ConformanceInput) -> bool {
    find_catalogue(locale, input.catalogues).is_none()
        && input
            .policy
            .required_locales
            .iter()
            .any(|r| r.canonical == locale.canonical)
}

fn synthetic_surface(locale: &LocaleId) -> StringResource {
    StringResource {
        path: PathBuf::from(format!("{}.synthetic", locale.canonical)),
        format: ogma_model::CatalogueFormat::Custom("synthetic".to_string()),
        declared_locale: Some(locale.clone()),
        embedded_locale: None,
        units: Vec::new(),
    }
}

fn required_missing_report(locale: &LocaleId, input: &ConformanceInput) -> LocaleReport {
    let mut checks = BTreeMap::new();
    let mut insert = |key: &str, status: CheckStatus, detail: &str| {
        checks.insert(
            key.to_string(),
            CheckOutcome {
                status,
                detail: detail.to_string(),
            },
        );
    };
    insert(CHECK_TRANSLATION_COVERAGE, CheckStatus::Fail, "no catalogue found");
    insert(CHECK_STRUCTURAL_INTEGRITY, CheckStatus::Fail, "locale not implemented");
    for key in [
        CHECK_PLURALISATION,
        CHECK_FALLBACK_INDEPENDENCE,
        CHECK_LOCALE_FORMATTING,
        CHECK_TEXT_DIRECTION,
        CHECK_ACCESSIBILITY,
        CHECK_SURFACE_COVERAGE,
    ] {
        insert(key, CheckStatus::Unknown, "locale not implemented");
    }

    let violation = Violation {
        code: codes::MISSING_TRANSLATION.to_string(),
        severity: Severity::Error,
        message: "locale required by policy but no catalogue or declaration found".to_string(),
        file: None,
        line: None,
        locale: Some(locale.clone()),
        confidence: Confidence::Proven,
        details: Vec::new(),
    };

    LocaleReport {
        locale: locale.clone(),
        coverage: 0.0,
        native: false,
        fallback_dependencies: fallback_chain(
            locale,
            &available_excluding(locale, input),
            Some(&input.baseline_locale),
        ),
        checks,
        violations: vec![violation],
    }
}

fn evaluate_locale(
    locale: &LocaleId,
    target: &StringResource,
    is_baseline: bool,
    input: &ConformanceInput,
) -> LocaleReport {
    let policy = input.policy;
    let mut checks: BTreeMap<String, CheckOutcome> = BTreeMap::new();
    let mut violations: Vec<Violation> = Vec::new();

    // translation_coverage
    let f = if is_baseline {
        1.0
    } else {
        coverage(input.baseline_catalogue, target)
    };
    let coverage_detail = format!(
        "coverage {:.1}% (required {:.1}%)",
        f * 100.0,
        policy.translation_coverage * 100.0
    );
    if f >= policy.translation_coverage {
        checks.insert(
            CHECK_TRANSLATION_COVERAGE.to_string(),
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: coverage_detail,
            },
        );
    } else {
        checks.insert(
            CHECK_TRANSLATION_COVERAGE.to_string(),
            CheckOutcome {
                status: CheckStatus::Fail,
                detail: coverage_detail.clone(),
            },
        );
        violations.push(Violation {
            code: codes::COVERAGE_BELOW_THRESHOLD.to_string(),
            severity: Severity::Error,
            message: format!("coverage {:.3} below required {:.1}", f, policy.translation_coverage),
            file: Some(target.path.clone()),
            line: None,
            locale: Some(locale.clone()),
            confidence: Confidence::Proven,
            details: vec![coverage_detail],
        });
    }

    // structural_integrity and pluralisation share one comparison run.
    // The baseline catalogue compared against itself must not report its own
    // values as untranslated (OGMA-I18N-006) — identity is not a translation
    // defect; duplicate/plural/placeholder findings still apply.
    let required_plurals = profile_for(&locale.language).required_plural_categories;
    let is_self_compare = target.path == input.baseline_catalogue.path
        && target.effective_locale().map(|l| l.canonical.clone())
            == input.baseline_catalogue.effective_locale().map(|l| l.canonical.clone());
    let comparison_all = compare_catalogues(
        input.baseline_catalogue,
        target,
        &required_plurals,
        input.source_keys,
    );
    let comparison: Vec<_> = comparison_all
        .iter()
        .filter(|v| !(is_self_compare && v.code == codes::UNTRANSLATED_VALUE))
        .cloned()
        .collect();
    violations.extend(comparison.iter().cloned());

    let error_count = comparison
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    let structural_detail = if comparison.is_empty() {
        "no structural issues found".to_string()
    } else {
        summarize_by_code(&comparison)
    };
    checks.insert(
        CHECK_STRUCTURAL_INTEGRITY.to_string(),
        CheckOutcome {
            status: if error_count > 0 {
                CheckStatus::Fail
            } else {
                CheckStatus::Pass
            },
            detail: structural_detail,
        },
    );

    // pluralisation
    let plural_outcome = if !policy.require_pluralisation {
        CheckOutcome {
            status: CheckStatus::Pass,
            detail: "not required by policy".to_string(),
        }
    } else {
        let plural_violations: Vec<&Violation> = comparison
            .iter()
            .filter(|v| {
                v.code == codes::MISSING_PLURAL_FORM || v.code == codes::MALFORMED_PLURAL
            })
            .collect();
        if !plural_violations.is_empty() {
            CheckOutcome {
                status: CheckStatus::Fail,
                detail: summarize_by_code(plural_violations),
            }
        } else {
            let plural_groups = units_by_key(target)
                .values()
                .filter(|units| {
                    units.len() > 1 || units.iter().any(|u| u.plural_form.is_some())
                })
                .count();
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: if plural_groups == 0 {
                    "no plural groups".to_string()
                } else {
                    "all required plural categories present".to_string()
                },
            }
        }
    };
    checks.insert(CHECK_PLURALISATION.to_string(), plural_outcome);

    // fallback_independence
    let (fallback_outcome, dependencies) = if is_baseline {
        (
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: "baseline language".to_string(),
            },
            Vec::new(),
        )
    } else {
        let rules = fallback_chain(
            locale,
            &available_excluding(locale, input),
            Some(&input.baseline_locale),
        );
        let crossed: Vec<&ogma_model::FallbackRule> =
            rules.iter().filter(|r| r.crosses_language()).collect();
        let outcome = if crossed.is_empty() {
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: if rules.is_empty() {
                    "no fallback required".to_string()
                } else {
                    format!(
                        "fallback within language: {}",
                        rules
                            .iter()
                            .map(|r| format!("{}→{}", r.requested.canonical, r.target.canonical))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            }
        } else if policy.allow_fallback {
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: format!(
                    "language fallback permitted: {}",
                    crossed
                        .iter()
                        .map(|r| format!("{}→{}", r.requested.canonical, r.target.canonical))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        } else {
            for rule in &crossed {
                violations.push(Violation {
                    code: codes::LANGUAGE_FALLBACK.to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "{} falls back to {} ({})",
                        rule.requested.canonical,
                        rule.target.canonical,
                        fallback_kind_label(rule.kind)
                    ),
                    file: None,
                    line: None,
                    locale: Some(locale.clone()),
                    confidence: Confidence::Proven,
                    details: Vec::new(),
                });
            }
            CheckOutcome {
                status: CheckStatus::Fail,
                detail: format!(
                    "language fallback: {}",
                    crossed
                        .iter()
                        .map(|r| format!("{}→{}", r.requested.canonical, r.target.canonical))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        };
        (outcome, rules)
    };
    checks.insert(CHECK_FALLBACK_INDEPENDENCE.to_string(), fallback_outcome);

    // locale_formatting
    let insensitive_sites = input
        .semantic_violations
        .iter()
        .filter(|v| {
            v.code == codes::LOCALE_INSENSITIVE_DATE || v.code == codes::LOCALE_INSENSITIVE_NUMBER
        })
        .count();
    checks.insert(
        CHECK_LOCALE_FORMATTING.to_string(),
        if insensitive_sites > 0 {
            CheckOutcome {
                status: CheckStatus::Fail,
                detail: format!("{insensitive_sites} locale-insensitive formatting sites"),
            }
        } else {
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: "no locale-insensitive formatting sites found".to_string(),
            }
        },
    );

    // text_direction
    let direction = profile_for(&locale.language).direction;
    checks.insert(
        CHECK_TEXT_DIRECTION.to_string(),
        match direction {
            TextDirection::Ltr => CheckOutcome {
                status: CheckStatus::Pass,
                detail: "LTR language".to_string(),
            },
            TextDirection::Rtl => {
                let bidi_missing = input
                    .semantic_violations
                    .iter()
                    .any(|v| v.code == codes::RTL_BIDI);
                if bidi_missing {
                    CheckOutcome {
                        status: CheckStatus::Unknown,
                        detail: "no direction-aware implementation evidence found".to_string(),
                    }
                } else {
                    CheckOutcome {
                        status: CheckStatus::Pass,
                        detail: "direction handling evidence found".to_string(),
                    }
                }
            }
        },
    );

    // accessibility
    checks.insert(
        CHECK_ACCESSIBILITY.to_string(),
        if policy.require_accessibility {
            if input.hard_coded_aria_labels > 0 {
                CheckOutcome {
                    status: CheckStatus::Fail,
                    detail: format!(
                        "{} hard-coded aria-label strings bypass i18n",
                        input.hard_coded_aria_labels
                    ),
                }
            } else {
                CheckOutcome {
                    status: CheckStatus::Pass,
                    detail: "no hard-coded aria-label strings".to_string(),
                }
            }
        } else {
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: "not required by policy".to_string(),
            }
        },
    );

    // surface_coverage
    checks.insert(
        CHECK_SURFACE_COVERAGE.to_string(),
        CheckOutcome {
            status: CheckStatus::Pass,
            detail: format!(
                "linguistic surface: {} baseline units",
                units_by_key(input.baseline_catalogue).len()
            ),
        },
    );

    sort_violations(&mut violations);

    let native = checks
        .values()
        .all(|c| policy.outcome_satisfies(c, Confidence::Proven));

    LocaleReport {
        locale: locale.clone(),
        coverage: f,
        native,
        fallback_dependencies: dependencies,
        checks,
        violations,
    }
}

fn available_excluding(locale: &LocaleId, input: &ConformanceInput) -> Vec<LocaleId> {
    let mut set: BTreeSet<LocaleId> = input.supported_locales.iter().cloned().collect();
    set.extend(input.policy.required_locales.iter().cloned());
    set.remove(locale);
    set.into_iter().collect()
}

fn summarize_by_code<'a>(violations: impl IntoIterator<Item = &'a Violation>) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for v in violations {
        *counts.entry(v.code.as_str()).or_insert(0) += 1;
    }
    counts
        .iter()
        .map(|(code, n)| format!("{n} {}", code_label(code)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn code_label(code: &str) -> &'static str {
    match code {
        codes::MISSING_TRANSLATION => "missing translation",
        codes::ORPHANED_KEY => "orphaned key",
        codes::MISSING_PLURAL_FORM => "missing plural form",
        codes::MALFORMED_PLURAL => "malformed plural",
        codes::INCONSISTENT_PLACEHOLDERS => "inconsistent placeholders",
        codes::UNTRANSLATED_VALUE => "untranslated value",
        codes::DUPLICATE_KEY => "duplicate key",
        _ => "issue",
    }
}

fn fallback_kind_label(kind: ogma_model::FallbackKind) -> &'static str {
    match kind {
        ogma_model::FallbackKind::Regional => "regional fallback",
        ogma_model::FallbackKind::LinguisticChain => "linguistic chain",
        ogma_model::FallbackKind::MissingTranslation => "missing translation",
        ogma_model::FallbackKind::PrimaryLanguage => "primary language",
    }
}

fn sort_violations(violations: &mut [Violation]) {
    violations.sort_by(|a, b| {
        (&a.file, a.line.unwrap_or(0), &a.code, &a.message).cmp(&(
            &b.file,
            b.line.unwrap_or(0),
            &b.code,
            &b.message,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogma_model::{
        CatalogueFormat, EvidenceKind, EvidenceStrength, LanguageEvidence, Policy, TranslationUnit,
    };

    fn loc(s: &str) -> LocaleId {
        ogma_locale::parse_locale(s).unwrap()
    }

    fn unit(key: &str, value: Option<&str>, plural: Option<&str>) -> TranslationUnit {
        TranslationUnit {
            key: key.to_string(),
            value: value.map(|v| v.to_string()),
            locale: None,
            plural_form: plural.map(|p| p.to_string()),
            placeholders: ogma_strings::extract_placeholders(value.unwrap_or("")),
        }
    }

    fn res(path: &str, locale: Option<&str>, units: Vec<TranslationUnit>) -> StringResource {
        StringResource {
            path: PathBuf::from(path),
            format: CatalogueFormat::Json,
            declared_locale: locale.map(|l| loc(l)),
            embedded_locale: None,
            units,
        }
    }

    fn en_baseline() -> StringResource {
        res(
            "en.json",
            Some("en"),
            vec![
                unit("a", Some("Hello"), None),
                unit("b", Some("Hi {name}"), None),
            ],
        )
    }

    fn perfect_fr() -> StringResource {
        res(
            "fr.json",
            Some("fr"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        )
    }

    fn evidence(language: &str, strength: EvidenceStrength) -> LanguageEvidence {
        LanguageEvidence {
            source: "test".to_string(),
            kind: EvidenceKind::SourceStrings,
            strength,
            language: Language::new(language),
            detail: String::new(),
        }
    }

    fn semantic(code: &str) -> Violation {
        Violation {
            code: code.to_string(),
            severity: Severity::Warning,
            message: String::new(),
            file: Some(PathBuf::from("src/main.rs")),
            line: Some(1),
            locale: None,
            confidence: Confidence::Proven,
            details: Vec::new(),
        }
    }

    #[test]
    fn detect_baseline_empty_evidence_is_unknown() {
        let policy = Policy::default();
        let d = detect_baseline(&[], &policy);
        assert_eq!(d.language, None);
        assert_eq!(d.confidence, Confidence::Unknown);
        assert!(d.evidence.is_empty());
    }

    #[test]
    fn detect_baseline_single_strong_is_proven() {
        let policy = Policy::default();
        let d = detect_baseline(&[evidence("en", EvidenceStrength::Strong)], &policy);
        assert_eq!(d.language, Some(Language::new("en")));
        assert_eq!(d.confidence, Confidence::Proven);
    }

    #[test]
    fn detect_baseline_tie_is_ambiguous() {
        let policy = Policy::default();
        let d = detect_baseline(
            &[
                evidence("en", EvidenceStrength::Strong),
                evidence("fr", EvidenceStrength::Strong),
            ],
            &policy,
        );
        assert_eq!(d.language, None);
        assert_eq!(d.confidence, Confidence::Unknown);
    }

    #[test]
    fn detect_baseline_explicit_policy_wins() {
        let policy = Policy {
            baseline: BaselinePolicy::Explicit(loc("de")),
            ..Policy::default()
        };
        let d = detect_baseline(&[evidence("en", EvidenceStrength::Strong)], &policy);
        assert_eq!(d.language, Some(Language::new("de")));
        assert_eq!(d.confidence, Confidence::Proven);
        assert_eq!(d.evidence.len(), 1);
    }

    #[test]
    fn detect_baseline_weak_only_is_likely() {
        let policy = Policy::default();
        let d = detect_baseline(
            &[
                evidence("en", EvidenceStrength::Weak),
                evidence("en", EvidenceStrength::Weak),
            ],
            &policy,
        );
        assert_eq!(d.language, Some(Language::new("en")));
        assert_eq!(d.confidence, Confidence::Likely);
    }

    #[test]
    fn detect_baseline_clear_leader_over_runner_up() {
        let policy = Policy::default();
        let d = detect_baseline(
            &[
                evidence("en", EvidenceStrength::Strong),
                evidence("fr", EvidenceStrength::Weak),
            ],
            &policy,
        );
        assert_eq!(d.language, Some(Language::new("en")));
        assert_eq!(d.confidence, Confidence::Proven);
    }

    #[test]
    fn evaluate_perfect_app_is_native() {
        // A regional variant of the baseline language falls back within
        // language, so the default (no cross-language fallback) policy passes.
        let baseline = en_baseline();
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en"), loc("en-GB")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        assert_eq!(out.locales.len(), 2);
        assert_eq!(out.locales[0].locale.canonical, "en");
        assert_eq!(out.locales[1].locale.canonical, "en-GB");
        assert!(out.locales[0].native);
        assert!(out.locales[1].native);
        assert_eq!(out.locales[0].coverage, 1.0);
        assert_eq!(out.locales[1].coverage, 1.0);
        assert_eq!(
            out.locales[1].checks["fallback_independence"].detail,
            "fallback within language: en-GB→en"
        );
        assert_eq!(out.global_violations.len(), 0);
        assert_eq!(
            out.locales[0].checks.keys().cloned().collect::<Vec<_>>(),
            vec![
                "accessibility",
                "fallback_independence",
                "locale_formatting",
                "pluralisation",
                "structural_integrity",
                "surface_coverage",
                "text_direction",
                "translation_coverage",
            ]
        );
        // Baseline self-comparison must not report its own values as
        // untranslated (identity is not a translation defect): with an empty
        // baseline catalogue there are no violations at all.
        assert!(out.locales[0].violations.is_empty());
    }

    #[test]
    fn evaluate_missing_translations_fails_locale() {
        let baseline = en_baseline();
        let fr = res("fr.json", Some("fr"), vec![unit("a", Some("Bonjour"), None)]);
        let catalogues = [baseline.clone(), fr.clone()];
        let supported = vec![loc("en"), loc("fr")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let fr_report = &out.locales[1];
        assert!(!fr_report.native);
        assert_eq!(fr_report.coverage, 0.5);
        assert_eq!(
            fr_report.checks["translation_coverage"].status,
            CheckStatus::Fail
        );
        assert_eq!(
            fr_report.checks["structural_integrity"].status,
            CheckStatus::Fail
        );
        assert!(fr_report
            .violations
            .iter()
            .any(|v| v.code == codes::COVERAGE_BELOW_THRESHOLD));
        assert!(fr_report
            .violations
            .iter()
            .any(|v| v.code == codes::MISSING_TRANSLATION));
        assert_eq!(
            fr_report.checks["translation_coverage"].detail,
            "coverage 50.0% (required 100.0%)"
        );
    }

    #[test]
    fn evaluate_fallback_crossing_disallowed_fails() {
        let baseline = en_baseline();
        let fr_ca = res(
            "fr-CA.json",
            Some("fr-CA"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        );
        let catalogues = [baseline.clone(), fr_ca.clone()];
        let supported = vec![loc("en"), loc("fr-CA")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[1];
        assert_eq!(report.locale.canonical, "fr-CA");
        assert_eq!(
            report.checks["fallback_independence"].status,
            CheckStatus::Fail
        );
        assert!(!report.native);
        let fb = report
            .violations
            .iter()
            .filter(|v| v.code == codes::LANGUAGE_FALLBACK)
            .collect::<Vec<_>>();
        assert_eq!(fb.len(), 1);
        assert_eq!(fb[0].severity, Severity::Error);
        assert_eq!(fb[0].message, "fr-CA falls back to en (primary language)");
        assert_eq!(report.fallback_dependencies.len(), 1);
    }

    #[test]
    fn evaluate_fallback_crossing_allowed_passes() {
        let baseline = en_baseline();
        let fr_ca = res(
            "fr-CA.json",
            Some("fr-CA"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        );
        let catalogues = [baseline.clone(), fr_ca.clone()];
        let supported = vec![loc("en"), loc("fr-CA")];
        let source_keys = BTreeSet::new();
        let policy = Policy {
            allow_fallback: true,
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[1];
        assert_eq!(
            report.checks["fallback_independence"].status,
            CheckStatus::Pass
        );
        assert!(report.native);
        assert!(report
            .checks["fallback_independence"]
            .detail
            .contains("fr-CA→en"));
    }

    #[test]
    fn evaluate_fallback_within_language_passes() {
        let baseline = res(
            "fr.json",
            Some("fr"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        );
        let fr_ca = res(
            "fr-CA.json",
            Some("fr-CA"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        );
        let catalogues = [baseline.clone(), fr_ca.clone()];
        let supported = vec![loc("fr"), loc("fr-CA")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("fr"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "fr-CA")
            .unwrap();
        assert_eq!(
            report.checks["fallback_independence"].status,
            CheckStatus::Pass
        );
        assert_eq!(
            report.checks["fallback_independence"].detail,
            "fallback within language: fr-CA→fr"
        );
        assert!(report.native);
    }

    #[test]
    fn evaluate_rtl_without_bidi_evidence_is_unknown() {
        let baseline = res("en.json", Some("en"), vec![]);
        let ar = res("ar.json", Some("ar"), vec![]);
        let catalogues = [baseline.clone(), ar.clone()];
        let supported = vec![loc("en"), loc("ar")];
        let source_keys = BTreeSet::new();
        let semantic = [semantic(codes::RTL_BIDI)];
        let policy = Policy {
            allow_fallback: true,
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &semantic,
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "ar")
            .unwrap();
        assert_eq!(
            report.checks["text_direction"].status,
            CheckStatus::Unknown
        );
        assert_eq!(
            report.checks["text_direction"].detail,
            "no direction-aware implementation evidence found"
        );
        // Native determination delegates to Policy::outcome_satisfies; the
        // conformance engine emits no RTL_BIDI violation of its own.
        assert_eq!(
            report.native,
            policy.outcome_satisfies(
                &report.checks["text_direction"],
                Confidence::Proven
            )
        );
        assert!(report
            .violations
            .iter()
            .all(|v| v.code != codes::RTL_BIDI));
    }

    #[test]
    fn evaluate_unknown_check_passes_native_only_when_policy_allows_unknown() {
        let baseline = res("en.json", Some("en"), vec![]);
        let ar = res("ar.json", Some("ar"), vec![]);
        let catalogues = [baseline.clone(), ar.clone()];
        let supported = vec![loc("en"), loc("ar")];
        let source_keys = BTreeSet::new();
        let semantic = [semantic(codes::RTL_BIDI)];
        fn make_input<'a>(
            policy: &'a Policy,
            baseline: &'a ogma_model::StringResource,
            catalogues: &'a [ogma_model::StringResource],
            supported: &'a [ogma_model::LocaleId],
            source_keys: &'a std::collections::BTreeSet<String>,
            semantic: &'a [ogma_model::Violation],
        ) -> ConformanceInput<'a> {
            ConformanceInput {
                baseline_locale: loc("en"),
                baseline_catalogue: baseline,
                catalogues,
                source_keys,
                supported_locales: supported,
                semantic_violations: semantic,
                hard_coded_aria_labels: 0,
                policy,
            }
        }
        let strict = Policy {
            allow_fallback: true,
            ..Policy::default()
        };
        let permissive = Policy {
            allow_fallback: true,
            allow_unknown: true,
            ..Policy::default()
        };
        let strict_out = evaluate(&make_input(&strict, &baseline, &catalogues, &supported, &source_keys, &semantic));
        let permissive_out = evaluate(&make_input(&permissive, &baseline, &catalogues, &supported, &source_keys, &semantic));
        let strict_ar = strict_out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "ar")
            .unwrap();
        let permissive_ar = permissive_out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "ar")
            .unwrap();
        assert_eq!(
            strict_ar.checks["text_direction"].status,
            CheckStatus::Unknown
        );
        assert!(!strict_ar.native);
        assert!(permissive_ar.native);
    }

    #[test]
    fn evaluate_rtl_with_bidi_evidence_passes() {
        let baseline = res("en.json", Some("en"), vec![]);
        let ar = res("ar.json", Some("ar"), vec![]);
        let catalogues = [baseline.clone(), ar.clone()];
        let supported = vec![loc("en"), loc("ar")];
        let source_keys = BTreeSet::new();
        let policy = Policy {
            allow_fallback: true,
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "ar")
            .unwrap();
        assert_eq!(report.checks["text_direction"].status, CheckStatus::Pass);
        assert_eq!(
            report.checks["text_direction"].detail,
            "direction handling evidence found"
        );
        assert!(report.native);
    }

    #[test]
    fn evaluate_required_missing_locale_fails() {
        let baseline = en_baseline();
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en")];
        let required = vec![loc("de")];
        let source_keys = BTreeSet::new();
        let policy = Policy {
            required_locales: required.clone(),
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        assert_eq!(out.locales.len(), 2);
        let report = out
            .locales
            .iter()
            .find(|r| r.locale.canonical == "de")
            .unwrap();
        assert_eq!(report.coverage, 0.0);
        assert!(!report.native);
        assert_eq!(
            report.checks["translation_coverage"].status,
            CheckStatus::Fail
        );
        assert_eq!(
            report.checks["translation_coverage"].detail,
            "no catalogue found"
        );
        assert_eq!(
            report.checks["structural_integrity"].status,
            CheckStatus::Fail
        );
        for key in [
            "pluralisation",
            "fallback_independence",
            "locale_formatting",
            "text_direction",
            "accessibility",
            "surface_coverage",
        ] {
            assert_eq!(report.checks[key].status, CheckStatus::Unknown);
            assert_eq!(report.checks[key].detail, "locale not implemented");
        }
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].code, codes::MISSING_TRANSLATION);
        assert_eq!(
            report.violations[0].message,
            "locale required by policy but no catalogue or declaration found"
        );
        assert_eq!(report.violations[0].severity, Severity::Error);
    }

    #[test]
    fn evaluate_duplicate_keys_in_baseline_reported() {
        let baseline = res(
            "en.json",
            Some("en"),
            vec![
                unit("a", Some("Hello"), None),
                unit("a", Some("Hello again"), None),
            ],
        );
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[0];
        assert!(report
            .violations
            .iter()
            .any(|v| v.code == codes::DUPLICATE_KEY));
    }

    #[test]
    fn evaluate_aria_labels_fail_when_required() {
        let baseline = en_baseline();
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en")];
        let source_keys = BTreeSet::new();
        let policy = Policy {
            require_accessibility: true,
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 2,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[0];
        assert_eq!(report.checks["accessibility"].status, CheckStatus::Fail);
        assert_eq!(
            report.checks["accessibility"].detail,
            "2 hard-coded aria-label strings bypass i18n"
        );
        assert!(!report.native);
    }

    #[test]
    fn evaluate_aria_labels_not_required_passes() {
        let baseline = en_baseline();
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en")];
        let source_keys = BTreeSet::new();
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 5,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[0];
        assert_eq!(report.checks["accessibility"].status, CheckStatus::Pass);
        assert_eq!(
            report.checks["accessibility"].detail,
            "not required by policy"
        );
        assert!(report.native);
    }

    #[test]
    fn evaluate_coverage_boundary_equal_passes() {
        let baseline = en_baseline();
        let fr = res("fr.json", Some("fr"), vec![unit("a", Some("Bonjour"), None)]);
        let catalogues = [baseline.clone(), fr.clone()];
        let supported = vec![loc("en"), loc("fr")];
        let source_keys = BTreeSet::new();
        let policy = Policy {
            translation_coverage: 0.5,
            ..Policy::default()
        };
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &[],
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[1];
        assert_eq!(report.coverage, 0.5);
        assert_eq!(
            report.checks["translation_coverage"].status,
            CheckStatus::Pass
        );
    }

    #[test]
    fn evaluate_locale_formatting_counts_semantic_violations() {
        let baseline = en_baseline();
        let catalogues = [baseline.clone()];
        let supported = vec![loc("en")];
        let source_keys = BTreeSet::new();
        let semantic = [
            semantic(codes::LOCALE_INSENSITIVE_DATE),
            semantic(codes::LOCALE_INSENSITIVE_DATE),
            semantic(codes::LOCALE_INSENSITIVE_NUMBER),
        ];
        let policy = Policy::default();
        let input = ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &semantic,
            hard_coded_aria_labels: 0,
            policy: &policy,
        };
        let out = evaluate(&input);
        let report = &out.locales[0];
        assert_eq!(
            report.checks["locale_formatting"].status,
            CheckStatus::Fail
        );
        assert_eq!(
            report.checks["locale_formatting"].detail,
            "3 locale-insensitive formatting sites"
        );
        assert!(!report.native);
    }

    #[test]
    fn evaluate_is_deterministic() {
        let baseline = en_baseline();
        let fr = perfect_fr();
        let fr_ca = res(
            "fr-CA.json",
            Some("fr-CA"),
            vec![
                unit("a", Some("Bonjour"), None),
                unit("b", Some("Salut {name}"), None),
            ],
        );
        let catalogues = [baseline.clone(), fr.clone(), fr_ca.clone()];
        let supported = vec![loc("en"), loc("fr"), loc("fr-CA")];
        let source_keys: BTreeSet<String> = ["a".to_string()].into_iter().collect();
        let semantic = [
            semantic(codes::LOCALE_INSENSITIVE_DATE),
            semantic(codes::RTL_BIDI),
        ];
        let policy = Policy {
            required_locales: vec![loc("de")],
            ..Policy::default()
        };
        let make_input = || ConformanceInput {
            baseline_locale: loc("en"),
            baseline_catalogue: &baseline,
            catalogues: &catalogues,
            source_keys: &source_keys,
            supported_locales: &supported,
            semantic_violations: &semantic,
            hard_coded_aria_labels: 1,
            policy: &policy,
        };
        let first = evaluate(&make_input());
        let second = evaluate(&make_input());
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
        // Locale reports sorted by locale: de, en, fr, fr-CA.
        let names: Vec<&str> = first
            .locales
            .iter()
            .map(|r| r.locale.canonical.as_str())
            .collect();
        assert_eq!(names, vec!["de", "en", "fr", "fr-CA"]);
        for report in &first.locales {
            for w in report.violations.windows(2) {
                let a = (&w[0].file, w[0].line.unwrap_or(0), &w[0].code, &w[0].message);
                let b = (&w[1].file, w[1].line.unwrap_or(0), &w[1].code, &w[1].message);
                assert!(a <= b);
            }
        }
    }
}
