//! Deterministic renderers for Ogma audit results: human-readable text,
//! pretty JSON, and SARIF 2.1.0, plus per-violation-code explanations.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ogma_model::{
    codes, AuditResult, Confidence, LocaleReport, OverallStatus, Policy, Severity, Violation,
};
use serde_json::json;

/// Render the human-readable audit report (the format shown by `ogma audit`).
pub fn render_text(result: &AuditResult, policy: &Policy) -> String {
    let mut out = String::new();

    out.push_str("OGMA 0.1\n");
    out.push_str("Deterministic Multilingual Conformance\n\n");

    out.push_str("Application\n");
    let name = result.application_name.as_deref().unwrap_or("(unnamed)");
    field_line(&mut out, "Name:", name);
    let baseline = result
        .baseline
        .language
        .as_ref()
        .map(|l| l.code().to_string())
        .unwrap_or_else(|| "undetected".to_string());
    field_line(&mut out, "Baseline:", &baseline);
    let frameworks = if result.frameworks.is_empty() {
        "-".to_string()
    } else {
        result.frameworks.join(", ")
    };
    field_line(&mut out, "Frameworks:", &frameworks);
    field_line(&mut out, "Locales:", &result.locales.len().to_string());
    let surface = format!(
        "{} units, {} user-facing strings ({} hard-coded)",
        group_digits(result.linguistic_surface.total_units),
        group_digits(result.linguistic_surface.user_facing_occurrences),
        group_digits(result.linguistic_surface.hard_coded_occurrences),
    );
    field_line(&mut out, "Surface:", &surface);

    out.push('\n');
    push_locale_table(&mut out, result, policy);

    if !result.violations.is_empty()
        || result
            .locales
            .iter()
            .any(|l| !l.violations.is_empty())
    {
        out.push_str("\nViolations\n\n");
        if !result.violations.is_empty() {
            out.push_str("GLOBAL\n");
            for v in sorted_violations(&result.violations) {
                push_violation_line(&mut out, v);
            }
        }
        for locale in &result.locales {
            if locale.violations.is_empty() {
                continue;
            }
            out.push_str(&locale.locale.canonical.to_uppercase());
            out.push('\n');
            for v in sorted_violations(&locale.violations) {
                push_violation_line(&mut out, v);
            }
        }
    }

    out.push('\n');
    let overall = match result.overall {
        OverallStatus::Pass => "PASSED",
        OverallStatus::Fail => "FAILED",
        OverallStatus::Unknown => "UNKNOWN",
    };
    out.push_str(&format!("Overall: {overall}\n"));

    if policy.translation_coverage < 1.0 || policy.allow_fallback {
        out.push_str("\nPolicy\n");
        if policy.translation_coverage < 1.0 {
            out.push_str(&format!(
                "  coverage >= {:.1}%\n",
                policy.translation_coverage * 100.0
            ));
        }
        if policy.allow_fallback {
            out.push_str("  fallback allowed\n");
        }
    }

    out
}

/// Render the audit result as deterministic pretty JSON.
pub fn render_json(result: &AuditResult) -> String {
    serde_json::to_string_pretty(result)
        .expect("AuditResult is always serialisable")
}

/// Render the audit result as SARIF 2.1.0 for CI/security-style tooling.
pub fn render_sarif(result: &AuditResult) -> String {
    let mut all = Vec::new();
    all.extend(result.violations.iter());
    for locale in &result.locales {
        all.extend(locale.violations.iter());
    }

    let mut rule_ids: BTreeSet<&str> = BTreeSet::new();
    for v in &all {
        rule_ids.insert(v.code.as_str());
    }
    let rules: Vec<_> = rule_ids
        .iter()
        .map(|code| {
            let severity = all
                .iter()
                .filter(|v| v.code.as_str() == *code)
                .map(|v| v.severity)
                .max()
                .unwrap_or(Severity::Info);
            json!({
                "id": code,
                "shortDescription": { "text": short_description(code) },
                "defaultConfiguration": { "level": level(severity) },
            })
        })
        .collect();

    let mut results = Vec::new();
    let mut push = |v: &Violation| {
        let message_text = match &v.locale {
            Some(l) => format!("[{}] {}", l.canonical.to_uppercase(), v.message),
            None => v.message.clone(),
        };
        let mut result = json!({
            "ruleId": v.code,
            "level": level(v.severity),
            "message": { "text": message_text },
        });
        if let Some(f) = &v.file {
            let mut physical_location = json!({
                "artifactLocation": {
                    "uri": uri_string(f),
                    "uriBaseId": "SRCROOT",
                },
            });
            if let Some(line) = v.line {
                physical_location["region"] = json!({ "startLine": line });
            }
            result["locations"] = json!([{ "physicalLocation": physical_location }]);
        }
        results.push(result);
    };
    for v in sorted_violations(&result.violations) {
        push(v);
    }
    for locale in &result.locales {
        for v in sorted_violations(&locale.violations) {
            push(v);
        }
    }

    let log = json!({
        "version": "2.1.0",
        "$schema": "https://json.schemas.microsoft.com/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "ogma",
                    "version": "0.1.0",
                    "rules": rules,
                },
            },
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&log).expect("SARIF structure is always serialisable")
}

