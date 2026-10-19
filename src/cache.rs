//! Content-hash-keyed result cache for file checks.
//!
//! A cache entry is only valid for the exact triple it was recorded under:
//! the deliver version, the full check definition (any spec change rewrites
//! the key), and the file's content hash. `--incremental` additionally pins
//! entries to the git HEAD they were validated at, so unchanged files can
//! replay without being re-read.

use crate::{CheckResult, FileCheck};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResult {
    pub content_hash: String,
    pub head: Option<String>,
    pub pass: bool,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Cache {
    pub path: PathBuf,
    entries: HashMap<String, Vec<CachedResult>>,
    touched: HashSet<String>,
}

impl Cache {
    /// Load the cache at `path`; a missing or corrupt file yields an empty
    /// cache (caching is an optimization, never a gate).
    pub fn load(path: PathBuf) -> Self {
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            touched: HashSet::new(),
        }
    }

    /// Key identifying the check definition: deliver version plus the
    /// serialized `FileCheck`, so any spec edit invalidates prior entries.
    pub fn check_key(check: &FileCheck) -> String {
        let json = serde_json::to_string(check).expect("FileCheck serializes");
        sha256_hex(
            [
                env!("CARGO_PKG_VERSION").as_bytes(),
                &[0],
                json.as_bytes(),
            ]
            .concat()
            .as_slice(),
        )
    }

    pub fn content_hash(content: &[u8]) -> String {
        sha256_hex(content)
    }

    /// Replay an entry validated at git commit `head` (used by
    /// `--incremental` for files unchanged since that commit).
    pub fn find_by_head(&mut self, key: &str, head: &str) -> Option<CachedResult> {
        let hit = self
            .entries
            .get(key)?
            .iter()
            .find(|entry| entry.head.as_deref() == Some(head))?
            .clone();
        self.touched.insert(key.to_string());
        Some(hit)
    }

    /// Replay an entry for identical file content, regardless of HEAD.
    pub fn find_by_content(&mut self, key: &str, content_hash: &str) -> Option<CachedResult> {
        let hit = self
            .entries
            .get(key)?
            .iter()
            .find(|entry| entry.content_hash == content_hash)?
            .clone();
        self.touched.insert(key.to_string());
        Some(hit)
    }

    pub fn insert(
        &mut self,
        key: &str,
        content_hash: String,
        head: Option<&str>,
        result: &CheckResult,
    ) {
        self.touched.insert(key.to_string());
        let bucket = self.entries.entry(key.to_string()).or_default();
        let entry = CachedResult {
            content_hash,
            head: head.map(str::to_string),
            pass: result.pass,
            message: result.message.clone(),
        };
        match bucket.iter_mut().find(|e| e.content_hash == entry.content_hash) {
            Some(existing) => *existing = entry,
            None => bucket.push(entry),
        }
    }

    /// Persist entries touched by this run; entries not touched are dropped,
    /// so the cache prunes itself without bookkeeping.
    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let pruned: HashMap<&String, &Vec<CachedResult>> = self
            .entries
            .iter()
            .filter(|(key, _)| self.touched.contains(*key))
            .collect();
        let file = fs::File::create(&self.path)?;
        serde_json::to_writer(file, &pruned).map_err(io::Error::other)
    }
}

