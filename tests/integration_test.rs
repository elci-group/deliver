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
name = "rustc version"
cmd = ["rustc", "--version"]
expect_exit = 0
stdout_contains = ["rustc"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--allow-commands")
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
}

#[test]
fn failed_command_expectations_do_not_echo_expected_values() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    let sentinel = "secret-sentinel-expected-output";
    for field in ["stdout_contains", "stderr_contains"] {
        fs::write(
            &spec,
            format!(
                "[[command]]\nname = 'output check'\ncmd = ['rustc', '--version']\n{field} = ['{sentinel}']\n"
            ),
        )
        .unwrap();

        let output = Command::new(deliver_binary())
            .arg("--spec")
            .arg(&spec)
            .arg("--allow-commands")
            .arg("--base")
            .arg(dir.path())
            .arg("--format")
            .arg("json")
            .arg("--strict")
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("{field}[0]")));
        assert!(!stdout.contains(sentinel));
    }
}

#[cfg(unix)]
#[test]
fn command_check_requires_explicit_trust_without_running_it() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "[[command]]\nname = 'marker'\ncmd = ['sh', '-c', 'touch marker.txt']\n",
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--allow-commands"));
    assert!(!dir.path().join("marker.txt").exists());
}

#[test]
fn misspelled_spec_field_is_rejected() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = 'x.txt'\nrequried = false\n").unwrap();
    let output = Command::new(deliver_binary())
        .arg("validate")
        .arg(&spec)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requried"));
}

#[test]
fn external_json_schema_reference_is_rejected_without_fetching() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("data.json"), "{}").unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "[[file]]\npath = 'data.json'\njson_schema = '''{\"$ref\":\"http://127.0.0.1:9/external.json\"}'''\n",
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
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("resolve-http"));
}

#[test]
fn file_check_rejects_parent_escape() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "[[file]]\npath = '../outside.txt'\nrequired = false\n",
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
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("path must stay inside --base"));
}

#[cfg(unix)]
#[test]
fn command_output_over_limit_fails() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "[[command]]\nname = 'noisy'\ncmd = ['sh', '-c', 'head -c 1048577 /dev/zero']\n",
    )
    .unwrap();
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--allow-commands")
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("output exceeded"));
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

#[cfg(unix)]
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
        .arg("--allow-commands")
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("--allow-commands"));

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

#[test]
fn test_weighted_checks_appear_in_json_output() {
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
weight = 1.0

[[command]]
name = "rustc version"
cmd = ["rustc", "--version"]
expect_exit = 0
weight = 4.0
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--allow-commands")
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("report is valid JSON");
    let checks = report["checks"].as_array().unwrap();
    assert_eq!(checks[0]["weight"], 1.0);
    assert_eq!(checks[1]["weight"], 4.0);
    assert!(stdout.contains("\"weight\": 4.0"));
}

#[test]
fn test_unweighted_checks_omit_weight_in_json_output() {
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
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("report is valid JSON");
    for check in report["checks"].as_array().unwrap() {
        assert!(check.get("weight").is_none());
    }
    assert!(!stdout.contains("\"weight\""));
}

#[test]
fn test_text_summary_reports_weighted_completion() {
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
weight = 1.0

[[command]]
name = "rustc version"
cmd = ["rustc", "--version"]
expect_exit = 0
weight = 4.0
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--allow-commands")
        .arg("--base")
        .arg(dir.path())
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("complete by weight"));
    assert!(stdout.contains("100% complete by weight"));
}

#[test]
fn test_text_summary_omits_weighted_completion_without_weights() {
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
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("complete by weight"));
}

#[test]
fn test_when_clause_skips_instead_of_failing() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "Cargo.lock"
required = true
when = { file_exists = "Cargo.toml" }
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
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success(), "a skipped check must not fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS"));
    assert!(stdout.contains("1 checks: 0 passed, 0 failed, 1 skipped"));
    assert!(stdout.contains("skipped: when.file_exists"));
}

#[test]
fn test_when_clause_runs_when_condition_holds() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "Cargo.lock"
required = true
when = { file_exists = "Cargo.toml" }
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL"));
    assert!(stdout.contains("1 checks: 0 passed, 1 failed"));
}

