//! Persistence for the knowledge base.
//!
//! Path creation prefers the `bank` utility (mkdir + touch in one step).
//! When bank is not installed, or its invocation fails for any reason, the
//! store falls back to plain `mkdir -p` for the directory. The content
//! itself is always written the same way: staged into a temporary file in
//! the destination directory (created with `create_new(true)`, so there is
//! no predictable-name symlink surface), fsynced, then atomically renamed
//! onto the target while the previous bank stays intact, and the directory
//! is fsynced after the rename. Both backends produce identical on-disk
//! records; [`KnowledgeStore::save`] reports which backend actually wrote
//! the file.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::knowledge::{KbError, KnowledgeBase, LoadReport};

/// How the store creates its paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Primary: the `bank` utility creates parent directories and the file.
    Bank,
    /// Fallback: `mkdir -p` creates the directory.
    CpMkdir,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Backend::Bank => f.write_str("bank"),
            Backend::CpMkdir => f.write_str("cp+mkdir (fallback)"),
        }
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    /// An external tool ran and failed.
    ToolFailed {
        tool: &'static str,
        stderr: String,
    },
    /// The store file exists but its records are malformed.
    Load(KbError),
    /// The save failed and the leftover staging file could not be removed
    /// either; both failures are reported.
    CleanupFailed {
        save: Box<StoreError>,
        cleanup: std::io::Error,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(err) => write!(f, "i/o error: {err}"),
            StoreError::ToolFailed { tool, stderr } => {
                write!(f, "{tool} failed: {stderr}")
            }
            StoreError::Load(err) => write!(f, "knowledge records malformed: {err}"),
            StoreError::CleanupFailed { save, cleanup } => write!(
                f,
                "{save}; additionally the staging file could not be removed: {cleanup}"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(err) => Some(err),
            StoreError::Load(err) => Some(err),
            StoreError::CleanupFailed { save, .. } => Some(save),
            StoreError::ToolFailed { .. } => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}

impl From<KbError> for StoreError {
    fn from(err: KbError) -> Self {
        StoreError::Load(err)
    }
}

/// A knowledge bank on disk: a line-oriented record file managed through
/// `bank` when available, with `mkdir -p` as the path-creation fallback.
pub struct KnowledgeStore {
    path: PathBuf,
    backend: Backend,
}

static STAGING_SEQ: AtomicU32 = AtomicU32::new(0);

/// How many staging names to try before giving up on `create_new`.
const STAGING_ATTEMPTS: u32 = 64;

impl KnowledgeStore {
    /// Store at `path`, auto-detecting the backend: `bank` when the utility
    /// is installed, `mkdir -p` otherwise.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let backend = if bank_available() {
            Backend::Bank
        } else {
            Backend::CpMkdir
        };
        Self {
            path: path.into(),
            backend,
        }
    }

    pub fn with_backend(path: impl Into<PathBuf>, backend: Backend) -> Self {
        Self {
            path: path.into(),
            backend,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// True when the `bank` utility runs successfully on this machine.
    pub fn bank_available() -> bool {
        bank_available()
    }

    /// Persist the knowledge base, returning the backend that wrote the file.
    ///
    /// With [`Backend::Bank`], `bank -p -f` creates the parent directories
    /// and the (empty) file, and the records are then staged and atomically
    /// renamed into place. If bank fails for any reason — missing from
    /// `PATH`, permissions, a blocked parent — the store falls back to
    /// `mkdir -p` plus the same atomic staging rename, deterministically.
    /// A failure mid-save leaves the previously saved bank untouched.
    pub fn save(&self, kb: &KnowledgeBase) -> Result<Backend, StoreError> {
        let body = render(kb);
        match self.backend {
            Backend::Bank => match self.save_with_bank(&body) {
                Ok(()) => Ok(Backend::Bank),
                Err(_) => {
                    self.save_with_cp_mkdir(&body)?;
                    Ok(Backend::CpMkdir)
                }
            },
            Backend::CpMkdir => {
                self.save_with_cp_mkdir(&body)?;
                Ok(Backend::CpMkdir)
            }
        }
    }

    /// Load the knowledge bank from disk. A missing file is an error, not an
    /// empty bank: callers should know their knowledge was never persisted.
    pub fn load(&self) -> Result<KnowledgeBase, StoreError> {
        Ok(self.load_reported()?.0)
    }

    /// Like [`KnowledgeStore::load`], but also returns the [`LoadReport`]
    /// (record count and duplicate-key line numbers).
    pub fn load_reported(&self) -> Result<(KnowledgeBase, LoadReport), StoreError> {
        let text = std::fs::read_to_string(&self.path)?;
        let mut kb = KnowledgeBase::new();
        let report = kb.load_records(text.lines().map(str::to_string))?;
        Ok((kb, report))
    }

    fn save_with_bank(&self, body: &str) -> Result<(), StoreError> {
        run("bank", &["-p", "-f", &self.path.to_string_lossy()])?;
        atomic_replace(&self.path, body)
    }

    fn save_with_cp_mkdir(&self, body: &str) -> Result<(), StoreError> {
        if let Some(parent) = parent_dir(&self.path) {
            run("mkdir", &["-p", &parent.to_string_lossy()])?;
        }
        atomic_replace(&self.path, body)
    }
}

/// Write `body` to a fresh staging file inside `target`'s directory, fsync
/// it, then atomically rename it onto `target` and fsync the directory, so
/// the previous bank survives any failure before the rename. Staging files
/// are created with `create_new(true)`: a name collision is an error to
/// retry, never an existing file to follow or truncate.
fn atomic_replace(target: &Path, body: &str) -> Result<(), StoreError> {
    let staging = stage(target, body)?;
    match std::fs::rename(&staging, target) {
        Ok(()) => {
            sync_parent_dir(target)?;
            Ok(())
        }
        Err(err) => {
            let save = StoreError::Io(err);
            match std::fs::remove_file(&staging) {
                Ok(()) => Err(save),
                Err(cleanup) => Err(StoreError::CleanupFailed {
                    save: Box::new(save),
                    cleanup,
                }),
            }
        }
    }
}

/// The directory a staging file lives in: the target's parent, or the
/// current directory for bare filenames.
fn staging_dir(target: &Path) -> PathBuf {
    parent_dir(target)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn stage(target: &Path, body: &str) -> Result<PathBuf, StoreError> {
    let dir = staging_dir(target);
    for _ in 0..STAGING_ATTEMPTS {
        let name = format!(
            ".defail-kb-stage-{}-{}",
            std::process::id(),
            STAGING_SEQ.fetch_add(1, Ordering::SeqCst)
        );
        let staging = dir.join(name);
        let attempt = OpenOptions::new().write(true).create_new(true).open(&staging);
        match attempt {
            Ok(mut file) => {
                file.write_all(body.as_bytes())?;
                file.sync_data()?;
                return Ok(staging);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique staging file name",
    )
    .into())
}

fn sync_parent_dir(target: &Path) -> Result<(), StoreError> {
    File::open(staging_dir(target))?.sync_all()?;
    Ok(())
}

fn render(kb: &KnowledgeBase) -> String {
    let records = kb.to_records();
    format!("{}\n", records.join("\n"))
}

/// The meaningful parent of a path: `None` for bare filenames and roots.
fn parent_dir(path: &Path) -> Option<&Path> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent),
        _ => None,
    }
}

