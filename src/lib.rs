//! `deliver` — deterministic deliverable validator for agent workflows.
//!
//! The library crate behind the `deliver` CLI: load a TOML or JSON spec,
//! run file, command, directory, plugin, SBOM, and Git checks against a
//! base directory, and produce a stable, machine-readable report.
//!
//! # Entry points
//!
//! - [`Spec`] — load and validate a spec, then run it:
//!   [`Spec::load_file`]/[`Spec::from_toml`]/[`Spec::from_json`], then
//!   [`Spec::validate`], [`Spec::validate_with_jobs`], or
//!   [`Spec::validate_with_options`].
//! - [`CheckResult`] / [`Report`] — the result records; their serialized
//!   JSON shape is part of the public contract (see `--format json`).
//! - [`ValidateOptions`] — caching, incremental validation, and fail-fast.
//! - [`spec_json_schema`] — a draft-07 JSON Schema for the spec format.
//! - [`quick_check_files`] — the `--file` shorthand used by the CLI.
//!
//! # API stability (1.0)
//!
//! This crate follows Semantic Versioning from 1.0.0:
//!
//! - Everything reachable from this crate root is public API: the spec
//!   types (`Spec`, `FileCheck`, `CommandCheck`, `DirectoryCheck`,
//!   `GitCheck`, `PluginCheck`, `SbomCheck`, `When`, `CommandSpec`), the
//!   report types (`Report`, `CheckResult`), the run options
//!   ([`ValidateOptions`]), and the [`cache`] / [`incremental`] modules.
//! - Serialized shapes are contract too: the JSON report (check objects
//!   with `name`, `pass`, `kind`, `message`, optional `weight` and
//!   `skipped`), the attestation bundle layout, and the plugin protocol
//!   (v1) only ever change additively within a major version.
//! - The spec file format is fail-closed (unknown fields are parse
//!   errors), so adding a spec field is a minor change; removing,
//!   renaming, or reinterpreting one requires a major version.
//! - Breaking changes to any of the above require a major version bump.

#![deny(missing_docs)]

pub mod cache;
mod command;
mod directory_check;
mod file_check;
mod git_check;
mod glob;
pub mod incremental;
mod path_guard;
mod plugin_check;
mod sbom_check;
mod schema;

pub use file_check::validate_file;
pub use glob::expand_paths;
pub use plugin_check::PROTOCOL_VERSION;
pub use schema::spec_json_schema;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A validation spec: the checks to run against a base directory, loaded
/// from TOML or JSON. Parse it with [`Spec::load_file`] (follows `extends`),
/// [`Spec::from_toml`], or [`Spec::from_json`]; run it with [`Spec::validate`],
/// [`Spec::validate_with_jobs`], or [`Spec::validate_with_options`].
///
/// Unknown fields are parse errors, so a misspelled requirement can never be
/// silently skipped.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Path to a parent spec to inherit from, resolved relative to the
    /// directory of the spec file that declares it. Only honored by
    /// [`Spec::load_file`]; ignored by [`Spec::from_toml`]/[`Spec::from_json`].
    #[serde(default)]
    pub extends: Option<String>,
    /// File checks; deserialize from `[[file]]` (or `[[files]]`).
    #[serde(default, alias = "file")]
    pub files: Vec<FileCheck>,
    /// Command checks; deserialize from `[[command]]` (or `[[commands]]`).
    #[serde(default, alias = "command")]
    pub commands: Vec<CommandCheck>,
    /// Git cleanliness checks, run last and sequentially.
    #[serde(default)]
    pub git: GitCheck,
    /// Directory checks; deserialize from `[[directory]]` (or
    /// `[[directories]]`).
    #[serde(default, alias = "directory")]
    pub directories: Vec<DirectoryCheck>,
    /// Plugin-protocol checks; accumulate across `extends` like commands.
    #[serde(default, alias = "plugin")]
    pub plugins: Vec<PluginCheck>,
    /// SBOM presence/shape check; present when the spec declares `[sbom]`.
    #[serde(default)]
    pub sbom: Option<SbomCheck>,
    /// Free-form labels recorded alongside the spec; never validated.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Skip conditions for a check. Every declared condition must hold for the
/// check to run; when any condition fails, the check is skipped instead of
/// being evaluated. Skipped checks neither pass nor fail the run, and are
/// excluded from weighted completion reporting.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Base-relative path that must exist for the check to run. A path that
    /// escapes `--base` never matches, and says so in the skip message.
    #[serde(default)]
    pub file_exists: Option<String>,
    /// Operating system that must match the running one (`linux`, `macos`,
    /// or `windows`; compared case-insensitively).
    #[serde(default)]
    pub os: Option<String>,
}

