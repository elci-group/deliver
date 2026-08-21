use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn deliver_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_deliver"))
}

#[test]
fn test_basic_file_check() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
encoding = "utf-8"
forbid_symlinks = false
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_missing_required_file() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "nonexistent.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL"));
}

#[test]
fn test_command_check() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[command]]
name = "echo test"
cmd = ["echo", "hello"]
expect_exit = 0
stdout_contains = ["hello"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_git_check() {
    let dir = TempDir::new().unwrap();

    // Initialize git repo
    Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .arg("config")
        .arg("user.email")
        .arg("test@example.com")
        .current_dir(dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .arg("config")
        .arg("user.name")
        .arg("Test User")
        .current_dir(dir.path())
        .output()
        .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[git]
no_uncommitted_changes = true
no_untracked_files = false
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--color")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_json_output() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pass\": true"));
}

#[test]
fn test_strict_mode_failure() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "nonexistent.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn test_quick_file_check() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--file")
        .arg("test.txt")
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_regex_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "fn main() { println!(\"hello\"); }").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.rs"
required = true
require_regex = ["fn main"]
forbid_regex = ["TODO", "FIXME"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_size_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
min_size_bytes = 5
max_size_bytes = 100
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_timeout_handling() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[command]]
name = "sleep command"
cmd = ["sh", "-c", "sleep 5"]
expect_exit = 0
timeout_secs = 1
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    // The command should fail due to timeout
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("timed out") || !output.status.success());
}

#[test]
fn test_json_spec_input() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let json_spec = r#"{"files":[{"path":"test.txt","required":true}]}"#;

    let output = Command::new(deliver_binary())
        .arg("--json")
        .arg(json_spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_version_flag() {
    let output = Command::new(deliver_binary())
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("deliver"));
}

#[test]
fn test_init_command() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("deliver.toml");

    let output = Command::new(deliver_binary())
        .arg("init")
        .arg("-o")
        .arg("deliver.toml")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(spec_path.exists());

    let content = fs::read_to_string(&spec_path).unwrap();
    assert!(content.contains("File checks"));
    assert!(content.contains("Command checks"));
    assert!(content.contains("Git checks"));
}

#[test]
fn test_init_existing_file() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("deliver.toml");
    fs::write(&spec_path, "existing content").unwrap();

    let output = Command::new(deliver_binary())
        .arg("init")
        .arg("-o")
        .arg("deliver.toml")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"));
}

#[test]
fn test_validate_command() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("deliver.toml");
    fs::write(
        &spec_path,
        r#"
[[file]]
path = "test.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("validate")
        .arg("deliver.toml")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("valid"));
}

#[test]
fn test_validate_invalid_toml() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("deliver.toml");
    fs::write(&spec_path, "invalid toml [[[[").unwrap();

    let output = Command::new(deliver_binary())
        .arg("validate")
        .arg("deliver.toml")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("validation failed")
            || stdout.contains("TOML error")
            || stderr.contains("validation failed")
            || stderr.contains("TOML error")
    );
}

#[test]
fn test_encoding_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
encoding = "ascii"
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[cfg(unix)]
#[test]
fn test_symlink_detection() {
    let dir = TempDir::new().unwrap();
    let real_file = dir.path().join("real.txt");
    fs::write(&real_file, "content").unwrap();

    let symlink_file = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real_file, &symlink_file).unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "link.txt"
required = true
forbid_symlinks = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("symlink"));
}

#[test]
fn test_hash_verification() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
sha256 = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.txt");
    fs::write(&test_file, "hello world").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "test.txt"
required = true
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hash mismatch"));
}

#[test]
fn test_directory_validation() {
    let dir = TempDir::new().unwrap();
    let test_dir = dir.path().join("test_dir");
    fs::create_dir(&test_dir).unwrap();
    fs::write(test_dir.join("file1.txt"), "content").unwrap();
    fs::write(test_dir.join("file2.txt"), "content").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[directory]]
path = "test_dir"
required = true
min_files = 1
max_files = 10
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_directory_empty_check() {
    let dir = TempDir::new().unwrap();
    let test_dir = dir.path().join("test_dir");
    fs::create_dir(&test_dir).unwrap();

    let output = Command::new(deliver_binary())
        .arg("--json")
        .arg("{\"directories\":[{\"path\":\"test_dir\",\"required\":true,\"forbid_empty\":true}]}")
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("empty"));
}

#[test]
fn test_license_header_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "// MIT License\nfn main() {}").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--json")
        .arg("{\"files\":[{\"path\":\"test.rs\",\"required\":true,\"require_license_header\":\"MIT\"}]}")
        .arg("--base")
        .arg(dir.path())
        .output()
    .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_license_header_missing() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "fn main() {}").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--json")
        .arg("{\"files\":[{\"path\":\"test.rs\",\"required\":true,\"require_license_header\":\"MIT\"}]}")
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
    .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("license header"));
}

#[test]
fn test_json_schema_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("data.json");
    fs::write(&test_file, r#"{"name":"test","value":42}"#).unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "data.json"
required = true
json_schema = '{"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"}},"required":["name","value"]}'
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_json_schema_failure() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("data.json");
    fs::write(&test_file, r#"{"name":"test","value":"not a number"}"#).unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "data.json"
required = true
json_schema = '{"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"}},"required":["name","value"]}'
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("schema validation failed"));
}