/// Explain a violation: what the code means, why it matters, how to fix it.
/// Unknown codes get a generic honest explanation.
pub fn explain(code: &str) -> String {
    let entry = match code {
        codes::HARD_CODED_STRING => (
            "Hard-coded user-facing string",
            "A string literal that end users will see was found in source outside the project's i18n lookup mechanism: no t(\"key\"), gettext(\"...\"), tr!(\"...\"), or equivalent wraps it. It is rendered verbatim in the developer's language for every user.",
            "Such strings can never be translated, so every hard-coded user-facing string is direct evidence against a claim that a locale is natively supported. They also defeat plural, gender, and formatting rules for every non-baseline language.",
            "Replace the literal with a translation-key call (t(\"settings.title\"), gettext(\"Save\"), tr!(\"save_button\")) and add the key with its baseline-language value to the primary catalogue.",
        ),
        codes::MISSING_TRANSLATION => (
            "Missing translation for required key",
            "A key that exists in the baseline linguistic surface has no entry in the catalogue of a locale the policy requires. The runtime will have nothing to display for that key in that locale.",
            "End users of the affected locale see fallback text, empty labels, or broken UI. A locale with untranslated required keys is not natively supported, whatever the coverage numbers claim.",
            "Add the missing key to the locale's catalogue (locales/de.json, messages.po, Localizable.strings, ...) with a value translated for that locale, or relax required_locales in the policy if the locale is intentionally optional.",
        ),
        codes::ORPHANED_KEY => (
            "Orphaned catalogue key",
            "A key exists in a translation catalogue but no source reference (t(), gettext, tr!(), formatMessage, ...) points to it anywhere in the scanned surface.",
            "Orphaned keys inflate catalogues, cost translator effort, and usually indicate dead UI or a refactor that dropped a call site. They make coverage numbers look better than the real surface is.",
            "Remove the key from every catalogue, or restore the source reference if the string is still meant to be shown.",
        ),
        codes::MISSING_PLURAL_FORM => (
            "Missing plural form",
            "A locale requires plural categories defined by CLDR (one, few, many, other, ...) that the catalogue does not provide for a key whose baseline has plural arms.",
            "Without the required forms the runtime picks a fallback arm, producing grammatically wrong or misleading text (\"1 items\", \"2 fichier\") for speakers of that locale.",
            "Add the missing plural arms using the platform's mechanism: ICU MessageFormat plural/select, gettext msgid_plural, or Apple .stringsdict entries, following CLDR plural rules for the locale.",
        ),
        codes::INCONSISTENT_PLACEHOLDERS => (
            "Inconsistent placeholders",
            "The interpolation placeholders in the target value differ from the baseline set: names, counts, or styles mismatch ({name} vs %s vs {{name}}), or a placeholder is missing/added.",
            "At runtime the interpolation either fails, throws, or silently drops data, producing malformed user-facing text exactly where the locale matters most.",
            "Make the target placeholder set match the baseline exactly, reordering placeholders as the target grammar requires but never renaming, adding, or dropping them.",
        ),
        codes::UNTRANSLATED_VALUE => (
            "Untranslated value",
            "The value for a key in a target locale is identical to the baseline value, so no actual translation was supplied.",
            "Identical values usually mean the string was copied during scaffolding and never translated, or that an English string leaked into a non-baseline catalogue. Either way the locale is not really supported for that key.",
            "Translate the value for the locale. If sharing is intentional (proper names, brand terms), declare the key as shared in policy so it stops counting as evidence against the locale.",
        ),
        codes::MALFORMED_PLURAL => (
            "Malformed plural rule",
            "A plural rule expression in a catalogue could not be parsed: the ICU plural/select syntax is invalid, or a gettext plural-forms header is broken.",
            "A malformed rule makes plural behaviour undefined at runtime: the application may crash, fall back to the wrong arm, or ignore the locale's CLDR categories entirely.",
            "Correct the expression against the ICU MessageFormat spec or the gettext plural-forms grammar, and verify it against CLDR plural rules for the locale.",
        ),
        codes::LOCALE_INSENSITIVE_DATE => (
            "Locale-insensitive date/time formatting",
            "A date or time destined for users is formatted with a fixed pattern or default locale API instead of the user's locale (hard-coded strftime \"%m/%d/%Y\", SimpleDateFormat with a fixed pattern, concatenated components).",
            "Date component order, separators, and calendar differ across locales; a fixed pattern renders meaningless or misleading dates for most of the world's users.",
            "Route the value through a locale-aware formatter: Intl.DateTimeFormat with the user's locale, chrono/date formatting with an explicit locale parameter, or the framework's localised date helper.",
        ),
        codes::LOCALE_INSENSITIVE_NUMBER => (
            "Locale-insensitive number formatting",
            "A user-facing number is formatted with hard-coded decimal/grouping separators or a fixed pattern (\"1,000.00\", format!(\"{:.2}\"), toFixed + manual commas).",
            "Decimal and thousands separators differ across locales (1.000,00 in much of Europe; 1,000.00 in English), so a fixed format misrepresents amounts, sizes, and counts.",
            "Use a locale-aware number formatter: Intl.NumberFormat, the platform's localized currency/number APIs, or a formatting library configured with the user's locale.",
        ),
        codes::RTL_BIDI => (
            "RTL bidirectional support gap",
            "A right-to-left locale (ar, he, fa, ur, ...) is declared as supported, but the implementation shows no direction-aware evidence: no dir attribute, no CSS logical properties, no mirrored layout handling.",
            "Rendering an RTL language in a left-to-right layout produces unreadable UI: wrong alignment, broken punctuation flow, and icons that point the wrong way.",
            "Set text direction per locale (dir=\"rtl\", direction CSS with the locale), prefer logical CSS properties (margin-inline-start), and mirror asymmetric icons and animations.",
        ),
        codes::LANGUAGE_FALLBACK => (
            "Unacceptable fallback to another primary language",
            "A locale resolves content by falling back across primary languages (e.g. fr-CA -> en) while the policy disallows fallback.",
            "Users silently receive text in a language they did not choose; the application is not natively supporting the requested locale even though it appears in the locale list.",
            "Provide complete translations for the affected keys, or reconfigure the fallback chain to stay within the same primary language (fr-CA -> fr) and adjust policy.allow_fallback explicitly if cross-language fallback is intended.",
        ),
        codes::COVERAGE_BELOW_THRESHOLD => (
            "Translation coverage below policy threshold",
            "The fraction of the baseline linguistic surface translated for a locale is below the policy's translation_coverage minimum.",
            "Coverage is the aggregate proof of native support; below-threshold coverage means the locale is only partially supported regardless of how good the translated strings are.",
            "Translate the missing keys to raise coverage, or lower policy.translation_coverage deliberately if the current level is the accepted trade-off.",
        ),
        codes::DUPLICATE_KEY => (
            "Duplicate key in catalogue",
            "The same key is declared more than once inside a single catalogue file.",
            "Duplicate keys make behaviour loader-dependent (last-write-wins) and can silently flip which value users see, undermining determinism between builds and environments.",
            "Remove the duplicates from the catalogue, keeping the intended value, and add a lint or CI check so the key set stays unique.",
        ),
        other => {
            return format!(
                "No explanation is registered for code '{other}'. This may indicate a newer Ogma version produced it."
            )
        }
    };

    format!(
        "{code} — {}\n\nWhat it means: {}\nWhy it matters: {}\nHow to fix: {}\n",
        entry.0, entry.1, entry.2, entry.3
    )
}

