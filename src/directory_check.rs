use crate::{CheckResult, DirectoryCheck};
use std::fs;
use std::path::Path;

pub fn validate_directory(check: &DirectoryCheck, base: &Path) -> CheckResult {
    let path = base.join(&check.path);
    let name = format!("directory: {}", check.path);

    if !path.exists() {
        return missing_directory(check, name, &path);
    }

    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return CheckResult {
                name,
                pass: false,
                kind: "directory".to_string(),
                message: format!("cannot read metadata: {}", error),
            }
        }
    };

    if !metadata.is_dir() {
        return CheckResult {
            name,
            pass: false,
            kind: "directory".to_string(),
            message: format!("path is a file, not a directory: {}", path.display()),
        };
    }

    let file_count = match count_files(&path) {
        Ok(count) => count,
        Err(error) => {
            return CheckResult {
                name,
                pass: false,
                kind: "directory".to_string(),
                message: format!("cannot count files: {}", error),
            }
        }
    };

    if let Some(result) = validate_file_count(check, &name, file_count) {
        return result;
    }

    if check.forbid_empty && file_count == 0 {
        return CheckResult {
            name,
            pass: false,
            kind: "directory".to_string(),
            message: "directory is empty. Suggestion: add files or disable forbid_empty."
                .to_string(),
        };
    }

    CheckResult {
        name,
        pass: true,
        kind: "directory".to_string(),
        message: format!("OK ({} files)", file_count),
    }
}

fn missing_directory(check: &DirectoryCheck, name: String, path: &Path) -> CheckResult {
    if check.required {
        CheckResult {
            name,
            pass: false,
            kind: "directory".to_string(),
            message: format!(
                "required directory does not exist: {}. Suggestion: create the directory or remove this check.",
                path.display()
            ),
        }
    } else {
        CheckResult {
            name,
            pass: true,
            kind: "directory".to_string(),
            message: "optional directory missing, ignored; set required=true if it must exist"
                .to_string(),
        }
    }
}

fn validate_file_count(check: &DirectoryCheck, name: &str, count: usize) -> Option<CheckResult> {
    if let Some(max) = check.max_files {
        if count > max {
            return Some(CheckResult {
                name: name.to_string(),
                pass: false,
                kind: "directory".to_string(),
                message: format!(
                    "directory has {} files, exceeds max {}. Suggestion: remove files or increase max_files.",
                    count, max
                ),
            });
        }
    }
    if let Some(min) = check.min_files {
        if count < min {
            return Some(CheckResult {
                name: name.to_string(),
                pass: false,
                kind: "directory".to_string(),
                message: format!(
                    "directory has {} files, below min {}. Suggestion: add files or reduce min_files.",
                    count, min
                ),
            });
        }
    }
    None
}

fn count_files(path: &Path) -> Result<usize, std::io::Error> {
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.path().is_file() {
            count += 1;
        }
    }
    Ok(count)
}