impl When {
    /// `Some(reason)` when a declared condition does not hold and the check
    /// should be skipped; `None` when every condition holds (or none are
    /// declared). The reason is human-readable and names the failing clause.
    pub fn skip_reason(&self, base: &Path) -> Option<String> {
        if let Some(file) = &self.file_exists {
            match crate::path_guard::resolve(base, Path::new(file)) {
                Ok(path) if path.exists() => {}
                Ok(_) => {
                    return Some(format!("when.file_exists \"{}\" not found", file));
                }
                Err(_) => {
                    return Some(format!("when.file_exists \"{}\" is outside --base", file));
                }
            }
        }
        if let Some(os) = &self.os {
            if !os.eq_ignore_ascii_case(std::env::consts::OS) {
                return Some(format!(
                    "when.os \"{}\" does not match \"{}\"",
                    os,
                    std::env::consts::OS
                ));
            }
        }
        None
    }
}

/// A single-file check: existence, size, content patterns, encoding, hashes,
/// schema validation, key paths, reference resolution, and secret scanning.
/// Paths are relative to `--base` and may not escape it (including through
/// symlinks).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileCheck {
    /// Base-relative path of the file to validate.
    pub path: String,
    /// Fail when the file does not exist; `false` makes absence a pass.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Fail when the file is larger than this many bytes.
    #[serde(default)]
    pub max_size_bytes: Option<u64>,
    /// Fail when the file is smaller than this many bytes.
    #[serde(default)]
    pub min_size_bytes: Option<u64>,
    /// Fail when any pattern matches the file content.
    #[serde(default)]
    pub forbid_regex: Vec<String>,
    /// Fail unless every pattern matches the file content.
    #[serde(default)]
    pub require_regex: Vec<String>,
    /// Fail when the file has fewer than this many lines.
    #[serde(default)]
    pub require_line_count: Option<usize>,
    /// Required content encoding (`utf-8`, `utf8`, or `ascii`).
    #[serde(default)]
    pub encoding: Option<String>,
    /// Fail when the path is a symlink rather than a regular file.
    #[serde(default)]
    pub forbid_symlinks: bool,
    /// SHA-256 the file content must match (lowercase or uppercase hex).
    #[serde(default)]
    pub sha256: Option<String>,
    /// BLAKE3 hash the file content must match.
    #[serde(default)]
    pub blake3: Option<String>,
    /// Substring that must appear in the first ten lines (e.g. a license
    /// identifier).
    #[serde(default)]
    pub require_license_header: Option<String>,
    /// JSON Schema document (as a string) the parsed JSON content must
    /// satisfy. External references fail; schemas must be self-contained.
    #[serde(default)]
    pub json_schema: Option<String>,
    /// Dotted key paths that must exist in the parsed TOML content.
    #[serde(default)]
    pub require_toml_keys: Vec<String>,
    /// Dotted key paths that must exist in the parsed JSON content.
    #[serde(default)]
    pub require_json_keys: Vec<String>,
    /// YAML/JSON Schema document (as a string) the parsed YAML content must
    /// satisfy.
    #[serde(default)]
    pub yaml_schema: Option<String>,
    /// For `.rs`/`.py`/`.js`/`.ts`/`.jsx`/`.tsx`/C/C++ files, fail when a
    /// local module/import/include reference points at a missing file.
    #[serde(default)]
    pub check_references: bool,
    /// Fail when the file contains content matching a built-in,
    /// high-confidence secret pattern (AWS keys, provider tokens, private
    /// key blocks, ...). The report names the pattern and line, never the
    /// matched content.
    #[serde(default)]
    pub forbid_secrets: bool,
    /// Relative importance in weighted completion reporting; unweighted
    /// checks count as 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Skip conditions; the check is skipped rather than failed when any
    /// declared condition does not hold.
    #[serde(default)]
    pub when: Option<When>,
}