fn short_description(code: &str) -> String {
    let full = explain(code);
    full.lines()
        .next()
        .and_then(|line| line.split(" — ").nth(1))
        .map(|s| s.to_string())
        .unwrap_or_else(|| code.to_string())
}

fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

fn uri_string(path: &PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn violation_key(v: &Violation) -> (String, String, usize) {
    (
        v.code.clone(),
        v.file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        v.line.unwrap_or(0),
    )
}

fn sorted_violations(violations: &[Violation]) -> Vec<&Violation> {
    let mut sorted: Vec<&Violation> = violations.iter().collect();
    sorted.sort_by_key(|v| violation_key(v));
    sorted
}

fn location_string(v: &Violation) -> String {
    match (&v.file, v.line) {
        (Some(f), Some(l)) => format!("{}:{l}", f.display()),
        (Some(f), None) => f.display().to_string(),
        (None, _) => "-".to_string(),
    }
}

fn push_violation_line(out: &mut String, v: &Violation) {
    out.push_str(&format!(
        "  {}  {}  {}\n",
        v.code,
        location_string(v),
        v.message
    ));
}

fn field_line(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("  {label:<13}{value}\n"));
}

fn locale_status(report: &LocaleReport, policy: &Policy) -> &'static str {
    let check_fail = report
        .checks
        .values()
        .any(|c| !policy.outcome_satisfies(c, Confidence::Proven));
    let has_error = report
        .violations
        .iter()
        .any(|v| v.severity == Severity::Error);
    if check_fail || has_error {
        "FAIL"
    } else if report.violations.is_empty() {
        "PASS"
    } else {
        "UNKNOWN"
    }
}

