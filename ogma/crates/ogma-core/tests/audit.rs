use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ogma_core::{run_audit, AuditConfig};
use ogma_model::{codes, BaselinePolicy, Confidence, LocaleId, OverallStatus, Policy, Severity};

static FIXTURE_SEQ: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let seq = FIXTURE_SEQ.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("ogma-core-test-{}-{}-{}", name, std::process::id(), seq));
    let _ = fs::remove_dir_all(&dir);
    for (path, content) in files {
        let p = dir.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }
    dir
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

fn i18next_app() -> PathBuf {
    fixture(
        "i18next",
        &[
            (
                "package.json",
                r#"{ "name": "demo-app", "dependencies": { "react": "18.0.0", "i18next": "23.0.0" } }
"#,
            ),
            (
                "src/i18n.js",
                "i18next.init({ fallbackLng: 'en', defaultLocale: 'en' });\n",
            ),
            (
                "src/App.jsx",
                "const label = t('welcome');\nbutton.setText('Delete account');\nconst input = <input aria-label=\"Search the catalog\" />;\n",
            ),
            (
                "locales/en.json",
                "{\"welcome\": \"Welcome\", \"goodbye\": \"Goodbye {name}\", \"title\": \"Settings\", \"retry\": \"Retry {count} times\"}\n",
            ),
            (
                "locales/fr.json",
                "{\"welcome\": \"Bienvenue\", \"title\": \"Settings\", \"retry\": \"Réessayer {nombre} fois\"}\n",
            ),
        ],
    )
}

#[test]
fn i18next_app_audit() {
    let root = i18next_app();
    let result = run_audit(&root, &AuditConfig::default()).unwrap();

    assert_eq!(result.application_name.as_deref(), Some("demo-app"));
    assert_eq!(result.baseline.language.as_ref().map(|l| l.code()), Some("en"));
    assert_eq!(result.baseline.confidence, Confidence::Proven);

    let frameworks: Vec<&str> = result.frameworks.iter().map(|s| s.as_str()).collect();
    assert_eq!(frameworks, vec!["generic", "react"]);

    let en = result.locales.iter().find(|r| r.locale.canonical == "en").unwrap();
    assert!(en.native, "baseline locale must be native: {:?}", en.checks);
    assert_eq!(en.coverage, 1.0);

    let fr = result.locales.iter().find(|r| r.locale.canonical == "fr").unwrap();
    assert!(!fr.native);
    let fr_codes: Vec<&str> = fr.violations.iter().map(|v| v.code.as_str()).collect();
    assert!(fr_codes.contains(&codes::COVERAGE_BELOW_THRESHOLD), "{fr_codes:?}");
    assert!(fr_codes.contains(&codes::MISSING_TRANSLATION), "{fr_codes:?}");
    assert!(fr_codes.contains(&codes::INCONSISTENT_PLACEHOLDERS), "{fr_codes:?}");
    assert!(fr_codes.contains(&codes::UNTRANSLATED_VALUE), "{fr_codes:?}");

    let hard = result
        .violations
        .iter()
        .filter(|v| v.code == codes::HARD_CODED_STRING)
        .collect::<Vec<_>>();
    assert_eq!(hard.len(), 1);
    assert_eq!(hard[0].severity, Severity::Warning);
    assert_eq!(hard[0].file.as_ref().unwrap().to_str().unwrap(), "src/App.jsx");
    assert_eq!(hard[0].line, Some(2));
    assert_eq!(hard[0].message, "2 hard-coded user-facing strings");
    assert_eq!(
        hard[0].details,
        vec![
            "line 2: Delete account".to_string(),
            "line 3: Search the catalog".to_string(),
        ]
    );

    assert_eq!(result.linguistic_surface.total_units, 4);
    assert_eq!(result.linguistic_surface.hard_coded_occurrences, 2);
    assert_eq!(result.linguistic_surface.by_kind["user_facing"], 3);
    assert_eq!(result.linguistic_surface.by_kind.get("developer_facing"), None);

    assert_eq!(result.overall, OverallStatus::Fail);
    cleanup(&root);
}

#[test]
fn rust_cli_without_catalogue_passes() {
    let root = fixture(
        "rust-cli",
        &[("src/main.rs", "fn main() { println!(\"Hello world\"); }\n")],
    );
    let result = run_audit(&root, &AuditConfig::default()).unwrap();

    assert_eq!(result.baseline.language, None);
    assert_eq!(result.baseline.confidence, Confidence::Unknown);
    assert_eq!(result.locales.len(), 1);
    let en = &result.locales[0];
    assert_eq!(en.locale.canonical, "en");
    assert!(en.native, "baseline-only locale must be native: {:?}", en.checks);
    assert_eq!(en.coverage, 1.0);

    // Hard-coded strings are Warnings, never Errors: overall stays Pass.
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].code, codes::HARD_CODED_STRING);
    assert_eq!(result.violations[0].severity, Severity::Warning);
    assert_eq!(result.overall, OverallStatus::Pass);
    cleanup(&root);
}

