use crate::{CheckResult, GitCheck};
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

    results
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