#[test]
fn test_toml_keys_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("config.toml");
    fs::write(
        &test_file,
        r#"[package]
name = "test"
version = "1.0.0"
"#,
    )
    .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "config.toml"
required = true
require_toml_keys = ["package.name", "package.version"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_toml_keys_missing() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("config.toml");
    fs::write(
        &test_file,
        r#"[package]
name = "test"
"#,
    )
    .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "config.toml"
required = true
require_toml_keys = ["package.name", "package.version"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("missing required keys"));
}

#[test]
fn test_json_keys_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("package.json");
    fs::write(&test_file, r#"{"name":"test","version":"1.0.0"}"#).unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "package.json"
required = true
require_json_keys = ["name", "version"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_json_keys_missing() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("package.json");
    fs::write(&test_file, r#"{"name":"test"}"#).unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "package.json"
required = true
require_json_keys = ["name", "version"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("missing required keys"));
}

#[test]
fn test_yaml_schema_validation() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("config.yaml");
    fs::write(
        &test_file,
        r#"name: test
value: 42
"#,
    )
    .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "config.yaml"
required = true
yaml_schema = '{"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"}},"required":["name","value"]}'
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_yaml_schema_failure() {
    let dir = TempDir::new().unwrap();
    let test_file = dir.path().join("config.yaml");
    fs::write(
        &test_file,
        r#"name: test
value: "not a number"
"#,
    )
    .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "config.yaml"
required = true
yaml_schema = '{"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"}},"required":["name","value"]}'
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("schema validation failed"));
}

#[test]
fn test_rust_reference_validation() {
    let dir = TempDir::new().unwrap();
    let module_file = dir.path().join("utils.rs");
    fs::write(&module_file, "pub fn helper() {}").unwrap();

    let main_file = dir.path().join("main.rs");
    fs::write(&main_file, "mod utils;").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "main.rs"
required = true
check_references = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn test_rust_reference_missing() {
    let dir = TempDir::new().unwrap();
    let main_file = dir.path().join("main.rs");
    fs::write(&main_file, "mod utils;").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "main.rs"
required = true
check_references = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("missing files"));
}

#[test]
fn test_extends_merges_parent_and_child_checks() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("base.txt"), "base").unwrap();
    fs::write(dir.path().join("leaf.txt"), "leaf").unwrap();

    fs::write(
        dir.path().join("base.toml"),
        r#"
[[file]]
path = "base.txt"
required = true
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("deliver.toml"),
        r#"
extends = "base.toml"

[[file]]
path = "leaf.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(dir.path().join("deliver.toml"))
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
    assert!(stdout.contains("base.txt"));
    assert!(stdout.contains("leaf.txt"));
}

#[test]
fn test_extends_cycle_is_rejected() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.toml"), "extends = \"b.toml\"\n").unwrap();
    fs::write(dir.path().join("b.toml"), "extends = \"a.toml\"\n").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(dir.path().join("a.toml"))
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("circular"));
}

#[test]
fn test_validate_reports_extended_spec() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("base.toml"), "[[file]]\npath = \"a.txt\"\n").unwrap();
    fs::write(
        dir.path().join("deliver.toml"),
        "extends = \"base.toml\"\n\n[[file]]\npath = \"b.txt\"\n",
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("validate")
        .arg("deliver.toml")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 file"));
}

#[test]
fn test_schema_command_prints_valid_json_schema() {
    let output = Command::new(deliver_binary())
        .arg("schema")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("schema is valid JSON");
    assert_eq!(value["title"], "deliver spec");
    assert!(value["definitions"]["file_check"].is_object());
}

#[test]
fn test_jobs_flag_runs_many_file_checks_correctly() {
    let dir = TempDir::new().unwrap();
    let mut spec_body = String::new();
    for i in 0..20 {
        let name = format!("f{i}.txt");
        fs::write(dir.path().join(&name), "x").unwrap();
        spec_body.push_str(&format!("[[file]]\npath = \"{name}\"\nrequired = true\n\n"));
    }
    let spec_path = dir.path().join("spec.toml");
    fs::write(&spec_path, &spec_body).unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec_path)
        .arg("--base")
        .arg(dir.path())
        .arg("--jobs")
        .arg("4")
        .arg("--strict")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("20 checks: 20 passed"));
}

#[test]
fn test_sarif_output_only_lists_failures() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("present.txt"), "x").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "present.txt"
required = true

[[file]]
path = "missing.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("sarif")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("SARIF is valid JSON");
    assert_eq!(value["version"], "2.1.0");
    let results = value["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["message"]["text"]
        .as_str()
        .unwrap()
        .contains("missing.txt"));
}

#[test]
fn test_junit_output_lists_every_check() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("present.txt"), "x").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "present.txt"
required = true

[[file]]
path = "missing.txt"
required = true
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("junit")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("<testsuite name=\"deliver\" tests=\"2\" failures=\"1\""));
    assert!(stdout.contains("<failure"));
    assert!(stdout.contains("present.txt"));
    assert!(stdout.contains("missing.txt"));
}