/// A command check: run a program from a working directory under `--base`
/// and validate its exit status and captured output. Commands require the
/// CLI's `--allow-commands` trust flag.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCheck {
    /// Report name for the check.
    pub name: String,
    /// Program and arguments; a string is whitespace-split, an array is
    /// passed through verbatim (prefer the array form).
    pub cmd: CommandSpec,
    /// Working directory relative to `--base` (default `.`).
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    /// Additional or overriding environment variables for the child.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Exit code the process must produce (default 0).
    #[serde(default = "default_expect_exit")]
    pub expect_exit: i32,
    /// Wall-clock budget in seconds before the process (group) is killed
    /// (default 300).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Substrings that must appear in captured stdout.
    #[serde(default)]
    pub stdout_contains: Vec<String>,
    /// Substrings that must appear in captured stderr.
    #[serde(default)]
    pub stderr_contains: Vec<String>,
    /// Relative importance in weighted completion reporting; unweighted
    /// checks count as 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Skip conditions; the check is skipped rather than failed when any
    /// declared condition does not hold.
    #[serde(default)]
    pub when: Option<When>,
}

/// The `cmd` field of command and plugin checks: either a single string
/// (whitespace-split) or an explicit argument array.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CommandSpec {
    /// A single string, split on whitespace.
    String(String),
    /// An explicit program/argument array, passed through verbatim.
    Array(Vec<String>),
}

/// Git cleanliness checks, run in `--base` after all other checks.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GitCheck {
    /// Fail when tracked files have uncommitted modifications.
    #[serde(default)]
    pub no_uncommitted_changes: bool,
    /// Fail when untracked files exist.
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

/// A directory check: existence and the number of regular files directly
/// inside it (non-recursive).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryCheck {
    /// Base-relative path of the directory to validate.
    pub path: String,
    /// Fail when the directory does not exist; `false` makes absence a pass.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Fail when fewer than this many files sit directly inside.
    #[serde(default)]
    pub min_files: Option<usize>,
    /// Fail when more than this many files sit directly inside.
    #[serde(default)]
    pub max_files: Option<usize>,
    /// Fail when the directory contains zero files.
    #[serde(default)]
    pub forbid_empty: bool,
    /// Relative importance in weighted completion reporting; unweighted
    /// checks count as 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Skip conditions; the check is skipped rather than failed when any
    /// declared condition does not hold.
    #[serde(default)]
    pub when: Option<When>,
}

/// An extensibility check: runs an external program that speaks the plugin
/// protocol (one JSON request on stdin, one JSON response on stdout, v1).
/// Plugins execute arbitrary code and require the same explicit
/// `--allow-commands` trust decision as command checks.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCheck {
    /// Report name for the check; also sent in the protocol request.
    pub name: String,
    /// Program and arguments; a string is whitespace-split, an array is
    /// passed through verbatim.
    pub cmd: CommandSpec,
    /// Working directory relative to `--base` (default `.`).
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    /// Wall-clock budget in seconds before the plugin (group) is killed
    /// (default 300).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Relative importance in weighted completion reporting; unweighted
    /// checks count as 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Skip conditions; the check is skipped rather than failed when any
    /// declared condition does not hold.
    #[serde(default)]
    pub when: Option<When>,
}

/// An SBOM presence and shape check: searches standard Software Bill of
/// Materials locations under `--base` and validates that a found document
/// has the core fields of its declared format.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SbomCheck {
    /// Fail when no SBOM is found under `--base`. With `required = false`,
    /// absence passes; an existing but malformed SBOM still fails.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Accepted formats (`spdx`, `cyclonedx`); empty accepts any supported
    /// format.
    #[serde(default)]
    pub formats: Vec<String>,
    /// Relative importance in weighted completion reporting; unweighted
    /// checks count as 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
}

/// The outcome of a full validation run: overall pass/fail plus one
/// [`CheckResult`] per check, in spec order.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// True only when every executed check passed (skipped checks never
    /// fail a run).
    pub pass: bool,
    /// Results in spec order: files, commands, directories, plugins, the
    /// SBOM check, then git checks.
    pub checks: Vec<CheckResult>,
    /// Wall-clock duration of the run in milliseconds.
    pub duration_ms: u128,
}

/// The result of one check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Report name: `file: <path>`, the command/plugin `name`,
    /// `directory: <path>`, `sbom`, or a `git: ...` label.
    pub name: String,
    /// Whether the check passed; skipped checks report `true`.
    pub pass: bool,
    /// Check kind: `file`, `command`, `directory`, `plugin`, `sbom`, or
    /// `git`.
    pub kind: String,
    /// Human-readable outcome, including suggestions on failure and skip
    /// reasons; never echoes secret content or expected command values.
    pub message: String,
    /// Declared weight from the spec, when the check sets one. Omitted from
    /// serialized reports for unweighted checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// True when the check's `when` clause did not hold and the check was
    /// skipped: skipped checks report `pass = true` (they never fail a run)
    /// but are excluded from weighted completion reporting. Omitted from
    /// serialized reports for executed checks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

