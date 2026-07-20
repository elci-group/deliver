mod command;
mod file_check;
mod git_check;
mod glob;

pub use glob::expand_paths;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    #[serde(default, alias = "file")]
    pub files: Vec<FileCheck>,
    #[serde(default, alias = "command")]
    pub commands: Vec<CommandCheck>,
    #[serde(default)]
    pub git: GitCheck,
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

    pub fn validate(&self, base: &Path) -> Report {
        let start = Instant::now();
        let mut checks = Vec::new();
        let mut pass = true;

        for file in &self.files {
            let result = crate::file_check::validate_file(file, base);
            if !result.pass {
                pass = false;
            }
            checks.push(result);
        }

        for command in &self.commands {
            let result = crate::command::validate_command(command, base);
            if !result.pass {
                pass = false;
            }
            checks.push(result);
        }

        if self.git.no_uncommitted_changes || self.git.no_untracked_files {
            let git_results = crate::git_check::validate_git(&self.git, base);
            for r in git_results {
                if !r.pass {
                    pass = false;
                }
                checks.push(r);
            }
        }

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
            };
            crate::file_check::validate_file(&check, base)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn validates_required_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hello.txt");
        fs::write(&path, "hello world").unwrap();

        let spec = Spec {
            files: vec![FileCheck {
                path: "hello.txt".to_string(),
                required: true,
                max_size_bytes: None,
                min_size_bytes: None,
                forbid_regex: Vec::new(),
                require_regex: vec!["hello".to_string()],
                require_line_count: None,
            }],
            commands: Vec::new(),
            git: GitCheck::default(),
            metadata: HashMap::new(),
        };

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

        let spec = Spec {
            files: vec![FileCheck {
                path: "bad.rs".to_string(),
                required: true,
                max_size_bytes: None,
                min_size_bytes: None,
                forbid_regex: vec!["panic!".to_string()],
                require_regex: Vec::new(),
                require_line_count: None,
            }],
            commands: Vec::new(),
            git: GitCheck::default(),
            metadata: HashMap::new(),
        };

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
}
