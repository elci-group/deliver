use crate::{CheckResult, FileCheck};
use regex::Regex;
use std::fs;
use std::path::Path;

pub fn validate_file(check: &FileCheck, base: &Path) -> CheckResult {
    let path = base.join(&check.path);
    let name = format!("file: {}", check.path);

    if !path.exists() {
        return missing_file(check, name, &path);
    }

    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return CheckResult {
                name,
                pass: false,
                kind: "file".to_string(),
                message: format!("cannot read metadata: {}", error),
            }
        }
    };

    if metadata.is_dir() {
        return CheckResult {
            name,
            pass: false,
            kind: "file".to_string(),
            message: format!("path is a directory, not a file: {}", path.display()),
        };
    }

    if let Some(result) = validate_size(check, &name, metadata.len()) {
        return result;
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            return CheckResult {
                name,
                pass: false,
                kind: "file".to_string(),
                message: format!("cannot read file: {}", error),
            }
        }
    };

    if let Some(result) = validate_line_count(check, &name, &content) {
        return result;
    }
    if let Some(result) = validate_patterns(check, &name, &content) {
        return result;
    }

    CheckResult {
        name,
        pass: true,
        kind: "file".to_string(),
        message: format!("OK ({} bytes)", metadata.len()),
    }
}

fn missing_file(check: &FileCheck, name: String, path: &Path) -> CheckResult {
    if check.required {
        CheckResult {
            name,
            pass: false,
            kind: "file".to_string(),
            message: format!("required file does not exist: {}", path.display()),
        }
    } else {
        CheckResult {
            name,
            pass: true,
            kind: "file".to_string(),
            message: "optional file missing, ignored; set required=true if it must exist"
                .to_string(),
        }
    }
}

fn validate_size(check: &FileCheck, name: &str, size: u64) -> Option<CheckResult> {
    if let Some(max) = check.max_size_bytes {
        if size > max {
            return Some(CheckResult {
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                message: format!("size {} bytes exceeds max {} bytes", size, max),
            });
        }
    }
    if let Some(min) = check.min_size_bytes {
        if size < min {
            return Some(CheckResult {
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                message: format!(
                    "size {} bytes is below min {} bytes; add the expected content",
                    size, min
                ),
            });
        }
    }
    None
}

fn validate_line_count(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    let min_lines = check.require_line_count?;
    let lines = content.lines().count();
    if lines >= min_lines {
        return None;
    }

    Some(CheckResult {
        name: name.to_string(),
        pass: false,
        kind: "file".to_string(),
        message: format!("file has {} lines, required {}", lines, min_lines),
    })
}

fn validate_patterns(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    for pattern in &check.forbid_regex {
        match Regex::new(pattern) {
            Ok(regex) if regex.is_match(content) => {
                return Some(CheckResult {
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    message: format!("content matches forbidden regex: {}", pattern),
                });
            }
            Ok(_) => {}
            Err(error) => {
                return Some(CheckResult {
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    message: format!("invalid forbid regex '{}': {}", pattern, error),
                });
            }
        }
    }

    for pattern in &check.require_regex {
        match Regex::new(pattern) {
            Ok(regex) if !regex.is_match(content) => {
                return Some(CheckResult {
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    message: format!("content does not match required regex: {}", pattern),
                });
            }
            Ok(_) => {}
            Err(error) => {
                return Some(CheckResult {
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    message: format!("invalid require regex '{}': {}", pattern, error),
                });
            }
        }
    }

    None
}