/// A `CheckResult` for a check whose `when` clause did not hold: recorded as
/// passing (a skip never fails the run) with the skip reason in the message.
fn skipped_check(kind: &str, name: String, weight: Option<f64>, reason: String) -> CheckResult {
    CheckResult {
        name,
        pass: true,
        kind: kind.to_string(),
        weight,
        message: format!("skipped: {}", reason),
        skipped: true,
    }
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
    /// Parse a spec from a TOML string. Does not follow `extends`.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Parse a spec from a JSON string. Does not follow `extends`.
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
        let mut plugins = parent.plugins;
        plugins.extend(child.plugins);
        let mut metadata = parent.metadata;
        metadata.extend(child.metadata);
        // A child's [sbom] section wins; a child cannot unset a parent's
        // check, matching the can-only-add rule of the git flags.
        let sbom = child.sbom.or(parent.sbom);
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
            plugins,
            sbom,
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
        self.validate_with_options(base, jobs, &ValidateOptions::default())
            .expect("default options cannot fail")
    }

    /// [`validate_with_jobs`] with optional result caching, git-diff-aware
    /// incremental validation, and fail-fast early termination.
    ///
    /// Returns `Err` only for setup failures (currently: `incremental`
    /// requires `base` to be inside a git repository with at least one
    /// commit). Cache read/write failures never fail the run; caching is an
    /// optimization, never a gate.
    pub fn validate_with_options(
        &self,
        base: &Path,
        jobs: usize,
        options: &ValidateOptions,
    ) -> Result<Report, String> {
        self.validate_with_options_and_events(base, jobs, options, &|_| {})
    }

    /// [`validate_with_options`] plus a callback invoked once per check the
    /// moment its result exists — including skipped checks and git checks —
    /// so callers can stream results instead of waiting for the whole run.
    /// With `jobs = 1` events arrive in spec order; with more jobs they
    /// arrive in completion order. The returned `Report` is unchanged and
    /// always spec-ordered.
    pub fn validate_with_options_and_events(
        &self,
        base: &Path,
        jobs: usize,
        options: &ValidateOptions,
        on_result: &(dyn Fn(&CheckResult) + Send + Sync),
    ) -> Result<Report, String> {
        let start = Instant::now();
        let jobs = jobs.max(1);

        let incremental_state = if options.incremental {
            let head = incremental::head_commit(base)?;
            let changed = incremental::changed_paths(base)?;
            Some((head, changed))
        } else {
            None
        };

        let cache = options
            .cache_dir
            .as_ref()
            .map(|dir| std::sync::Mutex::new(cache::Cache::load(cache::cache_path(dir, base))));

        let failed = std::sync::atomic::AtomicBool::new(false);

        enum Task<'a> {
            File(&'a FileCheck),
            Command(&'a CommandCheck),
            Directory(&'a DirectoryCheck),
            Plugin(&'a PluginCheck),
        }

        let mut tasks: Vec<Task> = Vec::with_capacity(
            self.files.len() + self.commands.len() + self.directories.len() + self.plugins.len(),
        );
        tasks.extend(self.files.iter().map(Task::File));
        tasks.extend(self.commands.iter().map(Task::Command));
        tasks.extend(self.directories.iter().map(Task::Directory));
        tasks.extend(self.plugins.iter().map(Task::Plugin));

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
                        let (kind, name, weight) = match tasks[i] {
                            Task::File(file) => {
                                ("file", format!("file: {}", file.path), file.weight)
                            }
                            Task::Command(command) => {
                                ("command", command.name.clone(), command.weight)
                            }
                            Task::Directory(directory) => (
                                "directory",
                                format!("directory: {}", directory.path),
                                directory.weight,
                            ),
                            Task::Plugin(plugin) => ("plugin", plugin.name.clone(), plugin.weight),
                        };
                        let result = if options.fail_fast
                            && failed.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            skipped_check(
                                kind,
                                name,
                                weight,
                                "fail-fast: remaining checks were not run".to_string(),
                            )
                        } else {
                            let result = match tasks[i] {
                                Task::File(file) => {
                                    match file.when.as_ref().and_then(|when| when.skip_reason(base))
                                    {
                                        Some(reason) => skipped_check(
                                            "file",
                                            format!("file: {}", file.path),
                                            file.weight,
                                            reason,
                                        ),
                                        None => match &cache {
                                            Some(cache) => validate_file_cached(
                                                file,
                                                base,
                                                cache,
                                                incremental_state.as_ref(),
                                            ),
                                            None => crate::file_check::validate_file(file, base),
                                        },
                                    }
                                }
                                Task::Command(command) => {
                                    match command
                                        .when
                                        .as_ref()
                                        .and_then(|when| when.skip_reason(base))
                                    {
                                        Some(reason) => skipped_check(
                                            "command",
                                            command.name.clone(),
                                            command.weight,
                                            reason,
                                        ),
                                        None => crate::command::validate_command(command, base),
                                    }
                                }
                                Task::Directory(directory) => {
                                    match directory
                                        .when
                                        .as_ref()
                                        .and_then(|when| when.skip_reason(base))
                                    {
                                        Some(reason) => skipped_check(
                                            "directory",
                                            format!("directory: {}", directory.path),
                                            directory.weight,
                                            reason,
                                        ),
                                        None => crate::directory_check::validate_directory(
                                            directory, base,
                                        ),
                                    }
                                }
                                Task::Plugin(plugin) => {
                                    match plugin
                                        .when
                                        .as_ref()
                                        .and_then(|when| when.skip_reason(base))
                                    {
                                        Some(reason) => skipped_check(
                                            "plugin",
                                            plugin.name.clone(),
                                            plugin.weight,
                                            reason,
                                        ),
                                        None => crate::plugin_check::validate_plugin(plugin, base),
                                    }
                                }
                            };
                            if !result.pass {
                                failed.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            result
                        };
                        on_result(&result);
                        *slots[i].lock().unwrap() = Some(result);
                    });
                }
            });
        }

        let mut checks: Vec<CheckResult> = slots
            .into_iter()
            .map(|slot| slot.into_inner().unwrap().expect("every task slot filled"))
            .collect();

        let failed_so_far = checks.iter().any(|check| !check.pass);
        if let Some(sbom) = &self.sbom {
            let result = if options.fail_fast && failed_so_far {
                skipped_check(
                    "sbom",
                    "sbom".to_string(),
                    sbom.weight,
                    "fail-fast: remaining checks were not run".to_string(),
                )
            } else {
                crate::sbom_check::validate_sbom(sbom, base)
            };
            on_result(&result);
            checks.push(result);
        }
        if self.git.any_enabled() {
            if options.fail_fast && failed_so_far {
                let reason = "fail-fast: remaining checks were not run".to_string();
                let mut push = |name: &str| {
                    let result = skipped_check("git", name.to_string(), None, reason.clone());
                    checks.push(result);
                    on_result(checks.last().unwrap());
                };
                if self.git.no_uncommitted_changes {
                    push("git: no uncommitted changes");
                }
                if self.git.no_untracked_files {
                    push("git: no untracked files");
                }
                if self.git.require_branch_regex.is_some() {
                    push("git: branch name");
                }
                if self.git.require_commit_message_regex.is_some() {
                    push("git: last commit message");
                }
            } else {
                let results = crate::git_check::validate_git(&self.git, base);
                for result in &results {
                    checks.push(result.clone());
                    on_result(result);
                }
            }
        }

        // Persist the cache for the next run; failures to write are
        // intentionally ignored (caching is an optimization, never a gate).
        if let Some(cache) = &cache {
            let _ = cache.lock().unwrap().save();
        }

        let pass = checks.iter().all(|check| check.pass);

        Ok(Report {
            pass,
            checks,
            duration_ms: start.elapsed().as_millis(),
        })
    }
}