fn push_locale_table(out: &mut String, result: &AuditResult, policy: &Policy) {
    let name_width = result
        .locales
        .iter()
        .map(|l| l.locale.canonical.len())
        .max()
        .unwrap_or(0)
        .max(14);
    let header = format!(
        "{:<nw$}  {:>8}    {:<6}    {}",
        "Locale",
        "Coverage",
        "Native",
        "Status",
        nw = name_width
    );
    out.push_str(&header);
    out.push('\n');
    out.push_str(&"-".repeat(header.chars().count()));
    out.push('\n');
    for report in &result.locales {
        out.push_str(&format!(
            "{:<nw$}  {:>8}    {:<6}    {}\n",
            report.locale.canonical,
            format!("{:.1}%", report.coverage * 100.0),
            if report.native { "YES" } else { "NO" },
            locale_status(report, policy),
            nw = name_width
        ));
    }
}

fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}


#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use ogma_model::{
        codes, AuditResult, BaselineDetection, CheckOutcome, CheckStatus, Confidence,
        Language, LinguisticSurface, LocaleId, LocaleReport, OverallStatus, Policy, Severity,
        Violation,
    };

    use super::{explain, render_json, render_sarif, render_text};

    fn violation(
        code: &str,
        severity: Severity,
        message: &str,
        file: Option<&str>,
        line: Option<usize>,
        locale: Option<LocaleId>,
    ) -> Violation {
        Violation {
            code: code.to_string(),
            severity,
            message: message.to_string(),
            file: file.map(PathBuf::from),
            line,
            locale,
            confidence: Confidence::Proven,
            details: Vec::new(),
        }
    }

    fn sample_result() -> AuditResult {
        let de = LocaleId::canonical_only("de-DE");
        let mut checks = BTreeMap::new();
        checks.insert(
            "coverage".to_string(),
            CheckOutcome {
                status: CheckStatus::Pass,
                detail: "fully translated".to_string(),
            },
        );
        AuditResult {
            application_name: Some("ExampleApp".to_string()),
            baseline: BaselineDetection {
                language: Some(Language::new("en")),
                confidence: Confidence::Proven,
                evidence: Vec::new(),
            },
            frameworks: vec!["React".to_string()],
            locales: vec![
                LocaleReport {
                    locale: LocaleId::canonical_only("en-GB"),
                    coverage: 1.0,
                    native: true,
                    fallback_dependencies: Vec::new(),
                    checks: checks.clone(),
                    violations: Vec::new(),
                },
                LocaleReport {
                    locale: LocaleId::canonical_only("fr-FR"),
                    coverage: 0.998,
                    native: true,
                    fallback_dependencies: Vec::new(),
                    checks: checks.clone(),
                    violations: Vec::new(),
                },
                LocaleReport {
                    locale: de.clone(),
                    coverage: 0.971,
                    native: false,
                    fallback_dependencies: Vec::new(),
                    checks,
                    violations: vec![
                        violation(
                            codes::MISSING_TRANSLATION,
                            Severity::Error,
                            "3 missing translations",
                            Some("locales/de.json"),
                            None,
                            Some(de.clone()),
                        ),
                        violation(
                            codes::COVERAGE_BELOW_THRESHOLD,
                            Severity::Warning,
                            "coverage 97.1% below threshold",
                            None,
                            None,
                            Some(de),
                        ),
                    ],
                },
            ],
            linguistic_surface: LinguisticSurface {
                total_units: 312,
                user_facing_occurrences: 1482,
                hard_coded_occurrences: 17,
                by_kind: BTreeMap::new(),
            },
            violations: vec![violation(
                codes::HARD_CODED_STRING,
                Severity::Error,
                "Hard-coded user-facing string",
                Some("src/settings.rs"),
                Some(184),
                None,
            )],
            overall: OverallStatus::Fail,
        }
    }

    #[test]
    fn text_report_contains_key_lines() {
        let text = render_text(&sample_result(), &Policy::default());
        assert!(text.contains("OGMA 0.1"), "header missing:\n{text}");
        assert!(text.contains("Deterministic Multilingual Conformance"));
        assert!(text.contains("Name:        ExampleApp"), "name line:\n{text}");
        assert!(text.contains("Baseline:    en"), "baseline line:\n{text}");
        assert!(text.contains("Frameworks:  React"));
        assert!(text.contains("Locales:     3"));
        assert!(
            text.contains("Surface:     312 units, 1,482 user-facing strings (17 hard-coded)"),
            "surface line:\n{text}"
        );
        assert!(text.contains("Locale"), "table header:\n{text}");
        assert!(text.contains("en-GB"), "locale row:\n{text}");
        assert!(text.contains("100.0%"));
        assert!(text.contains("99.8%"));
        assert!(text.contains("97.1%"));
        assert!(text.contains("YES"));
        assert!(text.contains("NO"));
        assert!(text.contains("PASS"));
        assert!(text.contains("FAIL"));
        assert!(text.contains("Violations"), "violations section:\n{text}");
        assert!(text.contains("GLOBAL"));
        assert!(text.contains("OGMA-I18N-001  src/settings.rs:184  Hard-coded user-facing string"));
        assert!(text.contains("DE-DE"));
        assert!(text.contains("OGMA-I18N-002  locales/de.json  3 missing translations"));
        assert!(text.contains("Overall: FAILED"), "overall line:\n{text}");
        assert!(!text.contains("Policy"), "policy block must be hidden by default:\n{text}");
    }

    #[test]
    fn text_report_handles_unnamed_and_undetected() {
        let mut result = sample_result();
        result.application_name = None;
        result.baseline.language = None;
        result.frameworks.clear();
        let text = render_text(&result, &Policy::default());
        assert!(text.contains("Name:        (unnamed)"), ":\n{text}");
        assert!(text.contains("Baseline:    undetected"), ":\n{text}");
        assert!(text.contains("Frameworks:  -"), ":\n{text}");
    }

    #[test]
    fn text_report_omits_violations_section_when_empty() {
        let mut result = sample_result();
        result.violations.clear();
        result.locales[2].violations.clear();
        result.overall = OverallStatus::Pass;
        let text = render_text(&result, &Policy::default());
        assert!(!text.contains("Violations"), ":\n{text}");
        assert!(text.contains("Overall: PASSED"));
    }

    #[test]
    fn text_report_shows_policy_echo_when_relaxed() {
        let policy = Policy {
            translation_coverage: 0.95,
            allow_fallback: true,
            ..Policy::default()
        };
        let text = render_text(&sample_result(), &policy);
        assert!(text.contains("Policy"), ":\n{text}");
        assert!(text.contains("  coverage >= 95.0%"), ":\n{text}");
        assert!(text.contains("  fallback allowed"), ":\n{text}");
    }

    #[test]
    fn text_report_sorts_violations_within_groups() {
        let mut result = sample_result();
        result.violations = vec![
            violation(
                codes::DUPLICATE_KEY,
                Severity::Warning,
                "dup",
                Some("b.txt"),
                Some(2),
                None,
            ),
            violation(
                codes::DUPLICATE_KEY,
                Severity::Warning,
                "dup",
                Some("a.txt"),
                Some(9),
                None,
            ),
            violation(
                codes::HARD_CODED_STRING,
                Severity::Error,
                "hard",
                Some("z.txt"),
                Some(1),
                None,
            ),
        ];
        let text = render_text(&result, &Policy::default());
        let i001 = text.find("OGMA-I18N-001").unwrap();
        let i_a = text.find("OGMA-I18N-013  a.txt:9").unwrap();
        let i_b = text.find("OGMA-I18N-013  b.txt:2").unwrap();
        assert!(i001 < i_a && i_a < i_b, "sorted order violated:\n{text}");
    }

    #[test]
    fn json_round_trips() {
        let result = sample_result();
        let json = render_json(&result);
        let parsed: AuditResult = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed, result);
    }

    #[test]
    fn sarif_is_valid_and_ordered() {
        let sarif = render_sarif(&sample_result());
        let value: serde_json::Value =
            serde_json::from_str(&sarif).expect("SARIF must parse as JSON");

        assert_eq!(value["version"], "2.1.0");
        assert_eq!(
            value["$schema"],
            "https://json.schemas.microsoft.com/sarif-2.1.0.json"
        );

        let run = &value["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], "ogma");
        assert_eq!(run["tool"]["driver"]["version"], "0.1.0");

        let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
        let rule_ids: Vec<&str> = rules
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            rule_ids,
            vec![
                codes::HARD_CODED_STRING,
                codes::MISSING_TRANSLATION,
                codes::COVERAGE_BELOW_THRESHOLD,
            ]
        );
        assert!(rules.iter().all(|r| !r["shortDescription"]["text"].as_str().unwrap().is_empty()));
        assert!(rules.iter().any(|r| r["defaultConfiguration"]["level"] == "error"));
        assert!(rules.iter().any(|r| r["defaultConfiguration"]["level"] == "warning"));

        let results = run["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["ruleId"], codes::HARD_CODED_STRING);
        assert_eq!(results[0]["level"], "error");
        let loc = &results[0]["locations"][0]["physicalLocation"];
        assert_eq!(loc["artifactLocation"]["uri"], "src/settings.rs");
        assert_eq!(loc["artifactLocation"]["uriBaseId"], "SRCROOT");
        assert_eq!(loc["region"]["startLine"], 184);
        assert_eq!(
            results[1]["message"]["text"],
            "[DE-DE] 3 missing translations"
        );
        assert_eq!(results[2]["ruleId"], codes::COVERAGE_BELOW_THRESHOLD);
        assert!(results[2].get("locations").is_none(), "no location key expected");
    }

    #[test]
    fn sarif_converts_windows_separators() {
        let mut result = sample_result();
        result.violations[0].file = Some(PathBuf::from("src\\deep\\settings.rs"));
        let value: serde_json::Value = serde_json::from_str(&render_sarif(&result)).unwrap();
        assert_eq!(
            value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                ["artifactLocation"]["uri"],
            "src/deep/settings.rs"
        );
    }

    #[test]
    fn explain_covers_all_codes() {
        let all_codes = [
            codes::HARD_CODED_STRING,
            codes::MISSING_TRANSLATION,
            codes::ORPHANED_KEY,
            codes::MISSING_PLURAL_FORM,
            codes::INCONSISTENT_PLACEHOLDERS,
            codes::UNTRANSLATED_VALUE,
            codes::MALFORMED_PLURAL,
            codes::LOCALE_INSENSITIVE_DATE,
            codes::LOCALE_INSENSITIVE_NUMBER,
            codes::RTL_BIDI,
            codes::LANGUAGE_FALLBACK,
            codes::COVERAGE_BELOW_THRESHOLD,
            codes::DUPLICATE_KEY,
        ];
        assert_eq!(all_codes.len(), 13);
        for code in all_codes {
            let text = explain(code);
            assert!(text.starts_with(code), "must start with the code:\n{text}");
            assert!(text.contains("What it means:"), "{code}:\n{text}");
            assert!(text.contains("Why it matters:"), "{code}:\n{text}");
            assert!(text.contains("How to fix:"), "{code}:\n{text}");
            assert!(text.lines().count() >= 5, "{code} too short:\n{text}");
        }
    }

    #[test]
    fn explain_unknown_code_is_honest() {
        let text = explain("OGMA-I18N-999");
        assert_eq!(
            text,
            "No explanation is registered for code 'OGMA-I18N-999'. This may indicate a newer Ogma version produced it."
        );
    }

    #[test]
    fn rendering_is_deterministic_and_order_independent() {
        let base = sample_result();

        let mut shuffled = sample_result();
        shuffled.locales[2].violations.reverse();
        shuffled.violations.reverse();

        assert_eq!(render_text(&base, &Policy::default()), render_text(&base, &Policy::default()));
        assert_eq!(render_json(&base), render_json(&base));
        assert_eq!(render_sarif(&base), render_sarif(&base));
        assert_eq!(render_text(&base, &Policy::default()), render_text(&shuffled, &Policy::default()));
        assert_eq!(render_sarif(&base), render_sarif(&shuffled));
    }
}
