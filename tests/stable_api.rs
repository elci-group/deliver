//! Compile-time inventory of the 1.0 API guarantees from the crate docs:
//! public spec/report types keep their derive set and stay `Send + Sync`
//! (they are shared across the parallel worker pool), and result values
//! serialize deterministically.

use deliver::{
    cache::{Cache, CachedResult},
    incremental, CheckResult, CommandCheck, CommandSpec, DirectoryCheck, FileCheck, GitCheck,
    PluginCheck, Report, SbomCheck, Spec, ValidateOptions, When,
};

/// Every public spec and report type: debuggable, cloneable, and usable
/// from the parallel worker threads.
fn assert_debug_clone_send_sync<T: std::fmt::Debug + Clone + Send + Sync>() {}

/// Spec types additionally round-trip through serde.
fn assert_serde<T: serde::Serialize + for<'de> serde::Deserialize<'de>>() {}

#[test]
fn stable_api_trait_guarantees_hold() {
    assert_debug_clone_send_sync::<Spec>();
    assert_debug_clone_send_sync::<FileCheck>();
    assert_debug_clone_send_sync::<CommandCheck>();
    assert_debug_clone_send_sync::<DirectoryCheck>();
    assert_debug_clone_send_sync::<GitCheck>();
    assert_debug_clone_send_sync::<PluginCheck>();
    assert_debug_clone_send_sync::<SbomCheck>();
    assert_debug_clone_send_sync::<When>();
    assert_debug_clone_send_sync::<CommandSpec>();
    assert_debug_clone_send_sync::<CheckResult>();
    assert_debug_clone_send_sync::<Report>();
    assert_debug_clone_send_sync::<ValidateOptions>();
    assert_debug_clone_send_sync::<Cache>();
    assert_debug_clone_send_sync::<CachedResult>();

    assert_serde::<Spec>();
    assert_serde::<FileCheck>();
    assert_serde::<CommandCheck>();
    assert_serde::<DirectoryCheck>();
    assert_serde::<GitCheck>();
    assert_serde::<PluginCheck>();
    assert_serde::<SbomCheck>();
    assert_serde::<When>();
    assert_serde::<CommandSpec>();
    assert_serde::<CachedResult>();
}

#[test]
fn report_serialization_is_field_ordered_and_stable() {
    // serde emits fields in declaration order without the preserve_order
    // feature for struct types, so identical reports serialize identically
    // — the property attestations and cache keys rely on.
    let report = Report {
        pass: true,
        checks: vec![CheckResult {
            name: "file: a.txt".to_string(),
            pass: true,
            kind: "file".to_string(),
            message: "OK".to_string(),
            weight: None,
            skipped: false,
        }],
        duration_ms: 1,
    };
    let first = serde_json::to_string(&report).unwrap();
    let second = serde_json::to_string(&report).unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("{\"pass\":"));
    assert!(first.contains("\"checks\":[{\"name\":\"file: a.txt\""));
}

#[test]
fn public_module_surface_is_reachable() {
    // The documented entry points compile and are callable.
    let _ = deliver::spec_json_schema();
    assert!(Spec::from_toml("[[file]]\npath = \"a.txt\"\n").is_ok());
    assert!(Spec::from_json(r#"{"file":[{"path":"a.txt"}]}"#).is_ok());
    let options = ValidateOptions::default();
    assert!(!options.fail_fast && !options.incremental && options.cache_dir.is_none());
    assert_eq!(deliver::PROTOCOL_VERSION, 1);
    let _ = incremental::head_commit; // exported path stays valid
}
