//! Structural comparison of catalogues: coverage, missing/orphaned/
//! duplicate/untranslated keys, placeholder integrity and plural forms.

use std::collections::{BTreeMap, BTreeSet};

use ogma_model::{codes, CatalogueFormat, Confidence, Severity, StringResource, TranslationUnit, Violation};

use crate::discovery::is_named_plural;

/// Group a resource's units by translation key; plural arms share the key.
/// Keys are sorted; deterministic.
pub fn units_by_key(res: &StringResource) -> BTreeMap<String, Vec<TranslationUnit>> {
    let mut map: BTreeMap<String, Vec<TranslationUnit>> = BTreeMap::new();
    for unit in &res.units {
        map.entry(unit.key.clone()).or_default().push(unit.clone());
    }
    map
}

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

fn has_non_empty_unit(units: &[TranslationUnit]) -> bool {
    units.iter().any(|u| non_empty(u.value.as_deref()))
}

fn is_po_arm(form: &Option<String>) -> bool {
    form.as_deref().is_some_and(|f| f.starts_with("arm") && f[3..].chars().all(|c| c.is_ascii_digit()))
}

fn is_po_raw_arm(form: &Option<String>) -> bool {
    form.as_deref()
        .is_some_and(|f| f.starts_with("rawarm") && f[6..].chars().all(|c| c.is_ascii_digit()))
}

fn named_plural_count(units: &[TranslationUnit]) -> usize {
    units.iter().filter(|u| is_named_plural(&u.plural_form)).count()
}

fn has_alphabetic(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic())
}

