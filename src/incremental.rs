//! Git-diff-aware change detection for `--incremental`.
//!
//! Computes the set of base-relative paths whose worktree state differs from
//! `HEAD` (modified, staged, or untracked files). File checks targeting paths
//! outside this set may replay cached results pinned to the current HEAD
//! without being re-read.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// The commit `HEAD` currently points at, as a hex object id.
pub fn head_commit(base: &Path) -> Result<String, String> {
    let head = git(base, &["rev-parse", "--verify", "HEAD"])?;
    Ok(head.trim().to_string())
}

/// Base-relative normalized paths changed since `HEAD`.
pub fn changed_paths(base: &Path) -> Result<HashSet<PathBuf>, String> {
    let top_raw = git(base, &["rev-parse", "--show-toplevel"])?;
    let top = normalize(Path::new(top_raw.trim()));
    // Canonicalize like `base` below so prefix stripping compares like paths;
    // lexical normalization stays as fallback if resolution fails.
    let top = std::fs::canonicalize(&top).unwrap_or(top);
    // Verified first so that repos without commits fail with a clear message.
    head_commit(base)?;

    let mut changed = HashSet::new();
    let diff = git(base, &["diff", "--name-only", "HEAD"])?;
    let untracked = git(base, &["ls-files", "--others", "--exclude-standard"])?;
    for line in diff.lines().chain(untracked.lines()) {
        if line.is_empty() {
            continue;
        }
        let path = normalize(Path::new(line));
        if let Some(rel) = path.strip_prefix(&top) {
            changed.insert(rel.to_path_buf());
        }
    }

    let canonical_base = base
        .canonicalize()
        .map_err(|e| format!("failed to resolve --base {}: {}", base.display(), e))?;
    let base_rel = canonical_base
        .strip_prefix(&top)
        .map_err(|_| {
            format!(
                "--base {} is not inside the git work tree {}",
                canonical_base.display(),
                top.display()
            )
        })?
        .to_path_buf();

    Ok(changed
        .into_iter()
        .filter_map(|path| path.strip_prefix(&base_rel).ok().map(Path::to_path_buf))
        .collect())
}

fn git(base: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(base)
        .output()
        .map_err(|e| format!("failed to run git: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "unknown error".to_string()
        } else {
            stderr
        };
        return Err(format!(
            "git {} failed: {}. Suggestion: --incremental requires --base to be a git repository with at least one commit.",
            args.join(" "),
            detail
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Lexical normalization: resolve `.` and `..` components without touching
/// the filesystem, so git-reported paths and spec paths compare equal.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Run git in `dir`, failing the test on error so assertions stay clean.
    fn git_ok(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=deliver-test", "-c", "user.email=test@example.com"])
            .args(args)
            .status()
            .expect("git binary available for incremental tests");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn commit_file(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
        git_ok(dir, &["add", name]);
        git_ok(dir, &["commit", "-m", &format!("add {name}")]);
    }

    #[test]
    fn clean_tree_reports_no_changes() {
        let dir = TempDir::new().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        commit_file(dir.path(), "a.txt", "a");

        let changed = changed_paths(dir.path()).unwrap();
        assert!(changed.is_empty());
    }

    #[test]
    fn modified_staged_and_untracked_files_are_reported() {
        let dir = TempDir::new().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        commit_file(dir.path(), "committed.txt", "v1");
        commit_file(dir.path(), "staged.txt", "v1");

        fs::write(dir.path().join("committed.txt"), "v2").unwrap();
        fs::write(dir.path().join("staged.txt"), "v2").unwrap();
        git_ok(dir.path(), &["add", "staged.txt"]);
        fs::write(dir.path().join("untracked.txt"), "new").unwrap();

        let changed = changed_paths(dir.path()).unwrap();
        assert!(changed.contains(Path::new("committed.txt")));
        assert!(changed.contains(Path::new("staged.txt")));
        assert!(changed.contains(Path::new("untracked.txt")));
        assert_eq!(changed.len(), 3);
    }

    #[test]
    fn nested_file_is_relativized_to_base() {
        let dir = TempDir::new().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        commit_file(dir.path(), "sub/inner.txt", "v1");
        fs::write(dir.path().join("sub/inner.txt"), "v2").unwrap();

        let repo_root = dir.path();
        let sub_base = repo_root.join("sub");
        let changed = changed_paths(&sub_base).unwrap();
        assert!(changed.contains(Path::new("inner.txt")));
        assert!(!changed.contains(Path::new("sub/inner.txt")));

        let root_changed = changed_paths(repo_root).unwrap();
        assert!(root_changed.contains(Path::new("sub/inner.txt")));
    }

    #[test]
    fn non_repo_is_an_error() {
        let dir = TempDir::new().unwrap();
        let result = changed_paths(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("--incremental"));
    }

    #[test]
    fn repo_without_commits_is_an_error() {
        let dir = TempDir::new().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        let result = changed_paths(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one commit"));
    }

    #[test]
    fn head_commit_returns_hex_id() {
        let dir = TempDir::new().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        commit_file(dir.path(), "a.txt", "a");
        let head = head_commit(dir.path()).unwrap();
        assert_eq!(head.len(), 40);
        assert!(head.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn normalize_resolves_dot_and_dotdot() {
        assert_eq!(normalize(Path::new("./a/./b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalize(Path::new("a/b/../../d")), PathBuf::from("d"));
    }
}
