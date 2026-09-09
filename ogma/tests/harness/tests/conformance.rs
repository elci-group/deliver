//! Fixture-driven conformance tests for Ogma.
//!
//! Every fixture in `fixtures/<name>/` carries a `fixture.toml` expectation
//! file. Each fixture is audited twice with its declared policy and the
//! `AuditResult` is compared field-by-field against the expectations
//! (baseline, locale set, native flags, exact violation-code sets per scope,
//! per-code file sets, frameworks, forbidden text). Two consecutive runs must
//! also render byte-identical JSON. The `rtl-app` fixture is additionally
//! audited under the DEFAULT policy to prove the RTL gate fails closed.

use std::path::PathBuf;

use ogma_fixture_tests as harness;
use ogma_model::{OverallStatus, Policy};

const EXPECTED_FIXTURE_COUNT: usize = 14;

#[test]
fn all_fixtures_conform() {
    let fixtures = harness::discover_fixtures();
    assert_eq!(
        fixtures.len(),
        EXPECTED_FIXTURE_COUNT,
        "expected {EXPECTED_FIXTURE_COUNT} fixtures under {}, found {}: {:?}",
        harness::fixtures_root().display(),
        fixtures.len(),
        fixtures.iter().map(|f| &f.name).collect::<Vec<_>>()
    );

    let mut failures = Vec::new();
    for fx in &fixtures {
        let first = harness::audit_fixture(fx);
        let second = harness::audit_fixture(fx);
        let mut problems = harness::check(&first, fx);
        if harness::render_json(&first) != harness::render_json(&second) {
            problems.push("render_json differed between two consecutive runs".to_string());
        }
        for p in problems {
            failures.push(format!("{}: {p}", fx.name));
        }
    }
    assert!(failures.is_empty(), "{} fixture(s) diverged:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn rtl_app_fails_under_default_policy() {
    let fx = harness::load_fixture(
        "rtl-app",
        &harness::fixtures_root().join("rtl-app"),
    );
    // Re-audit the same tree under the strict default policy.
    let strict = harness::Fixture {
        policy: Policy::default(),
        ..fx.clone()
    };
    let result = ogma_core::run_audit(
        &strict.dir,
        &ogma_core::AuditConfig { policy: strict.policy.clone(), incremental: false },
    )
    .expect("rtl-app audit under default policy must run");
    assert_eq!(
        result.overall,
        OverallStatus::Fail,
        "rtl-app must FAIL under the default policy (ar text direction UNKNOWN)"
    );
    let ar = result
        .locales
        .iter()
        .find(|l| l.locale.canonical == "ar")
        .expect("rtl-app must report an ar locale");
    assert!(
        !ar.native,
        "ar must be non-native under the default policy (no direction evidence)"
    );
}

#[test]
fn complete_react_golden_text_report() {
    let fx = harness::load_fixture(
        "complete-react",
        &harness::fixtures_root().join("complete-react"),
    );
    let result = harness::audit_fixture(&fx);
    let actual = harness::render_text(&result, &fx);
    let golden: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join("complete-react.txt");

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &actual).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&golden).unwrap_or_else(|e| {
        panic!(
            "cannot read golden {}: {e} (run with UPDATE_GOLDEN=1 to regenerate)",
            golden.display()
        )
    });
    assert_eq!(
        expected, actual,
        "golden text report mismatch for complete-react; run with UPDATE_GOLDEN=1 to regenerate"
    );
}