/// Structural comparison of a target catalogue against the baseline catalogue.
///
/// `required_plurals` are the CLDR plural categories the target locale
/// requires (from `ogma_locale::profile_for`); pass an empty slice to disable
/// that check. `source_keys` are keys referenced from source code; a non-empty
/// set disables orphan analysis for keys it contains.
///
/// Returns violations sorted by (file, line, code); deterministic. All
/// violations carry `Confidence::Proven` (structural facts), the target's
/// effective locale, and the target's repository-relative path.
pub fn compare_catalogues(
    baseline: &StringResource,
    target: &StringResource,
    required_plurals: &[String],
    source_keys: &BTreeSet<String>,
) -> Vec<Violation> {
    let base_groups = units_by_key(baseline);
    let target_groups = units_by_key(target);
    let locale = target.effective_locale().cloned();
    let file = Some(target.path.clone());
    let mut violations = Vec::new();

    let push = |code: &'static str,
                    severity: Severity,
                    message: String,
                    details: Vec<String>,
                    violations: &mut Vec<Violation>| {
        violations.push(Violation {
            code: code.to_string(),
            severity,
            message,
            file: file.clone(),
            line: None,
            locale: locale.clone(),
            confidence: Confidence::Proven,
            details,
        });
    };

    // Baseline-driven checks, in sorted key order.
    for (key, base_units) in &base_groups {
        if !has_non_empty_unit(base_units) {
            continue; // empty baseline value: skip entirely
        }
        let Some(target_units) = target_groups.get(key) else {
            push(
                codes::MISSING_TRANSLATION,
                Severity::Error,
                format!("missing translation for key '{key}'"),
                vec![format!("key '{key}' missing in {}", target.path.display())],
                &mut violations,
            );
            continue;
        };

        if !has_non_empty_unit(target_units) {
            push(
                codes::MISSING_TRANSLATION,
                Severity::Error,
                format!("key '{key}' has no translated value"),
                vec![format!(
                    "key '{key}' is present in {} but all values are empty",
                    target.path.display()
                )],
                &mut violations,
            );
            continue;
        }

        // Per-arm untranslated + placeholder checks.
        for tu in target_units {
            let Some(value) = tu.value.as_deref().filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let Some(counterpart) = base_units
                .iter()
                .find(|b| b.plural_form == tu.plural_form && non_empty(b.value.as_deref()))
            else {
                continue;
            };
            let base_value = counterpart.value.as_deref().unwrap_or("");

            if value.eq_ignore_ascii_case(base_value) && has_alphabetic(value) {
                let message = match &tu.plural_form {
                    Some(form) => format!("key '{key}' plural form '{form}' is untranslated"),
                    None => format!("key '{key}' is untranslated"),
                };
                push(
                    codes::UNTRANSLATED_VALUE,
                    Severity::Warning,
                    message,
                    vec![format!(
                        "value of key '{key}' in {} equals the baseline value",
                        target.path.display()
                    )],
                    &mut violations,
                );
            }

            if tu.placeholders != counterpart.placeholders {
                let base_set: BTreeSet<&str> =
                    counterpart.placeholders.iter().map(String::as_str).collect();
                let target_set: BTreeSet<&str> =
                    tu.placeholders.iter().map(String::as_str).collect();
                let only_base: Vec<&str> = base_set.difference(&target_set).copied().collect();
                let only_target: Vec<&str> = target_set.difference(&base_set).copied().collect();
                push(
                    codes::INCONSISTENT_PLACEHOLDERS,
                    Severity::Error,
                    format!("placeholder mismatch for key '{key}'"),
                    vec![format!(
                        "key '{key}': only in baseline: [{}]; only in target: [{}]",
                        only_base.join(", "),
                        only_target.join(", ")
                    )],
                    &mut violations,
                );
            }
        }

        // Plural-form checks.
        let base_is_plural =
            base_units.len() > 1 || base_units.iter().any(|u| u.plural_form.is_some());
        if base_is_plural {
            let target_has_raw_arm = target_units.iter().any(|u| is_po_raw_arm(&u.plural_form));
            let target_has_arm = target_units.iter().any(|u| is_po_arm(&u.plural_form));
            if target.format == CatalogueFormat::Po && target_has_raw_arm {
                push(
                    codes::MALFORMED_PLURAL,
                    Severity::Error,
                    format!("malformed plural declaration for key '{key}'"),
                    vec![format!(
                        "key '{key}' in {} has msgid_plural but the file has no Plural-Forms header",
                        target.path.display()
                    )],
                    &mut violations,
                );
            } else if target.format == CatalogueFormat::Po && target_has_arm {
                let nplurals = target_units.iter().filter(|u| is_po_arm(&u.plural_form)).count();
                let translated =
                    target_units.iter().filter(|u| is_po_arm(&u.plural_form) && non_empty(u.value.as_deref())).count();
                if translated < nplurals {
                    push(
                        codes::MISSING_PLURAL_FORM,
                        Severity::Error,
                        format!("missing plural form for key '{key}'"),
                        vec![format!(
                            "key '{key}' in {} has {translated} of {nplurals} plural arms translated",
                            target.path.display()
                        )],
                        &mut violations,
                    );
                }
            } else {
                let target_cats: BTreeSet<&str> = target_units
                    .iter()
                    .filter_map(|u| u.plural_form.as_deref())
                    .filter(|f| is_named_plural(&Some((*f).to_string())))
                    .collect();
                let missing: Vec<&String> = required_plurals
                    .iter()
                    .filter(|c| !target_cats.contains(c.as_str()))
                    .collect();
                if !missing.is_empty() {
                    push(
                        codes::MISSING_PLURAL_FORM,
                        Severity::Error,
                        format!("missing plural form for key '{key}'"),
                        vec![format!(
                            "key '{key}' in {} is missing required plural categories: {}",
                            target.path.display(),
                            missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        )],
                        &mut violations,
                    );
                } else if named_plural_count(target_units) < named_plural_count(base_units) {
                    push(
                        codes::MISSING_PLURAL_FORM,
                        Severity::Error,
                        format!("missing plural form for key '{key}'"),
                        vec![format!(
                            "key '{key}' in {} has fewer plural arms than the baseline",
                            target.path.display()
                        )],
                        &mut violations,
                    );
                }
            }
        }
    }

    // Target-driven checks: duplicates and orphans, in sorted key order.
    for (key, target_units) in &target_groups {
        let non_plural = target_units.iter().filter(|u| u.plural_form.is_none()).count();
        if non_plural > 1 {
            push(
                codes::DUPLICATE_KEY,
                Severity::Warning,
                format!("duplicate key '{key}'"),
                vec![format!(
                    "key '{key}' appears {non_plural} times in {}",
                    target.path.display()
                )],
                &mut violations,
            );
        }
        if !base_groups.contains_key(key) {
            let excused = !source_keys.is_empty() && source_keys.contains(key);
            if !excused {
                push(
                    codes::ORPHANED_KEY,
                    Severity::Warning,
                    format!("orphaned key '{key}'"),
                    vec![format!(
                        "key '{key}' in {} has no counterpart in the baseline catalogue {}",
                        target.path.display(),
                        baseline.path.display()
                    )],
                    &mut violations,
                );
            }
        }
    }

    violations.sort_by(|a, b| {
        (&a.file, a.line.unwrap_or(0), &a.code, &a.message).cmp(&(
            &b.file,
            b.line.unwrap_or(0),
            &b.code,
            &b.message,
        ))
    });
    violations
}