/// Options for [`Spec::validate_with_options`].
#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    /// Git-diff-aware incremental validation: replay cached results for
    /// files unchanged since `HEAD` instead of re-reading them. Requires
    /// `base` to be inside a git repository with at least one commit, and
    /// is only meaningful together with a `cache_dir`.
    pub incremental: bool,
    /// Stop running remaining checks after the first failure; skipped
    /// checks are reported with `skipped = true`. With more than one job,
    /// the set of checks that still executes is timing-dependent.
    pub fail_fast: bool,
    /// Directory for the result cache (one JSON file per validated base
    /// directory). `None` disables caching.
    pub cache_dir: Option<PathBuf>,
}

/// Run a file check, replaying a cached result when one is valid for the
/// check definition and the file's content.
///
/// Replay rules: with incremental state, a file whose normalized path did
/// not change since `HEAD` replays the entry pinned to that `HEAD` without
/// being re-read; every other path (and the non-incremental path) replays
/// only an entry recorded for the file's current content hash. Checks that
/// depend on the filesystem rather than content alone (`forbid_symlinks`)
/// never replay. Replayed messages are suffixed with `(cached)` so a
/// replayed result is always distinguishable from a fresh one.
fn validate_file_cached(
    check: &FileCheck,
    base: &Path,
    cache: &std::sync::Mutex<cache::Cache>,
    incremental_state: Option<&(String, std::collections::HashSet<PathBuf>)>,
) -> CheckResult {
    if check.forbid_symlinks {
        // Symlink-ness is not captured by the content hash, so these
        // checks always run fresh.
        return crate::file_check::validate_file(check, base);
    }

    let key = cache::Cache::check_key(check);
    let resolved = crate::path_guard::resolve(base, Path::new(&check.path));

    if let Some((head, changed)) = incremental_state {
        let path = incremental::normalize_path(Path::new(&check.path));
        if !changed.contains(&path) {
            if let Some(entry) = cache.lock().unwrap().find_by_head(&key, head) {
                return replay_cached(check, entry);
            }
            // First run at this HEAD for an unchanged file: validate, then
            // record under this HEAD so later runs replay without re-reading.
            let content_hash = content_hash_of(&resolved);
            return record_result(cache, check, base, Some(head), content_hash);
        }
    }

    let content_hash = content_hash_of(&resolved);
    let Some(content_hash) = content_hash else {
        // Unresolvable path or unreadable file: run the check so its
        // proper error message surfaces; there is nothing to cache under.
        return crate::file_check::validate_file(check, base);
    };
    if let Some(entry) = cache.lock().unwrap().find_by_content(&key, &content_hash) {
        return replay_cached(check, entry);
    }
    let head = incremental_state.as_ref().map(|(head, _)| head.as_str());
    record_result(cache, check, base, head, Some(content_hash))
}

