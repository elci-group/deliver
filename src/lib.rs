mod command;
mod directory_check;
mod file_check;
mod git_check;
mod glob;
mod schema;

pub use file_check::validate_file;
pub use glob::expand_paths;
pub use schema::spec_json_schema;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    /// Path to a parent spec to inherit from, resolved relative to the
    /// directory of the spec file that declares it. Only honored by
    /// [`Spec::load_file`]; ignored by [`Spec::from_toml`]/[`Spec::from_json`].
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default, alias = "file")]
    pub files: Vec<FileCheck>,
    #[serde(default, alias = "command")]
    pub commands: Vec<CommandCheck>,
    #[serde(default)]
    pub git: GitCheck,
    #[serde(default, alias = "directory")]
    pub directories: Vec<DirectoryCheck>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileCheck {
    pub path: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub max_size_bytes: Option<u64>,
    #[serde(default)]
    pub min_size_bytes: Option<u64>,
    #[serde(default)]
    pub forbid_regex: Vec<String>,
    #[serde(default)]
    pub require_regex: Vec<String>,
    #[serde(default)]
    pub require_line_count: Option<usize>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub forbid_symlinks: bool,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub blake3: Option<String>,
    #[serde(default)]
    pub require_license_header: Option<String>,
    #[serde(default)]
    pub json_schema: Option<String>,
    #[serde(default)]
    pub require_toml_keys: Vec<String>,
    #[serde(default)]
    pub require_json_keys: Vec<String>,
    #[serde(default)]
    pub yaml_schema: Option<String>,
    #[serde(default)]
    pub check_references: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandCheck {
    pub name: String,
    pub cmd: CommandSpec,
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_expect_exit")]
    pub expect_exit: i32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub stdout_contains: Vec<String>,
    #[serde(default)]
    pub stderr_contains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CommandSpec {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GitCheck {
    #[serde(default)]
    pub no_uncommitted_changes: bool,
    #[serde(default)]
    pub no_untracked_files: bool,
    /// Fail unless the current branch name matches this regex.
    #[serde(default)]
    pub require_branch_regex: Option<String>,
    /// Fail unless the last commit's message matches this regex.
    #[serde(default)]
    pub require_commit_message_regex: Option<String>,
}

impl GitCheck {
    fn any_enabled(&self) -> bool {
        self.no_uncommitted_changes
            || self.no_untracked_files
            || self.require_branch_regex.is_some()
            || self.require_commit_message_regex.is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DirectoryCheck {
    pub path: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub min_files: Option<usize>,
    #[serde(default)]
    pub max_files: Option<usize>,
    #[serde(default)]
    pub forbid_empty: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub pass: bool,
    pub checks: Vec<CheckResult>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub pass: bool,
    pub kind: String,
    pub message: String,
}

fn default_true() -> bool {
    true
}

fn default_cwd() -> PathBuf {
    PathBuf::from(".")
}

fn default_expect_exit() -> i32 {
    0
}

fn default_timeout() -> u64 {
    300
}

impl Spec {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Load a spec from `path` (TOML or JSON, chosen by file extension),
    /// following its `extends` chain and merging parent specs into this one.
    /// Checks accumulate (parent checks run first); Git-cleanliness flags
    /// combine with OR, so a child spec can only add requirements on top of
    /// its parent, never relax one. Detects circular `extends` chains.
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let mut chain = Vec::new();
        Self::load_file_inner(path, &mut chain)
    }

    fn load_file_inner(path: &Path, chain: &mut Vec<PathBuf>) -> Result<Self, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("failed to resolve spec path {}: {}", path.display(), e))?;
        if chain.contains(&canonical) {
            return Err(format!(
                "circular `extends` chain detected at {}",
                path.display()
            ));
        }
        chain.push(canonical);

        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read spec {}: {}", path.display(), e))?;
        let is_json = path.extension().and_then(|e| e.to_str()) == Some("json");
        let mut spec = if is_json {
            Self::from_json(&text)
                .map_err(|e| format!("failed to parse JSON spec {}: {}", path.display(), e))?
        } else {
            Self::from_toml(&text)
                .map_err(|e| format!("failed to parse TOML spec {}: {}", path.display(), e))?
        };

        let result = match spec.extends.take() {
            Some(parent_rel) => {
                let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
                let parent_path = parent_dir.join(&parent_rel);
                let parent_spec = Self::load_file_inner(&parent_path, chain)?;
                Self::merge(parent_spec, spec)
            }
            None => spec,
        };

        chain.pop();
        Ok(result)
    }

    fn merge(parent: Spec, child: Spec) -> Spec {
        let mut files = parent.files;
        files.extend(child.files);
        let mut commands = parent.commands;
        commands.extend(child.commands);
        let mut directories = parent.directories;
        directories.extend(child.directories);
        let mut metadata = parent.metadata;
        metadata.extend(child.metadata);
        let git = GitCheck {
            no_uncommitted_changes: parent.git.no_uncommitted_changes
                || child.git.no_uncommitted_changes,
            no_untracked_files: parent.git.no_untracked_files || child.git.no_untracked_files,
            require_branch_regex: child
                .git
                .require_branch_regex
                .or(parent.git.require_branch_regex),
            require_commit_message_regex: child
                .git
                .require_commit_message_regex
                .or(parent.git.require_commit_message_regex),
        };

        Spec {
            extends: None,
            files,
            commands,
            git,
            directories,
            metadata,
        }
    }

    /// Run every check sequentially, in spec order. Equivalent to
    /// `validate_with_jobs(base, 1)`.
    pub fn validate(&self, base: &Path) -> Report {
        self.validate_with_jobs(base, 1)
    }

    /// Run file, command, and directory checks using up to `jobs` worker
    /// threads (clamped to at least 1), then run git checks sequentially.
    /// The report's check order always matches spec order (files, then
    /// commands, then directories, then git) regardless of `jobs`, so
    /// output is deterministic even when execution is not.
    pub fn validate_with_jobs(&self, base: &Path, jobs: usize) -> Report {
        let start = Instant::now();
        let jobs = jobs.max(1);

        enum Task<'a> {
            File(&'a FileCheck),
            Command(&'a CommandCheck),
            Directory(&'a DirectoryCheck),
        }

        let mut tasks: Vec<Task> =
            Vec::with_capacity(self.files.len() + self.commands.len() + self.directories.len());
        tasks.extend(self.files.iter().map(Task::File));
        tasks.extend(self.commands.iter().map(Task::Command));
        tasks.extend(self.directories.iter().map(Task::Directory));

        let slots: Vec<std::sync::Mutex<Option<CheckResult>>> = (0..tasks.len())
            .map(|_| std::sync::Mutex::new(None))
            .collect();
        let next = std::sync::atomic::AtomicUsize::new(0);

        if !tasks.is_empty() {
            std::thread::scope(|scope| {
                for _ in 0..jobs.min(tasks.len()) {
                    scope.spawn(|| loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= tasks.len() {
                            break;
                        }
                        let result = match tasks[i] {
                            Task::File(file) => crate::file_check::validate_file(file, base),
                            Task::Command(command) => {
                                crate::command::validate_command(command, base)
                            }
                            Task::Directory(directory) => {
                                crate::directory_check::validate_directory(directory, base)
                            }
                        };
                        *slots[i].lock().unwrap() = Some(result);
                    });
                }
            });
        }

        let mut checks: Vec<CheckResult> = slots
            .into_iter()
            .map(|slot| slot.into_inner().unwrap().expect("every task slot filled"))
            .collect();

        if self.git.any_enabled() {
            checks.extend(crate::git_check::validate_git(&self.git, base));
        }

        let pass = checks.iter().all(|check| check.pass);

        Report {
            pass,
            checks,
            duration_ms: start.elapsed().as_millis(),
        }
    }
}

/// Quick file-only check for a single path; used by CLI shorthand.
pub fn quick_check_files(paths: &[PathBuf], base: &Path) -> Vec<CheckResult> {
    paths
        .iter()
        .map(|p| {
            let relative = p.strip_prefix(base).unwrap_or(p);
            let check = FileCheck {
                path: relative.to_string_lossy().to_string(),
                required: true,
                max_size_bytes: None,
                min_size_bytes: None,
                forbid_regex: Vec::new(),
                require_regex: Vec::new(),
                require_line_count: None,
                encoding: None,
                forbid_symlinks: false,
                sha256: None,
                blake3: None,
                require_license_header: None,
                json_schema: None,
                require_toml_keys: Vec::new(),
                require_json_keys: Vec::new(),
                yaml_schema: None,
                check_references: false,
            };
            crate::file_check::validate_file(&check, base)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use tempfile::TempDir;

    fn base_file_check(path: &str) -> FileCheck {
        FileCheck {
            path: path.to_string(),
            required: true,
            max_size_bytes: None,
            min_size_bytes: None,
            forbid_regex: Vec::new(),
            require_regex: Vec::new(),
            require_line_count: None,
            encoding: None,
            forbid_symlinks: false,
            sha256: None,
            blake3: None,
            require_license_header: None,
            json_schema: None,
            require_toml_keys: Vec::new(),
            require_json_keys: Vec::new(),
            yaml_schema: None,
            check_references: false,
        }
    }

    fn base_spec(files: Vec<FileCheck>) -> Spec {
        Spec {
            extends: None,
            files,
            commands: Vec::new(),
            git: GitCheck::default(),
            directories: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn validates_required_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hello.txt");
        fs::write(&path, "hello world").unwrap();

        let spec = base_spec(vec![FileCheck {
            require_regex: vec!["hello".to_string()],
            ..base_file_check("hello.txt")
        }]);

        let report = spec.validate(dir.path());
        assert!(report.pass);
        assert_eq!(report.checks.len(), 1);
        assert!(report.checks[0].pass);
    }

    #[test]
    fn catches_forbidden_regex() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("bad.rs"),
            "fn main() { panic!(\"oh no\"); }",
        )
        .unwrap();

        let spec = base_spec(vec![FileCheck {
            forbid_regex: vec!["panic!".to_string()],
            ..base_file_check("bad.rs")
        }]);

        let report = spec.validate(dir.path());
        assert!(!report.pass);
    }

    #[test]
    fn quick_check_files_works() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        let checks = quick_check_files(&[dir.path().join("a.txt")], dir.path());
        assert_eq!(checks.len(), 1);
        assert!(checks[0].pass);
    }

    #[test]
    fn glob_expansion() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("x.rs"), "x").unwrap();
        fs::write(dir.path().join("y.rs"), "y").unwrap();
        fs::write(dir.path().join("z.txt"), "z").unwrap();
        let paths = expand_paths("*.rs", dir.path());
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn extends_merges_checks_and_ors_git_flags() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("parent.txt"), "parent").unwrap();
        fs::write(dir.path().join("child.txt"), "child").unwrap();

        fs::write(
            dir.path().join("base.toml"),
            r#"
[[file]]
path = "parent.txt"
required = true

[git]
no_uncommitted_changes = true
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("child.toml"),
            r#"
extends = "base.toml"

[[file]]
path = "child.txt"
required = true

[git]
no_untracked_files = true
"#,
        )
        .unwrap();

        let spec = Spec::load_file(&dir.path().join("child.toml")).unwrap();
        assert_eq!(spec.files.len(), 2);
        assert_eq!(spec.files[0].path, "parent.txt");
        assert_eq!(spec.files[1].path, "child.txt");
        assert!(spec.git.no_uncommitted_changes);
        assert!(spec.git.no_untracked_files);
    }

    #[test]
    fn extends_detects_cycles() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.toml"), "extends = \"b.toml\"\n").unwrap();
        fs::write(dir.path().join("b.toml"), "extends = \"a.toml\"\n").unwrap();

        let result = Spec::load_file(&dir.path().join("a.toml"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("circular"));
    }

    #[test]
    fn validate_with_jobs_matches_sequential_result_and_order() {
        let dir = TempDir::new().unwrap();
        for i in 0..12 {
            fs::write(dir.path().join(format!("f{i}.txt")), "content").unwrap();
        }
        let files: Vec<FileCheck> = (0..12)
            .map(|i| base_file_check(&format!("f{i}.txt")))
            .collect();
        let spec = base_spec(files);

        let sequential = spec.validate_with_jobs(dir.path(), 1);
        let parallel = spec.validate_with_jobs(dir.path(), 8);

        assert_eq!(sequential.pass, parallel.pass);
        assert_eq!(sequential.checks.len(), parallel.checks.len());
        let seq_names: Vec<&str> = sequential.checks.iter().map(|c| c.name.as_str()).collect();
        let par_names: Vec<&str> = parallel.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(seq_names, par_names, "check order must not depend on jobs");
    }

    #[test]
    fn validate_with_jobs_zero_is_treated_as_one() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        let spec = base_spec(vec![base_file_check("a.txt")]);
        let report = spec.validate_with_jobs(dir.path(), 0);
        assert!(report.pass);
        assert_eq!(report.checks.len(), 1);
    }

    // Property-based tests
    proptest! {
        #[test]
        fn file_size_validation_property(min_size in 0u64..1000, max_size in 1000u64..10000) {
            let dir = TempDir::new().unwrap();
            let test_size = (min_size + max_size) / 2;
            let path = dir.path().join("test.txt");
            let content = "x".repeat(test_size as usize);
            fs::write(&path, content).unwrap();

            let spec = base_spec(vec![FileCheck {
                max_size_bytes: Some(max_size),
                min_size_bytes: Some(min_size),
                ..base_file_check("test.txt")
            }]);

            let report = spec.validate(dir.path());
            prop_assert!(report.pass);
        }

        #[test]
        fn optional_files_pass_when_missing(path in "[a-zA-Z0-9_]+") {
            let dir = TempDir::new().unwrap();
            let spec = base_spec(vec![FileCheck {
                required: false,
                ..base_file_check(&format!("{}.txt", path))
            }]);

            let report = spec.validate(dir.path());
            prop_assert!(report.pass);
        }

        #[test]
        fn line_count_validation_property(lines in 10usize..100) {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.txt");
            let content = "\n".repeat(lines);
            fs::write(&path, content).unwrap();

            let spec = base_spec(vec![FileCheck {
                require_line_count: Some(lines / 2),
                ..base_file_check("test.txt")
            }]);

            let report = spec.validate(dir.path());
            prop_assert!(report.pass);
        }
    }
}