/// Fraction of baseline keys present with a non-empty value in target
/// [0.0, 1.0]. Plural groups count once; baseline keys whose values are all
/// empty are excluded from the denominator. An empty baseline yields 1.0.
pub fn coverage(baseline: &StringResource, target: &StringResource) -> f64 {
    let base_groups = units_by_key(baseline);
    let target_groups = units_by_key(target);
    let mut total = 0usize;
    let mut covered = 0usize;
    for (key, base_units) in &base_groups {
        if !has_non_empty_unit(base_units) {
            continue;
        }
        total += 1;
        if target_groups
            .get(key)
            .is_some_and(|units| has_non_empty_unit(units))
        {
            covered += 1;
        }
    }
    if total == 0 {
        1.0
    } else {
        covered as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogma_model::{CatalogueFormat, LocaleId};

    fn loc(s: &str) -> LocaleId {
        ogma_locale::parse_locale(s).unwrap()
    }

    fn resource(path: &str, units: Vec<(&str, Option<&str>, Option<&str>)>) -> StringResource {
        let mut units: Vec<TranslationUnit> = units
            .into_iter()
            .map(|(key, value, plural)| TranslationUnit {
                key: key.to_string(),
                value: value.map(|v| v.to_string()),
                locale: None,
                plural_form: plural.map(|p| p.to_string()),
                placeholders: crate::placeholders::extract_placeholders(value.unwrap_or("")),
            })
            .collect();
        crate::parsers::apply_plural_suffixes(&mut units);
        StringResource {
            path: path.into(),
            format: CatalogueFormat::Json,
            declared_locale: None,
            embedded_locale: None,
            units,
        }
    }

    fn codes_of(violations: &[Violation]) -> Vec<String> {
        violations.iter().map(|v| v.code.clone()).collect()
    }

    #[test]
    fn missing_key_reported() {
        let baseline = resource("en.json", vec![("a", Some("Hello"), None)]);
        let target = resource("fr.json", vec![]);
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert_eq!(codes_of(&v), vec![codes::MISSING_TRANSLATION]);
        assert_eq!(v[0].severity, Severity::Error);
        assert_eq!(v[0].confidence, Confidence::Proven);
        assert_eq!(
            v[0].details[0],
            "key 'a' missing in fr.json"
        );
    }

    #[test]
    fn untranslated_value_reported() {
        let baseline = resource("en.json", vec![("a", Some("Hello"), None)]);
        let target = resource("fr.json", vec![("a", Some("hello"), None)]);
        let target = StringResource {
            declared_locale: Some(loc("fr")),
            ..target
        };
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert_eq!(codes_of(&v), vec![codes::UNTRANSLATED_VALUE]);
        assert_eq!(v[0].severity, Severity::Warning);
        assert_eq!(v[0].locale.as_ref().map(|l| l.canonical.as_str()), Some("fr"));
    }

    #[test]
    fn placeholder_mismatch_reported() {
        let baseline = resource("en.json", vec![("a", Some("Hi {name}"), None)]);
        let target = resource("fr.json", vec![("a", Some("Salut {nom}"), None)]);
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert_eq!(codes_of(&v), vec![codes::INCONSISTENT_PLACEHOLDERS]);
        assert!(v[0].details[0].contains("{name}"));
        assert!(v[0].details[0].contains("{nom}"));
    }

    #[test]
    fn orphan_and_duplicate_reported() {
        let baseline = resource("en.json", vec![("a", Some("A"), None)]);
        let target = resource(
            "fr.json",
            vec![("a", Some("un"), None), ("a", Some("une"), None), ("z", Some("Z"), None)],
        );
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        // Sorted by (file, line, code): OGMA-I18N-003 precedes OGMA-I18N-013.
        assert_eq!(codes_of(&v), vec![codes::ORPHANED_KEY, codes::DUPLICATE_KEY]);
    }

    #[test]
    fn orphan_excused_by_source_keys() {
        let baseline = resource("en.json", vec![("a", Some("A"), None)]);
        let target = resource("fr.json", vec![("a", Some("un"), None), ("z", Some("Z"), None)]);
        let source: BTreeSet<String> = ["z".to_string()].into_iter().collect();
        let v = compare_catalogues(&baseline, &target, &[], &source);
        assert!(v.is_empty());
    }

    #[test]
    fn missing_plural_categories_reported() {
        let baseline = resource(
            "en.json",
            vec![
                ("items_one", Some("one item"), None),
                ("items_other", Some("{n} items"), None),
            ],
        );
        let target = resource("fr.json", vec![("items", Some("un item"), Some("one"))]);
        let v = compare_catalogues(
            &baseline,
            &target,
            &["one".to_string(), "other".to_string()],
            &BTreeSet::new(),
        );
        assert_eq!(codes_of(&v), vec![codes::MISSING_PLURAL_FORM]);
        assert!(v[0].details[0].contains("other"));
    }

    #[test]
    fn fewer_plural_arms_than_baseline_reported() {
        let baseline = resource(
            "en.json",
            vec![
                ("items_one", Some("one"), None),
                ("items_few", Some("few"), None),
                ("items_other", Some("other"), None),
            ],
        );
        let target = resource(
            "fr.json",
            vec![("items", Some("un"), Some("one")), ("items", Some("autres"), Some("other"))],
        );
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert_eq!(codes_of(&v), vec![codes::MISSING_PLURAL_FORM]);
    }

    #[test]
    fn empty_baseline_key_skipped() {
        let baseline = resource("en.json", vec![("a", None, None), ("b", Some(""), None)]);
        let target = resource("fr.json", vec![]);
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert!(v.is_empty());
    }

    #[test]
    fn all_empty_target_arms_is_missing() {
        let baseline = resource(
            "en.json",
            vec![("items_one", Some("one"), None), ("items_other", Some("other"), None)],
        );
        let target = resource(
            "fr.json",
            vec![("items", None, Some("one")), ("items", None, Some("other"))],
        );
        let v = compare_catalogues(&baseline, &target, &[], &BTreeSet::new());
        assert_eq!(codes_of(&v), vec![codes::MISSING_TRANSLATION]);
    }

    #[test]
    fn coverage_computed() {
        let baseline = resource(
            "en.json",
            vec![
                ("a", Some("A"), None),
                ("b", Some("B"), None),
                ("c", Some("C"), None),
                ("empty", None, None),
            ],
        );
        let target = resource("fr.json", vec![("a", Some("un"), None), ("b", None, None)]);
        assert_eq!(coverage(&baseline, &target), 1.0 / 3.0);
    }

    #[test]
    fn coverage_plural_group_counts_once() {
        let baseline = resource(
            "en.json",
            vec![("items_one", Some("one"), None), ("items_other", Some("other"), None), ("x", Some("X"), None)],
        );
        let target = resource("fr.json", vec![("items", Some("un"), Some("one")), ("items", None, Some("other"))]);
        assert_eq!(coverage(&baseline, &target), 0.5);
    }

    #[test]
    fn coverage_empty_baseline_is_one() {
        let baseline = resource("en.json", vec![]);
        let target = resource("fr.json", vec![]);
        assert_eq!(coverage(&baseline, &target), 1.0);
    }

    #[test]
    fn comparison_is_deterministic() {
        let baseline = resource(
            "en.json",
            vec![
                ("a", Some("Hello {name}"), None),
                ("gone", Some("Gone"), None),
                ("items_one", Some("one"), None),
                ("items_other", Some("{n} other"), None),
            ],
        );
        let target = resource(
            "fr.json",
            vec![
                ("a", Some("Hello {name}"), None),
                ("items", Some("un"), Some("one")),
                ("extra", Some("extra"), None),
                ("extra", Some("encore"), None),
            ],
        );
        let required = vec!["one".to_string(), "other".to_string(), "many".to_string()];
        let first = compare_catalogues(&baseline, &target, &required, &BTreeSet::new());
        let second = compare_catalogues(&baseline, &target, &required, &BTreeSet::new());
        assert_eq!(first, second);
        // Sorted by (file, line, code): DUPLICATE before INCONSISTENT? No —
        // codes sort lexically; assert the global sort key ordering instead.
        for w in first.windows(2) {
            let a = (&w[0].file, w[0].line.unwrap_or(0), &w[0].code, &w[0].message);
            let b = (&w[1].file, w[1].line.unwrap_or(0), &w[1].code, &w[1].message);
            assert!(a <= b);
        }
    }
}
