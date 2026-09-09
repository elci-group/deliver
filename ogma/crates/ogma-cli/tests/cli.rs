use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn ogma() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ogma"))
}

static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn fixture(name: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "ogma-cli-test-{}-{}-{id}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    write_fixture(&root, true);
    root
}

fn write_fixture(root: &Path, complete_fr: bool) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("locales")).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{
  "name": "fixture-app",
  "dependencies": { "react": "^18.0.0", "i18next": "^23.0.0" }
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/app.js"),
        r#"import { useTranslation } from 'react-i18next';

export function App() {
  const { t } = useTranslation();
  setText('Hard coded');
  return <button>{t('ok')}</button>;
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("locales/en.json"),
        "{ \"ok\": \"OK\", \"cancel\": \"Cancel\" }\n",
    )
    .unwrap();
    let fr = if complete_fr {
        "{ \"ok\": \"D'accord\", \"cancel\": \"Annuler\" }\n"
    } else {
        "{ \"ok\": \"D'accord\" }\n"
    };
    fs::write(root.join("locales/fr.json"), fr).unwrap();
}

fn write_passing_policy(root: &Path) {
    // The default policy disallows cross-language fallback, so a genuinely
    // multi-language app (fr + en baseline) must permit it explicitly.
    fs::write(
        root.join("ogma.toml"),
        "[application]\nbaseline = \"en\"\n\n[requirements]\nallow_fallback = true\n",
    )
    .unwrap();
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    ogma().args(args).current_dir(root).output().unwrap()
}

#[test]
fn check_passes_on_complete_fixture() {
    let root = fixture("pass");
    write_passing_policy(&root);
    let out = run(&root, &["check"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ogma: PASS");
    // last-audit.json is always written.
    let audit = fs::read_to_string(root.join(".ogma/last-audit.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(value["overall"], "PASS");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_fails_when_translation_missing() {
    let root = fixture("fail");
    write_fixture(&root, false);
    let out = run(&root, &["check"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("OGMA"), "stdout:\n{stdout}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn audit_json_is_parseable_and_writes_last_audit() {
    let root = fixture("audit");
    let out = run(&root, &["audit", "--format", "json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(value.get("locales").is_some());
    assert!(value.get("overall").is_some());
    let on_disk: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join(".ogma/last-audit.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk, value);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn audit_sarif_is_parseable() {
    let root = fixture("sarif");
    let out = run(&root, &["audit", "--format", "sarif"]);
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(value["version"], "2.1.0");
    assert_eq!(value["runs"][0]["tool"]["driver"]["name"], "ogma");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn init_writes_policy_and_refuses_second_run() {
    let root = fixture("init");
    let out = run(&root, &["init"]);
    assert!(out.status.success());
    let policy = fs::read_to_string(root.join("ogma.toml")).unwrap();
    assert!(policy.contains("[requirements]"));

    let again = run(&root, &["init"]);
    assert_eq!(again.status.code(), Some(2));
    let forced = run(&root, &["init", "--force"]);
    assert!(forced.status.success());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn explain_prints_registered_code() {
    let root = fixture("explain");
    let out = run(&root, &["explain", "OGMA-I18N-001"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("OGMA-I18N-001"));
    assert!(stdout.contains("What it means:"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn explain_without_audit_exits_2() {
    let root = fixture("explain-missing");
    let out = run(&root, &["explain", "src/app.js"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("run `ogma audit` first"), "stderr:\n{stderr}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn bad_policy_file_exits_3() {
    let root = fixture("bad-policy");
    fs::write(
        root.join("ogma.toml"),
        "[requirements]\ntranslation_coverage = 1.5\n",
    )
    .unwrap();
    let out = run(&root, &["check"]);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("translation_coverage"), "stderr:\n{stderr}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn explicit_missing_policy_file_exits_3() {
    let root = fixture("missing-policy");
    let out = run(&root, &["check", "--policy", "nope.toml"]);
    assert_eq!(out.status.code(), Some(3));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn version_prints_name_and_semver() {
    let root = fixture("version");
    let out = run(&root, &["--version"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ogma 0.1.0");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn detect_locales_and_strings_report_without_enforcing() {
    let root = fixture("reporting");
    let out = run(&root, &["detect"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fixture-app"), "stdout:\n{stdout}");

    let out = run(&root, &["locales"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Locale"), "stdout:\n{stdout}");
    assert!(stdout.contains("en"));

    let out = run(&root, &["strings"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Hard-coded"), "stdout:\n{stdout}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn policy_subcommands_work() {
    let root = fixture("policy");
    let out = run(&root, &["policy", "show"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("defaults"), "stdout:\n{stdout}");

    let out = run(&root, &["policy", "validate"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "policy OK");

    let out = run(&root, &["policy", "init"]);
    assert!(out.status.success());
    assert!(root.join("ogma.toml").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn ci_template_and_check_json_format() {
    let root = fixture("ci");
    write_passing_policy(&root);
    let out = run(&root, &["ci-template"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("name: ogma"));
    assert!(stdout.contains("ogma check"));

    let out = run(&root, &["check", "--format", "json"]);
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(value["overall"], "PASS");
    fs::remove_dir_all(&root).unwrap();
}
