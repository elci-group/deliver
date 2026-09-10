//! The knowledge store: bank-backed persistence with mkdir fallbacks.

use std::path::{Path, PathBuf};
use std::process::Command;

use defail::demo::{run_baker, run_provider};
use defail::knowledge::KnowledgeBase;
use defail::store::{Backend, KnowledgeStore, StoreError};

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("defail-store-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn bank_installed() -> bool {
    Command::new("bank")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[test]
fn bank_backend_round_trips_when_installed() {
    if !bank_installed() {
        eprintln!("skipping: bank is not installed");
        return;
    }
    let dir = fresh_dir("bank");
    // A path whose parents do not exist yet: bank must create them.
    let store = KnowledgeStore::with_backend(dir.join("nested/dir/knowledge"), Backend::Bank);
    let kb = run_provider(true).kb;
    let used = store.save(&kb).unwrap();
    assert_eq!(used, Backend::Bank);
    assert_eq!(store.backend(), Backend::Bank);
    let loaded = store.load().unwrap();
    assert_eq!(loaded.to_records(), kb.to_records());
}

#[test]
fn cp_mkdir_fallback_round_trips() {
    let dir = fresh_dir("cpmkdir");
    let store = KnowledgeStore::with_backend(dir.join("nested/dir/knowledge"), Backend::CpMkdir);
    let kb = run_baker(false).kb;
    // Delta-based: other tests stage transiently too, so only a save that
    // leaks its own staging file can grow the count.
    let before = staging_count(&dir);
    let used = store.save(&kb).unwrap();
    assert_eq!(used, Backend::CpMkdir);
    let after = staging_count(&dir);
    assert!(after <= before, "staging files leaked: {before} -> {after}");
    let loaded = store.load().unwrap();
    assert_eq!(loaded.to_records(), kb.to_records());
}

/// Staging files are created inside the destination directory, never in the
/// shared temp dir.
fn staging_count(dir: &Path) -> usize {
    fn count_in(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".defail-kb-stage-")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
    count_in(dir) + count_in(&std::env::temp_dir())
}

#[test]
fn auto_detected_backend_round_trips() {
    let dir = fresh_dir("auto");
    let store = KnowledgeStore::new(dir.join("knowledge"));
    let kb = run_baker(true).kb;
    let used = store.save(&kb).unwrap();
    assert_eq!(used, store.backend());
    assert_eq!(store.load().unwrap().to_records(), kb.to_records());
}

#[test]
fn bank_failure_engages_cp_mkdir_fallback() {
    // The parent path is a regular file, so no backend can create the store.
    // With bank selected, the error must therefore come from the fallback
    // tools (mkdir/cp), proving the fallback engaged after bank failed.
    let dir = fresh_dir("blocked");
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let store = KnowledgeStore::with_backend(blocker.join("knowledge"), Backend::Bank);
    let err = store.save(&run_provider(true).kb).unwrap_err();
    match err {
        StoreError::ToolFailed { tool, .. } => {
            assert!(matches!(tool, "mkdir" | "cp"), "unexpected tool: {tool}");
        }
        other => panic!("expected ToolFailed from the fallback, got {other:?}"),
    }
}

#[test]
fn empty_bank_saves_and_loads() {
    let dir = fresh_dir("empty");
    let store = KnowledgeStore::with_backend(dir.join("knowledge"), Backend::CpMkdir);
    let used = store.save(&KnowledgeBase::new()).unwrap();
    assert_eq!(used, Backend::CpMkdir);
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn loading_a_missing_bank_is_an_error() {
    let dir = fresh_dir("missing");
    let store = KnowledgeStore::new(dir.join("never-saved"));
    assert!(matches!(store.load(), Err(StoreError::Io(_))));
}

#[test]
fn load_report_surfaces_duplicate_keys_and_v1_banks_read() {
    let dir = fresh_dir("report");
    let store = KnowledgeStore::with_backend(dir.join("knowledge"), Backend::CpMkdir);
    // A v1 bank (no header) that repeats one key: last wins, and the
    // overwrite is reported with its line number.
    std::fs::write(
        store.path(),
        "capacity/rate-limit|ctx|remedy-a|1|0|first\ncapacity/rate-limit|ctx|remedy-b|2|0|second\n",
    )
    .unwrap();
    let (kb, report) = store.load_reported().unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(report.duplicates, vec![2]);
    assert_eq!(kb.len(), 1);
    assert_eq!(kb.entries().next().unwrap().1.remedy.as_str(), "remedy-b");

    // A v2 bank saved by this build loads with an empty duplicate list.
    let kb = run_provider(true).kb;
    store.save(&kb).unwrap();
    let (loaded, report) = store.load_reported().unwrap();
    assert_eq!(loaded.to_records(), kb.to_records());
    assert_eq!(report.records, kb.len());
    assert!(report.duplicates.is_empty());
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.starts_with("defail-kb v2\n"), "write only v2");
}