#[test]
fn test_when_clause_json_report_carries_skipped_flag() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "Cargo.lock"
required = true
when = { os = "nonexistent-os" }
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
    assert!(stdout.contains("\"skipped\": true"));
    assert!(stdout.contains("\"pass\": true"));
}

#[test]
fn test_when_clause_junit_marks_skipped() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "Cargo.lock"
required = true
when = { file_exists = "Cargo.toml" }
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
    assert!(stdout.contains("<skipped/>"));
    assert!(!stdout.contains("<failure"));
}

#[test]
fn test_when_clause_unknown_field_rejected() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "Cargo.lock"
when = { file_existsx = "Cargo.toml" }
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

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_fail_fast_stops_after_first_failure() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "has forbidden token").unwrap();
    fs::write(dir.path().join("b.txt"), "clean").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "a.txt"
forbid_regex = ["forbidden"]

[[file]]
path = "b.txt"
forbid_regex = ["impossible-match"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--fail-fast")
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL"));
    assert!(stdout.contains("fail-fast: remaining checks were not run"));
}

#[test]
fn test_cache_dir_replays_and_detects_content_changes() {
    let dir = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "good content").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "a.txt"
require_regex = ["good"]
"#,
    )
    .unwrap();

    let run = || {
        Command::new(deliver_binary())
            .arg("--spec")
            .arg(&spec)
            .arg("--base")
            .arg(dir.path())
            .arg("--cache-dir")
            .arg(cache_dir.path())
            .arg("--strict")
            .arg("--format")
            .arg("json")
            .arg("--progress")
            .arg("never")
            .output()
            .unwrap()
    };

    let output = run();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pass\": true"));
    assert!(!stdout.contains("(cached)"));

    // Identical content replays from the cache.
    let replayed = run();
    let stdout = String::from_utf8_lossy(&replayed.stdout);
    assert!(
        stdout.contains("(cached)"),
        "second run must replay: {stdout}"
    );

    // A cache file is written under the cache dir, one per base.
    let entries: Vec<_> = fs::read_dir(cache_dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);

    // Changed content must not replay the cached pass.
    fs::write(dir.path().join("a.txt"), "bad content").unwrap();
    let output = run();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pass\": false"));
    assert!(!stdout.contains("(cached)"));
}

#[test]
fn test_incremental_requires_git_repository() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "a").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = \"a.txt\"\n").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--incremental")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--incremental") || stderr.contains("git"));
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=deliver-test",
            "-c",
            "user.email=test@example.com",
        ])
        .args(args)
        .status()
        .expect("git binary available for incremental integration test");
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn test_incremental_replays_unchanged_files() {
    let dir = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "good content").unwrap();
    git_in(dir.path(), &["init", "-b", "main"]);
    git_in(dir.path(), &["add", "a.txt"]);
    git_in(dir.path(), &["commit", "-m", "add a.txt"]);

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "a.txt"
require_regex = ["good"]
"#,
    )
    .unwrap();

    let run = || {
        Command::new(deliver_binary())
            .arg("--spec")
            .arg(&spec)
            .arg("--base")
            .arg(dir.path())
            .arg("--incremental")
            .arg("--cache-dir")
            .arg(cache_dir.path())
            .arg("--format")
            .arg("json")
            .arg("--progress")
            .arg("never")
            .output()
            .unwrap()
    };

    let output = run();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("(cached)"));

    // Unchanged since HEAD: the result replays without re-validation.
    let replayed = run();
    let stdout = String::from_utf8_lossy(&replayed.stdout);
    assert!(
        stdout.contains("(cached)"),
        "unchanged file must replay: {stdout}"
    );
    assert!(stdout.contains("\"pass\": true"));
}

#[test]
fn test_ndjson_emits_check_lines_then_summary() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("good.txt"), "hello").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "good.txt"
require_regex = ["hello"]

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
        .arg("ndjson")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "two check lines then one summary line");

    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["type"], "check");
    assert_eq!(first["name"], "file: good.txt");
    assert_eq!(first["pass"], true);

    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["type"], "check");
    assert_eq!(second["name"], "file: missing.txt");
    assert_eq!(second["pass"], false);

    let summary: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(summary["type"], "summary");
    assert_eq!(summary["pass"], false);
    assert_eq!(summary["checks"], 2);
    assert_eq!(summary["passed"], 1);
    assert_eq!(summary["failed"], 1);
    assert_eq!(summary["skipped"], 0);
}

