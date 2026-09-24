use crate::{CheckResult, GitCheck};
use regex::Regex;
use std::path::Path;
use std::process::Command;

pub fn validate_git(check: &GitCheck, base: &Path) -> Vec<CheckResult> {
    let mut results = Vec::new();

    if check.no_uncommitted_changes {
        results.push(validate_no_uncommitted_changes(base));
    }

    if check.no_untracked_files {
        results.push(validate_no_untracked_files(base));
    }

    if let Some(pattern) = &check.require_branch_regex {
        results.push(validate_branch(base, pattern));
    }

    if let Some(pattern) = &check.require_commit_message_regex {
        results.push(validate_commit_message(base, pattern));
    }

    results
}

fn validate_branch(base: &Path, pattern: &str) -> CheckResult {
    let name = "git: branch name".to_string();
    let regex = match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => {
            return CheckResult {
                name,
                pass: false,
                kind: "git".to_string(),
                message: format!(
                    "invalid require_branch_regex '{}': {}. Suggestion: fix the regex syntax.",
                    pattern, error
                ),
            }
        }
    };

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(base)
        .output();

    let branch = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => {
            return CheckResult {
                name,
                pass: false,
                kind: "git".to_string(),
                message: "could not determine current branch. Suggestion: ensure --base is inside a Git repository with at least one commit.".to_string(),
            }
        }
    };

    let pass = regex.is_match(&branch);
    CheckResult {
        name,
        pass,
        kind: "git".to_string(),
        message: if pass {
            format!("branch '{}' matches '{}'", branch, pattern)
        } else {
            format!(
                "branch '{}' does not match required pattern '{}'. Suggestion: rename the branch or adjust require_branch_regex.",
                branch, pattern
            )
        },
    }
}

fn invalid_commit_message_regex(name: String, pattern: &str, error: regex::Error) -> CheckResult {
    CheckResult {
        name,
        pass: false,
        kind: "git".to_string(),
        message: format!(
            "invalid require_commit_message_regex '{}': {}. Suggestion: fix the regex syntax.",
            pattern, error
        ),
    }
}

fn validate_commit_message(base: &Path, pattern: &str) -> CheckResult {
    let name = "git: last commit message".to_string();
    let regex = match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => return invalid_commit_message_regex(name, pattern, error),
    };

    let output = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(base)
        .output();

    let message = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => {
            return CheckResult {
                name,
                pass: false,
                kind: "git".to_string(),
                message: "could not read the last commit message. Suggestion: ensure --base is inside a Git repository with at least one commit.".to_string(),
            }
        }
    };

    let pass = regex.is_match(&message);
    CheckResult {
        name,
        pass,
        kind: "git".to_string(),
        message: if pass {
            format!("commit message matches '{}'", pattern)
        } else {
            format!(
                "commit message does not match required pattern '{}'. Suggestion: amend the commit message or adjust require_commit_message_regex.",
                pattern
            )
        },
    }
}

fn validate_no_uncommitted_changes(base: &Path) -> CheckResult {
    let output = Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(base)
        .output();
    let pass = output
        .map(|output| output.status.success())
        .unwrap_or(false);

    CheckResult {
        name: "git: no uncommitted changes".to_string(),
        pass,
        kind: "git".to_string(),
        message: if pass {
            "working tree clean".to_string()
        } else {
            "uncommitted changes detected".to_string()
        },
    }
}

fn validate_no_untracked_files(base: &Path) -> CheckResult {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(base)
        .output();
    let untracked = output
        .map(|output| String::from_utf8_lossy(&output.stdout).lines().count())
        .unwrap_or(0);
    let pass = untracked == 0;

    CheckResult {
        name: "git: no untracked files".to_string(),
        pass,
        kind: "git".to_string(),
        message: if pass {
            "no untracked files".to_string()
        } else {
            format!("{} untracked file(s)", untracked)
        },
    }
}
