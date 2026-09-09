//! Git hook, CI and workflow integrations for Ogma.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Error type for integration operations.
#[derive(Clone, Debug)]
pub struct IntegrationError(pub String);

impl fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IntegrationError {}

const HOOK_BEGIN: &str = "# ogma-begin";
const HOOK_END: &str = "# ogma-end";

/// The pre-commit hook body Ogma installs.
pub fn pre_commit_hook_contents() -> &'static str {
    r#"#!/bin/sh
# ogma-begin
# Run Ogma's deterministic multilingual conformance check before each
# commit. A non-zero exit from `ogma check` blocks the commit.
ogma check
# ogma-end
"#
}

/// GitHub Actions workflow YAML running `ogma check` on push and pull
/// requests.
pub fn ci_workflow_yaml() -> &'static str {
    r#"# GitHub Actions workflow for Ogma deterministic multilingual
# conformance checks. Runs on every push and pull request.
name: ogma

on:
  push:
  pull_request:

jobs:
  ogma:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      # In-repo install: builds the `ogma` binary from this repository.
      # Once Ogma is published, replace this with:
      #   cargo install ogma-cli --locked
      - name: Install ogma
        run: cargo install --path crates/ogma-cli --locked

      - name: Run ogma check
        run: ogma check
"#
}

/// Install the ogma pre-commit hook under `<root>/.git/hooks/pre-commit`.
///
/// Idempotent: when the hook already contains the ogma marker this is a
/// no-op returning the hook path. When a hook exists without the marker,
/// the ogma block is appended. Errors when `<root>/.git` is absent.
pub fn install_pre_commit_hook(root: &Path) -> Result<PathBuf, IntegrationError> {
    let git_dir = root.join(".git");
    if !git_dir.is_dir() {
        return Err(IntegrationError(format!(
            "no .git directory at {}",
            git_dir.display()
        )));
    }
    let hooks_dir = git_dir.join("hooks");
    fs::create_dir_all(&hooks_dir).map_err(|e| {
        IntegrationError(format!("cannot create {}: {e}", hooks_dir.display()))
    })?;

    let hook_path = hooks_dir.join("pre-commit");
    let block = hook_block();

    let existing = fs::read_to_string(&hook_path).unwrap_or_default();
    if existing.contains(HOOK_BEGIN) {
        return Ok(hook_path);
    }

    let mut new_contents = existing;
    if new_contents.is_empty() {
        new_contents.push_str(pre_commit_hook_contents());
    } else {
        if !new_contents.ends_with('\n') {
            new_contents.push('\n');
        }
        new_contents.push_str(&block);
    }

    fs::write(&hook_path, &new_contents)
        .map_err(|e| IntegrationError(format!("cannot write {}: {e}", hook_path.display())))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)
            .map_err(|e| IntegrationError(format!("cannot stat {}: {e}", hook_path.display())))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms).map_err(|e| {
            IntegrationError(format!("cannot chmod {}: {e}", hook_path.display()))
        })?;
    }

    Ok(hook_path)
}

/// Remove the ogma block from an existing pre-commit hook. When the hook
/// consists solely of the ogma block the whole file is removed. Returns
/// true when something changed.
pub fn remove_pre_commit_hook(root: &Path) -> Result<bool, IntegrationError> {
    let hook_path = root.join(".git").join("hooks").join("pre-commit");
    let existing = match fs::read_to_string(&hook_path) {
        Ok(text) => text,
        Err(_) => return Ok(false),
    };
    if !existing.contains(HOOK_BEGIN) {
        return Ok(false);
    }

    let blockless = remove_block(&existing);
    // When nothing but whitespace and possibly the shebang line remains,
    // the hook consisted solely of the ogma block: remove the whole file.
    let remainder = blockless
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("#!"))
        .count();

    if remainder == 0 {
        fs::remove_file(&hook_path).map_err(|e| {
            IntegrationError(format!("cannot remove {}: {e}", hook_path.display()))
        })?;
    } else if blockless != existing {
        fs::write(&hook_path, blockless).map_err(|e| {
            IntegrationError(format!("cannot write {}: {e}", hook_path.display()))
        })?;
    } else {
        return Ok(false);
    }

    Ok(true)
}

/// The ogma block (markers plus check) as appended to existing hooks.
fn hook_block() -> String {
    let full = pre_commit_hook_contents();
    full[full.find(HOOK_BEGIN).unwrap()..].to_string()
}

/// Strip ogma begin..end blocks (inclusive) from hook text, dropping any
/// shebang the block carried when appending to an existing hook.
fn remove_block(text: &str) -> String {
    let mut out = String::new();
    let mut in_block = false;
    for line in text.lines() {
        if line.trim() == HOOK_BEGIN {
            in_block = true;
            continue;
        }
        if in_block && line.trim() == HOOK_END {
            in_block = false;
            continue;
        }
        if !in_block {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