fn content_hash_of(path: &Result<std::path::PathBuf, String>) -> Option<String> {
    match path {
        Ok(path) => fs::read(path)
            .ok()
            .map(|content| cache::Cache::content_hash(&content)),
        Err(_) => None,
    }
}

fn replay_cached(check: &FileCheck, entry: cache::CachedResult) -> CheckResult {
    CheckResult {
        name: format!("file: {}", check.path),
        pass: entry.pass,
        kind: "file".to_string(),
        weight: check.weight,
        message: format!("{} (cached)", entry.message),
        skipped: false,
    }
}

fn record_result(
    cache: &std::sync::Mutex<cache::Cache>,
    check: &FileCheck,
    base: &Path,
    head: Option<&str>,
    content_hash: Option<String>,
) -> CheckResult {
    let result = crate::file_check::validate_file(check, base);
    if let Some(content_hash) = content_hash {
        cache
            .lock()
            .unwrap()
            .insert(&cache::Cache::check_key(check), content_hash, head, &result);
    }
    result
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
                forbid_secrets: false,
                weight: None,
                when: None,
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
            forbid_secrets: false,
            weight: None,
            when: None,
        }
    }

    fn base_spec(files: Vec<FileCheck>) -> Spec {
        Spec {
            extends: None,
            files,
            commands: Vec::new(),
            git: GitCheck::default(),
            directories: Vec::new(),
            plugins: Vec::new(),
            sbom: None,
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

[[plugin]]
name = "org gate"
cmd = ["org-gate"]

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
        assert_eq!(spec.plugins.len(), 1);
        assert_eq!(spec.plugins[0].name, "org gate");
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

    fn when_spec(when: When) -> Spec {
        base_spec(vec![FileCheck {
            forbid_regex: vec!["forbidden".to_string()],
            when: Some(when),
            ..base_file_check("a.txt")
        }])
    }

    #[test]
    fn when_file_exists_missing_skips_the_check() {
        let dir = TempDir::new().unwrap();
        // The file the check targets is absent, which would normally fail;
        // the unmet `when` must skip it instead.
        let spec = when_spec(When {
            file_exists: Some("Cargo.toml".to_string()),
            os: None,
        });
        let report = spec.validate(dir.path());
        assert!(report.pass, "a skipped check must not fail the run");
        assert_eq!(report.checks.len(), 1);
        assert!(report.checks[0].skipped);
        assert!(report.checks[0].pass);
        assert!(report.checks[0].message.contains("when.file_exists"));
    }

    #[test]
    fn when_file_exists_present_runs_the_check() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "forbidden token").unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let spec = when_spec(When {
            file_exists: Some("Cargo.toml".to_string()),
            os: None,
        });
        let report = spec.validate(dir.path());
        assert!(!report.pass, "a met `when` must run, not skip, the check");
        assert!(!report.checks[0].skipped);
    }

    #[test]
    fn when_os_mismatch_skips_the_check() {
        let dir = TempDir::new().unwrap();
        let other_os = if cfg!(target_os = "linux") {
            "windows"
        } else {
            "linux"
        };
        let spec = when_spec(When {
            file_exists: None,
            os: Some(other_os.to_string()),
        });
        let report = spec.validate(dir.path());
        assert!(report.pass);
        assert!(report.checks[0].skipped);
        assert!(report.checks[0].message.contains("when.os"));
    }

    #[test]
    fn when_os_match_runs_the_check() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "forbidden token").unwrap();
        let spec = when_spec(When {
            file_exists: None,
            os: Some(std::env::consts::OS.to_uppercase()),
        });
        let report = spec.validate(dir.path());
        assert!(!report.pass);
        assert!(!report.checks[0].skipped);
    }

    #[test]
    fn when_conditions_combine_with_and() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        // `os` never matches, so the check must skip even though
        // `file_exists` holds.
        let spec = when_spec(When {
            file_exists: Some("Cargo.toml".to_string()),
            os: Some("nonexistent-os".to_string()),
        });
        let report = spec.validate(dir.path());
        assert!(report.pass);
        assert!(report.checks[0].skipped);
    }

    #[test]
    fn when_file_exists_outside_base_skips_with_reason() {
        let dir = TempDir::new().unwrap();
        let spec = when_spec(When {
            file_exists: Some("../outside.txt".to_string()),
            os: None,
        });
        let report = spec.validate(dir.path());
        assert!(report.pass);
        assert!(report.checks[0].skipped);
        assert!(report.checks[0].message.contains("outside --base"));
    }

    #[test]
    fn when_empty_clause_always_runs() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "forbidden token").unwrap();
        let spec = when_spec(When::default());
        let report = spec.validate(dir.path());
        assert!(!report.pass);
        assert!(!report.checks[0].skipped);
    }

    #[test]
    fn when_unknown_field_is_rejected() {
        let result = Spec::from_toml(
            r#"
            [[file]]
            path = "a.txt"
            when = { file_existsx = "Cargo.toml" }
            "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn when_applies_to_command_and_directory_checks() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("spec.toml"),
            r#"
            [[command]]
            name = "cargo test"
            cmd = ["echo", "ran"]
            when = { file_exists = "Cargo.toml" }

            [[directory]]
            path = "src"
            forbid_empty = true
            when = { os = "nonexistent-os" }
            "#,
        )
        .unwrap();
        let spec = Spec::load_file(&dir.path().join("spec.toml")).unwrap();
        let report = spec.validate(dir.path());
        assert!(report.pass);
        assert_eq!(report.checks.len(), 2);
        assert!(report.checks.iter().all(|check| check.skipped));
        assert!(report.checks[0].kind == "command");
        assert!(report.checks[1].kind == "directory");
    }

    fn cache_options(cache_dir: &Path) -> ValidateOptions {
        ValidateOptions {
            cache_dir: Some(cache_dir.to_path_buf()),
            ..Default::default()
        }
    }

    #[test]
    fn cache_replays_identical_content_and_reruns_changed_content() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "good content").unwrap();
        let spec = base_spec(vec![FileCheck {
            require_regex: vec!["good".to_string()],
            ..base_file_check("a.txt")
        }]);
        let options = cache_options(cache_dir.path());

        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(report.pass);
        assert!(!report.checks[0].message.contains("(cached)"));

        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(report.pass);
        assert!(
            report.checks[0].message.contains("(cached)"),
            "identical content must replay from the cache: {}",
            report.checks[0].message
        );

        // Changed content gets a new content hash, so the cached pass must
        // not be replayed as a false pass.
        fs::write(dir.path().join("a.txt"), "bad content").unwrap();
        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(!report.pass);
        assert!(!report.checks[0].message.contains("(cached)"));
    }

    #[test]
    fn cache_does_not_replay_for_forbid_symlinks_checks() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "content").unwrap();
        let spec = base_spec(vec![FileCheck {
            forbid_symlinks: true,
            ..base_file_check("a.txt")
        }]);
        let options = cache_options(cache_dir.path());

        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(report.pass);
        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(report.pass);
        assert!(
            !report.checks[0].message.contains("(cached)"),
            "symlink policy checks must always run fresh"
        );
    }

    #[test]
    fn spec_edit_invalidates_cached_entries() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "good content").unwrap();
        let options = cache_options(cache_dir.path());
        let build = |pattern: &str| {
            base_spec(vec![FileCheck {
                require_regex: vec![pattern.to_string()],
                ..base_file_check("a.txt")
            }])
        };

        assert!(
            build("good")
                .validate_with_options(dir.path(), 1, &options)
                .unwrap()
                .pass
        );
        // Same file content, different check definition: the check key
        // changes, so the fresh run must not report a cached result.
        let report = build("content")
            .validate_with_options(dir.path(), 1, &options)
            .unwrap();
        assert!(report.pass);
        assert!(!report.checks[0].message.contains("(cached)"));
    }

    #[test]
    fn fail_fast_skips_remaining_checks_and_git_checks() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "has forbidden token").unwrap();
        fs::write(dir.path().join("b.txt"), "clean").unwrap();
        let spec = base_spec(vec![
            FileCheck {
                forbid_regex: vec!["forbidden".to_string()],
                ..base_file_check("a.txt")
            },
            FileCheck {
                forbid_regex: vec!["impossible-match".to_string()],
                ..base_file_check("b.txt")
            },
        ]);
        let spec = Spec {
            git: GitCheck {
                no_uncommitted_changes: true,
                ..Default::default()
            },
            ..spec
        };
        let options = ValidateOptions {
            fail_fast: true,
            ..Default::default()
        };

        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(!report.pass);
        assert!(!report.checks[0].pass && !report.checks[0].skipped);
        assert!(report.checks[1].skipped && report.checks[1].pass);
        assert!(
            report.checks[1].message.contains("fail-fast"),
            "{}",
            report.checks[1].message
        );
        let git_result = report.checks.last().unwrap();
        assert!(
            git_result.skipped,
            "git checks must be skipped after failure"
        );
        assert!(git_result.message.contains("fail-fast"));

        // Without fail-fast every check runs.
        let report = spec
            .validate_with_options(dir.path(), 1, &Default::default())
            .unwrap();
        assert!(!report.checks.iter().any(|check| check.skipped));
    }

    #[test]
    fn fail_fast_is_not_triggered_by_skipped_checks() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("b.txt"), "has forbidden token").unwrap();
        let spec = base_spec(vec![
            FileCheck {
                when: Some(When {
                    file_exists: Some("Cargo.toml".to_string()),
                    os: None,
                }),
                ..base_file_check("a.txt")
            },
            FileCheck {
                forbid_regex: vec!["forbidden".to_string()],
                ..base_file_check("b.txt")
            },
        ]);
        let options = ValidateOptions {
            fail_fast: true,
            ..Default::default()
        };

        let report = spec.validate_with_options(dir.path(), 1, &options).unwrap();
        assert!(!report.pass);
        assert!(report.checks[0].skipped, "a `when` skip is not a failure");
        assert!(!report.checks[1].skipped, "the real failure must still run");
    }

    #[test]
    fn incremental_requires_git_repository() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        let spec = base_spec(vec![base_file_check("a.txt")]);
        let options = ValidateOptions {
            incremental: true,
            ..Default::default()
        };

        let error = spec
            .validate_with_options(dir.path(), 1, &options)
            .unwrap_err();
        assert!(error.contains("--incremental") || error.contains("git"));
    }

    #[test]
    fn events_fire_once_per_check_including_git() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        let spec = Spec {
            git: GitCheck {
                no_uncommitted_changes: true,
                ..Default::default()
            },
            ..base_spec(vec![base_file_check("a.txt"), base_file_check("b.txt")])
        };

        let collect = |events: &std::sync::Mutex<Vec<String>>| {
            let events_ref = &events;
            spec.validate_with_options_and_events(
                dir.path(),
                1,
                &Default::default(),
                &|result: &CheckResult| {
                    events_ref.lock().unwrap().push(result.name.clone());
                },
            )
            .unwrap()
        };

        let events = std::sync::Mutex::new(Vec::new());
        let report = collect(&events);
        let names = events.into_inner().unwrap();
        assert_eq!(names.len(), report.checks.len());
        // With one job, events arrive in spec order (files, then git).
        assert_eq!(names[0], "file: a.txt");
        assert_eq!(names[1], "file: b.txt");
        assert_eq!(names[2], "git: no uncommitted changes");

        // With many jobs every check still reports exactly once, in
        // completion order.
        let events = std::sync::Mutex::new(Vec::new());
        spec.validate_with_options_and_events(
            dir.path(),
            4,
            &Default::default(),
            &|result: &CheckResult| {
                events.lock().unwrap().push(result.name.clone());
            },
        )
        .unwrap();
        let mut names = events.into_inner().unwrap();
        names.sort();
        assert_eq!(
            names,
            vec![
                "file: a.txt".to_string(),
                "file: b.txt".to_string(),
                "git: no uncommitted changes".to_string()
            ]
        );
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