#[test]
fn test_ndjson_strict_exit_code_still_applies() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = \"missing.txt\"\nrequired = true\n").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("ndjson")
        .arg("--strict")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary: serde_json::Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(summary["type"], "summary");
    assert_eq!(summary["pass"], false);
}

#[test]
fn test_github_annotations_on_failure() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "missing.txt"
required = true

[[command]]
name = "passing cmd"
cmd = ["echo", "ok"]
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--allow-commands")
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("github")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "one error annotation plus one notice");
    assert!(
        lines[0].starts_with("::error title=file%3A missing.txt,file=missing.txt::"),
        "file checks must carry the file property: {}",
        lines[0]
    );
    assert!(lines[0].contains("missing.txt"));
    assert!(
        !stdout.contains("passing cmd"),
        "passing checks must not be annotated"
    );
    assert!(lines[1].starts_with("::notice title=deliver::2 checks: 1 passed, 1 failed"));
}

#[test]
fn test_github_annotations_escape_properties() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[file]]
path = "a,b:c%.txt"
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
        .arg("github")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("title=file%3A a%2Cb%3Ac%25.txt,file=a%2Cb%3Ac%25.txt::"),
        "property values must escape %, :, and ,: {stdout}"
    );
}

#[test]
fn test_github_only_notice_when_passing() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = \"a.txt\"\n").unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--format")
        .arg("github")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("::error"));
    assert!(stdout.starts_with("::notice title=deliver::1 checks: 1 passed, 0 failed"));
}

#[test]
fn test_sbom_check_passes_fails_and_filters() {
    let spec_for = |dir: &TempDir, required: bool| {
        let spec = dir.path().join("spec.toml");
        fs::write(
            &spec,
            format!(
                "[[file]]\npath = \"a.txt\"\n\n[sbom]\nrequired = {}\n",
                required
            ),
        )
        .unwrap();
        spec
    };

    // CycloneDX at the standard bom.json location passes.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "a").unwrap();
    fs::write(
        dir.path().join("bom.json"),
        r#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}"#,
    )
    .unwrap();
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(spec_for(&dir, true))
        .arg("--base")
        .arg(dir.path())
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CycloneDX 1.5"));
    assert!(stdout.contains("sbom]"));

    // Missing SBOM fails when required, passes when optional.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "a").unwrap();
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(spec_for(&dir, true))
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no SBOM found"));

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(spec_for(&dir, false))
        .arg("--base")
        .arg(dir.path())
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_forbid_secrets_fails_with_pattern_name_not_content() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("creds.txt"),
        "deploy key: ghp_abcdef0123456789abcdef0123456789abcdef01\n",
    )
    .unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        "[[file]]\npath = \"creds.txt\"\nforbid_secrets = true\n",
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("github-token"));
    assert!(stdout.contains("line 1"));
    assert!(
        !stdout.contains("ghp_abcdef0123456789"),
        "the secret value itself must never appear in the report: {stdout}"
    );
}

#[test]
fn test_attest_unsigned_bundle_lifecycle() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = \"a.txt\"\n").unwrap();
    let bundle = dir.path().join("attest.json");

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--attest")
        .arg(&bundle)
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(bundle.exists(), "a passing run must write the bundle");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("attestation written"));

    let verify = Command::new(deliver_binary())
        .arg("attest")
        .arg("verify")
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(verify.status.success());
    let stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(stdout.contains("integrity verified"));

    // Tampering with the payload must break verification.
    let text = fs::read_to_string(&bundle).unwrap();
    let tampered = text.replace("\"checks\": 1", "\"checks\": 99");
    fs::write(&bundle, tampered).unwrap();
    let verify = Command::new(deliver_binary())
        .arg("attest")
        .arg("verify")
        .arg(&bundle)
        .output()
        .unwrap();
    assert_eq!(verify.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&verify.stderr);
    assert!(stderr.contains("hash mismatch") || stderr.contains("modified"));
}

