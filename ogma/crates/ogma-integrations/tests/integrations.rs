use std::fs;
use std::path::PathBuf;

use ogma_integrations::{
    ci_workflow_yaml, install_pre_commit_hook, pre_commit_hook_contents, remove_pre_commit_hook,
};

fn fixture_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "ogma-integrations-test-{}-{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".git/hooks")).unwrap();
    root
}

#[test]
fn hook_contents_have_shebang_markers_and_check() {
    let contents = pre_commit_hook_contents();
    assert!(contents.starts_with("#!/bin/sh"));
    assert!(contents.contains("# ogma-begin"));
    assert!(contents.contains("# ogma-end"));
    assert!(contents.contains("ogma check"));
}

#[test]
fn ci_yaml_has_name_triggers_install_and_check() {
    let yaml = ci_workflow_yaml();
    assert!(yaml.contains("name: ogma"));
    assert!(yaml.contains("push:"));
    assert!(yaml.contains("pull_request:"));
    assert!(yaml.contains("actions/checkout@v4"));
    assert!(yaml.contains("rust-toolchain"));
    assert!(yaml.contains("cargo install --path crates/ogma-cli --locked"));
    assert!(yaml.contains("ogma check"));
}

#[test]
fn install_create_executable_hook_and_is_idempotent() {
    let root = fixture_root("install");
    let path = install_pre_commit_hook(&root).unwrap();
    assert_eq!(path, root.join(".git/hooks/pre-commit"));

    let contents = fs::read_to_string(&path).unwrap();
    assert_eq!(contents, pre_commit_hook_contents());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    // Second install is a no-op.
    install_pre_commit_hook(&root).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), contents);

    // Remove deletes the whole file (hook was solely the ogma block).
    assert!(remove_pre_commit_hook(&root).unwrap());
    assert!(!path.exists());
    // Second remove is a no-op.
    assert!(!remove_pre_commit_hook(&root).unwrap());

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn install_appends_to_existing_hook_and_remove_keeps_rest() {
    let root = fixture_root("append");
    let path = root.join(".git/hooks/pre-commit");
    let original = "#!/bin/sh\necho lint\n";
    fs::write(&path, original).unwrap();

    install_pre_commit_hook(&root).unwrap();
    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("echo lint"));
    assert!(contents.contains("# ogma-begin"));
    assert!(contents.contains("ogma check"));
    // Original content not duplicated.
    assert_eq!(contents.matches("echo lint").count(), 1);

    // Re-install does not append twice.
    install_pre_commit_hook(&root).unwrap();
    let again = fs::read_to_string(&path).unwrap();
    assert_eq!(again.matches("# ogma-begin").count(), 1);

    // Remove strips only the ogma block.
    assert!(remove_pre_commit_hook(&root).unwrap());
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("echo lint"));
    assert!(!after.contains("# ogma-begin"));
    assert!(path.exists());

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn install_errors_without_git_dir() {
    let root = std::env::temp_dir().join(format!(
        "ogma-integrations-test-nogit-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let err = install_pre_commit_hook(&root).unwrap_err();
    assert!(err.0.contains(".git"));
    fs::remove_dir_all(&root).unwrap();
}