/// `$XDG_CACHE_HOME/deliver` or `~/.cache/deliver`, when resolvable.
pub fn default_dir() -> Option<PathBuf> {
    default_dir_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

fn default_dir_from(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    match xdg {
        Some(xdg) if !xdg.is_empty() => Some(PathBuf::from(xdg).join("deliver")),
        _ => home.map(|home| PathBuf::from(home).join(".cache").join("deliver")),
    }
}

/// Cache file for a validated base directory inside `dir`: one JSON file per
/// canonicalized base path, so distinct checkouts never share entries.
pub fn cache_path(dir: &Path, base: &Path) -> PathBuf {
    let canonical = base
        .canonicalize()
        .unwrap_or_else(|_| base.to_path_buf());
    dir.join(sha256_hex(canonical.to_string_lossy().as_bytes()))
        .with_extension("json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn base_check(path: &str) -> FileCheck {
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
            weight: None,
        }
    }

    fn result(pass: bool, message: &str) -> CheckResult {
        CheckResult {
            name: "file: a.txt".to_string(),
            pass,
            kind: "file".to_string(),
            message: message.to_string(),
            weight: None,
            skipped: false,
        }
    }

    #[test]
    fn check_key_is_stable_and_sensitive_to_content_and_spec() {
        let check = base_check("a.txt");
        let key = Cache::check_key(&check);
        assert_eq!(key, Cache::check_key(&check));

        let other_file = base_check("b.txt");
        assert_ne!(key, Cache::check_key(&other_file));

        let other_spec = FileCheck {
            require_regex: vec!["x".to_string()],
            ..base_check("a.txt")
        };
        assert_ne!(key, Cache::check_key(&other_spec));
    }

    #[test]
    fn content_hash_changes_with_content() {
        assert_ne!(
            Cache::content_hash(b"hello"),
            Cache::content_hash(b"world")
        );
        assert_eq!(Cache::content_hash(b"hello"), Cache::content_hash(b"hello"));
    }

    #[test]
    fn insert_and_find_by_content_round_trips() {
        let mut cache = Cache::default();
        let key = "key";
        let outcome = result(true, "OK (5 bytes)");
        cache.insert(key, "hash-a".to_string(), None, &outcome);

        let hit = cache.find_by_content(key, "hash-a").unwrap();
        assert!(hit.pass);
        assert_eq!(hit.message, "OK (5 bytes)");
        assert!(cache.find_by_content(key, "hash-b").is_none());
    }

    #[test]
    fn find_by_head_matches_only_same_head() {
        let mut cache = Cache::default();
        cache.insert("key", "hash-a".to_string(), Some("abc123"), &result(true, "OK"));

        assert!(cache.find_by_head("key", "abc123").is_some());
        assert!(cache.find_by_head("key", "other").is_none());
        assert!(cache.find_by_head("missing", "abc123").is_none());
    }

    #[test]
    fn save_and_load_round_trip_and_prune_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let mut cache = Cache::load(path.clone());
        cache.insert("kept", "h1".to_string(), None, &result(true, "OK"));
        cache.insert("stale", "h2".to_string(), None, &result(true, "OK"));
        // Mark only "kept" as touched this run by looking it up.
        assert!(cache.find_by_content("kept", "h1").is_some());
        cache.save().unwrap();

        let mut reloaded = Cache::load(path);
        assert!(reloaded.find_by_content("kept", "h1").is_some());
        assert!(reloaded.find_by_content("stale", "h2").is_none());
    }

    #[test]
    fn load_tolerates_missing_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = Cache::load(dir.path().join("nope.json"));
        assert!(missing.entries.is_empty());

        let corrupt_path = dir.path().join("corrupt.json");
        fs::write(&corrupt_path, "{ not json").unwrap();
        let corrupt = Cache::load(corrupt_path);
        assert!(corrupt.entries.is_empty());
    }

    #[test]
    fn default_dir_prefers_xdg_then_home() {
        let xdg = Some(OsString::from("/tmp/xdg"));
        let home = Some(OsString::from("/tmp/home"));
        assert_eq!(
            default_dir_from(xdg.clone(), home.clone()),
            Some(PathBuf::from("/tmp/xdg/deliver"))
        );
        assert_eq!(
            default_dir_from(None, home),
            Some(PathBuf::from("/tmp/home/.cache/deliver"))
        );
        assert_eq!(default_dir_from(Some(OsString::new()), None), None);
        assert_eq!(default_dir_from(None, None), None);
    }

    #[test]
    fn cache_path_is_stable_and_distinct_per_base() {
        let dir = Path::new("/tmp/cache");
        let base_a = Path::new("/tmp/base-a");
        let base_b = Path::new("/tmp/base-b");
        assert_eq!(cache_path(dir, base_a), cache_path(dir, base_a));
        assert_ne!(cache_path(dir, base_a), cache_path(dir, base_b));
        assert_eq!(cache_path(dir, base_a).extension().unwrap(), "json");
    }
}