fn bank_available() -> bool {
    Command::new("bank")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn run(tool: &'static str, args: &[&str]) -> Result<(), StoreError> {
    match Command::new(tool).args(args).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(StoreError::ToolFailed {
            tool,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
        Err(err) => Err(StoreError::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    fn fresh_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("defail-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bank_with(tag: &str, remedy: &str) -> KnowledgeBase {
        let mut kb = KnowledgeBase::new();
        kb.learn(
            crate::knowledge::KbKey {
                class: crate::declare::FailureClass::new("capacity/rate-limit"),
                context: crate::knowledge::ContextSig(format!("test:{tag}")),
            },
            crate::declare::RemedyId::new(remedy),
            "verified".into(),
            true,
        );
        kb
    }

    fn staging_leak_count(dir: &Path) -> usize {
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

    #[test]
    fn mid_save_failure_preserves_the_previous_bank() {
        // Inject a failing staging step: pre-create every staging name the
        // store will try, so create_new(true) keeps colliding. The save must
        // fail (no symlink is followed, no predictable name is truncated) and
        // the bank written by the earlier successful save must survive intact.
        let dir = fresh_dir("collision");
        let target = dir.join("knowledge");
        let store = KnowledgeStore::with_backend(&target, Backend::CpMkdir);
        let original = bank_with("collision", "remedy-a");
        store.save(&original).unwrap();

        for seq in 0..(STAGING_ATTEMPTS + 256) {
            std::fs::write(
                dir.join(format!(".defail-kb-stage-{}-{seq}", std::process::id())),
                b"occupied",
            )
            .unwrap();
        }
        let err = store.save(&bank_with("collision", "remedy-b"));
        assert!(matches!(err, Err(StoreError::Io(_))), "got {err:?}");

        let text = std::fs::read_to_string(&target).unwrap();
        assert_eq!(text, render(&original), "previous bank must survive");
        assert_eq!(store.load().unwrap().to_records(), original.to_records());
    }

    #[test]
    fn rename_failure_is_reported_and_staging_is_cleaned_up() {
        // The target exists as a directory, so the staging rename must fail;
        // the error must be surfaced and the staging file removed, never
        // left behind silently.
        let dir = fresh_dir("rename-fails");
        let target = dir.join("knowledge");
        std::fs::create_dir(&target).unwrap();
        let store = KnowledgeStore::with_backend(&target, Backend::CpMkdir);
        let err = store.save(&bank_with("rename", "remedy-a"));
        assert!(matches!(err, Err(StoreError::Io(_))), "got {err:?}");
        assert_eq!(staging_leak_count(&dir), 0);
    }

    #[cfg(unix)]
    #[test]
    fn save_replaces_a_symlink_without_following_it() {
        // A symlinked target must be replaced, not written through: the file
        // the link points at keeps its content; the atomic rename unlinks
        // the symlink itself.
        if !symlinks_supported() {
            eprintln!("skipping: symlinks are not supported here");
            return;
        }
        let dir = fresh_dir("symlink");
        let real = dir.join("real-bank");
        let link = dir.join("link-bank");
        std::fs::write(&real, "pre-existing content").unwrap();
        symlink(&real, &link).unwrap();

        let store = KnowledgeStore::with_backend(&link, Backend::CpMkdir);
        store.save(&bank_with("symlink", "remedy-a")).unwrap();

        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "pre-existing content"
        );
        let via_link = std::fs::read_to_string(&link).unwrap();
        assert!(via_link.contains("defail-kb v2"));
        assert_eq!(staging_leak_count(&dir), 0);
    }

    #[cfg(unix)]
    fn symlinks_supported() -> bool {
        let dir = fresh_dir("symlink-probe");
        let probe = dir.join("probe");
        match symlink(dir.join("nowhere"), &probe) {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }
}
