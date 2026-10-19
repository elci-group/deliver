//! Keep spec-selected paths within the declared base directory.
//!
//! This rejects lexical escapes and symlinks to outside the base. It is a
//! validation guard, not an OS sandbox: a concurrently changed path can race
//! the check, and commands can still access the rest of the runner.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "path must stay inside --base: {}",
            relative.display()
        ));
    }

    let root = base
        .canonicalize()
        .map_err(|error| format!("cannot resolve --base {}: {error}", base.display()))?;
    if !root.is_dir() {
        return Err(format!("--base is not a directory: {}", root.display()));
    }

    let mut current = root.clone();
    for part in relative.components() {
        if let Component::Normal(name) = part {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(_) => {
                    current = current.canonicalize().map_err(|error| {
                        format!("cannot resolve path {}: {error}", current.display())
                    })?;
                    if !current.starts_with(&root) {
                        return Err(format!(
                            "path escapes --base through a symlink: {}",
                            relative.display()
                        ));
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "cannot inspect path {}: {error}",
                        current.display()
                    ))
                }
            }
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn accepts_existing_and_missing_paths_beneath_base() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        assert_eq!(
            resolve(dir.path(), Path::new("nested/soon.txt")).unwrap(),
            dir.path().canonicalize().unwrap().join("nested/soon.txt")
        );
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        let dir = TempDir::new().unwrap();
        assert!(resolve(dir.path(), Path::new("../outside")).is_err());
        assert!(resolve(dir.path(), dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_to_outside_base() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(resolve(dir.path(), Path::new("link/secret.txt")).is_err());
    }
}