#[test]
fn audit_is_deterministic_and_cache_round_trips() {
    let root = i18next_app();

    let first = run_audit(&root, &AuditConfig::default()).unwrap();
    assert!(root.join(".ogma/cache.json").exists());
    assert!(root.join(".ogma/occurrences.json").exists());

    let second = run_audit(&root, &AuditConfig::default()).unwrap();
    assert_eq!(format!("{first:?}"), format!("{second:?}"));

    // Cache content: hashes for every Source file, occurrences keyed by path.
    let cache_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join(".ogma/cache.json")).unwrap()).unwrap();
    assert_eq!(cache_json["version"], 1);
    assert_eq!(cache_json["files"]["src/App.jsx"].as_u64().is_some(), true);
    assert_eq!(cache_json["files"]["src/i18n.js"].as_u64().is_some(), true);
    let occ_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".ogma/occurrences.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(occ_json["src/App.jsx"].as_array().unwrap().len(), 3);

    // Modify one file: the other file's occurrences must still be reused and
    // the merged result must be identical to a cold run.
    fs::write(
        root.join("src/i18n.js"),
        "i18next.init({ fallbackLng: 'en', defaultLocale: 'en' });\nconst unused = 1;\n",
    )
    .unwrap();
    let third = run_audit(&root, &AuditConfig::default()).unwrap();
    let occ_json2: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".ogma/occurrences.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(occ_json2["src/App.jsx"].as_array().unwrap().len(), 3);
    assert_eq!(format!("{first:?}"), format!("{third:?}"));

    let _ = fs::remove_dir_all(root.join(".ogma"));
    let cold = run_audit(&root, &AuditConfig::default()).unwrap();
    assert_eq!(format!("{first:?}"), format!("{cold:?}"));
    cleanup(&root);
}

#[test]
fn missing_required_locale_fails() {
    let root = i18next_app();
    let config = AuditConfig {
        policy: Policy {
            required_locales: vec![ogma_locale::parse_locale("de").unwrap()],
            ..Policy::default()
        },
        incremental: false,
    };
    let result = run_audit(&root, &config).unwrap();

    let de = result.locales.iter().find(|r| r.locale.canonical == "de").unwrap();
    assert!(!de.native);
    assert_eq!(de.coverage, 0.0);
    assert!(de.violations.iter().any(|v| v.code == codes::MISSING_TRANSLATION));
    assert_eq!(result.overall, OverallStatus::Fail);
    cleanup(&root);
}

fn rtl_app() -> PathBuf {
    fixture(
        "rtl",
        &[("locales/ar.json", "{\"hello\": \"مرحبا بالعالم\"}\n")],
    )
}

#[test]
fn rtl_locale_requires_direction_evidence() {
    let root = rtl_app();
    let result = run_audit(&root, &AuditConfig::default()).unwrap();

    let ar = result.locales.iter().find(|r| r.locale.canonical == "ar").unwrap();
    assert!(!ar.native);
    assert_eq!(
        ar.checks["text_direction"].status,
        ogma_model::CheckStatus::Unknown
    );
    assert_eq!(result.overall, OverallStatus::Fail);
    cleanup(&root);
}

#[test]
fn rtl_locale_passes_when_unknown_allowed() {
    let root = rtl_app();
    let config = AuditConfig {
        policy: Policy { allow_unknown: true, ..Policy::default() },
        incremental: false,
    };
    let result = run_audit(&root, &config).unwrap();

    let ar = result.locales.iter().find(|r| r.locale.canonical == "ar").unwrap();
    assert!(ar.native, "ar checks: {:?}", ar.checks);
    assert_eq!(result.overall, OverallStatus::Pass);
    cleanup(&root);
}

#[test]
fn incremental_disabled_leaves_no_cache_dir() {
    let root = i18next_app();
    let config = AuditConfig { policy: Policy::default(), incremental: false };
    let result = run_audit(&root, &config).unwrap();
    assert_eq!(result.baseline.language.as_ref().map(|l| l.code()), Some("en"));
    assert!(!root.join(".ogma").exists());
    cleanup(&root);
}

#[test]
fn explicit_policy_baseline_overrides_detection() {
    let root = i18next_app();
    let config = AuditConfig {
        policy: Policy {
            baseline: BaselinePolicy::Explicit(LocaleId::canonical_only("en")),
            ..Policy::default()
        },
        incremental: false,
    };
    let result = run_audit(&root, &config).unwrap();
    assert_eq!(result.baseline.confidence, Confidence::Proven);
    assert_eq!(result.baseline.language.as_ref().map(|l| l.code()), Some("en"));
    cleanup(&root);
}

#[test]
fn unreadable_root_is_analysis_error() {
    let err = run_audit(Path::new("/definitely/not/a/real/ogma/path"), &AuditConfig::default())
        .unwrap_err();
    assert!(err.to_string().starts_with("analysis error:"));
}