#[test]
fn test_attest_signed_roundtrip_and_failure_skip() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let spec = dir.path().join("spec.toml");
    fs::write(&spec, "[[file]]\npath = \"a.txt\"\n").unwrap();

    let prefix = dir.path().join("deliver");
    let keygen = Command::new(deliver_binary())
        .arg("keygen")
        .arg("--out")
        .arg(&prefix)
        .output()
        .unwrap();
    assert!(
        keygen.status.success(),
        "{}",
        String::from_utf8_lossy(&keygen.stderr)
    );
    let key = dir.path().join("deliver.key");
    let public = dir.path().join("deliver.pub");
    assert!(key.exists() && public.exists());

    let bundle = dir.path().join("signed.json");
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--attest")
        .arg(&bundle)
        .arg("--attest-key")
        .arg(&key)
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(bundle.exists());

    let verify = Command::new(deliver_binary())
        .arg("attest")
        .arg("verify")
        .arg(&bundle)
        .arg("--pub")
        .arg(&public)
        .output()
        .unwrap();
    assert!(verify.status.success());
    let stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(stdout.contains("signature verified against supplied key"));

    // A different key must not authenticate the signature.
    let other_prefix = dir.path().join("other");
    assert!(Command::new(deliver_binary())
        .arg("keygen")
        .arg("--out")
        .arg(&other_prefix)
        .output()
        .unwrap()
        .status
        .success());
    let verify = Command::new(deliver_binary())
        .arg("attest")
        .arg("verify")
        .arg(&bundle)
        .arg("--pub")
        .arg(dir.path().join("other.pub"))
        .output()
        .unwrap();
    assert_eq!(verify.status.code(), Some(1));

    // Failing runs must not produce a bundle.
    let failed_spec = dir.path().join("bad.toml");
    fs::write(
        &failed_spec,
        "[[file]]\npath = \"missing.txt\"\nrequired = true\n",
    )
    .unwrap();
    let failed_bundle = dir.path().join("never.json");
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&failed_spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--attest")
        .arg(&failed_bundle)
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!failed_bundle.exists(), "failed runs never attest");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("attestation skipped"));
}

#[test]
fn test_plugin_check_requires_explicit_trust() {
    let dir = TempDir::new().unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[plugin]]
name = "org gate"
cmd = ["sh", "-c", "echo '{\"pass\": true}'"]
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

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("command or plugin checks"));
    assert!(stderr.contains("--allow-commands"));
}

#[test]
fn test_plugin_check_end_to_end() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();

    let spec = dir.path().join("spec.toml");
    fs::write(
        &spec,
        r#"
[[plugin]]
name = "passing gate"
cmd = ["sh", "-c", "cat > /dev/null; echo '{\"pass\":true,\"message\":\"all good\"}'"]
weight = 3.0

[[plugin]]
name = "failing gate"
cmd = ["sh", "-c", "cat > /dev/null; echo '{\"pass\":false,\"message\":\"spec drift detected\"}'"]

[[plugin]]
name = "skipped gate"
cmd = ["sh", "-c", "echo '{\"pass\":false}'"]
when = { os = "nonexistent-os" }
"#,
    )
    .unwrap();

    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(&spec)
        .arg("--base")
        .arg(dir.path())
        .arg("--allow-commands")
        .arg("--strict")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("3 checks: 1 passed, 1 failed, 1 skipped"));
    assert!(stdout.contains("[plugin]"));
    assert!(stdout.contains("all good"));
    assert!(stdout.contains("spec drift detected"));
    assert!(
        stdout.contains("75% complete by weight"),
        "plugin weights must count: {stdout}"
    );
}

#[test]
fn test_example_plugin_spec_passes() {
    // Keeps the documented example honest: run it from the repository.
    let example_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/plugin");
    let output = Command::new(deliver_binary())
        .arg("--spec")
        .arg(example_dir.join("spec.toml"))
        .arg("--base")
        .arg(&example_dir)
        .arg("--allow-commands")
        .arg("--color")
        .arg("never")
        .arg("--progress")
        .arg("never")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[plugin]"));
    assert!(stdout.contains("docs completeness"));
}
