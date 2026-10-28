use crate::{CheckResult, FileCheck};
use jsonschema::JSONSchema;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Validate one file against its spec: resolves the path under `--base`
/// (path guard enforced) and applies every configured requirement in a
/// fixed order, returning the first failure or a passing result.
pub fn validate_file(check: &FileCheck, base: &Path) -> CheckResult {
    let name = format!("file: {}", check.path);
    let path = match crate::path_guard::resolve(base, Path::new(&check.path)) {
        Ok(path) => path,
        Err(message) => {
            return CheckResult {
                skipped: false,
                name,
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message,
            }
        }
    };

    if !path.exists() {
        return missing_file(check, name, &path);
    }

    // Check if path is a symlink
    if check.forbid_symlinks {
        if let Ok(metadata) = fs::symlink_metadata(base.join(&check.path)) {
            if metadata.file_type().is_symlink() {
                return CheckResult {
            skipped: false,
                    name,
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "path is a symlink: {}. Suggestion: disable forbid_symlinks or use the target file.",
                        path.display()
                    ),
                };
            }
        }
    }

    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return CheckResult {
                skipped: false,
                name,
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!("cannot read metadata: {}", error),
            }
        }
    };

    if metadata.is_dir() {
        return CheckResult {
            skipped: false,
            name,
            pass: false,
            kind: "file".to_string(),
            weight: check.weight,
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
                skipped: false,
                name,
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!("cannot read file: {}", error),
            }
        }
    };

    if let Some(result) = validate_encoding(check, &name, &content) {
        return result;
    }

    if let Some(result) = validate_hashes(check, &name, &path) {
        return result;
    }

    if let Some(result) = validate_license_header(check, &name, &content) {
        return result;
    }

    if let Some(result) = validate_json_schema(check, &name, &content) {
        return result;
    }

    if let Some(result) = validate_toml_keys(check, &name, &content) {
        return result;
    }

    if let Some(result) = validate_json_keys(check, &name, &content) {
        return result;
    }

    if let Some(result) = validate_yaml_schema(check, &name, &content) {
        return result;
    }

    if check.check_references {
        if let Some(result) = validate_references(check, &name, &content, base) {
            return result;
        }
    }

    if let Some(result) = validate_line_count(check, &name, &content) {
        return result;
    }
    if let Some(result) = validate_patterns(check, &name, &content) {
        return result;
    }

    if check.forbid_secrets {
        if let Some(result) = validate_secrets(check, &name, &content) {
            return result;
        }
    }

    CheckResult {
        skipped: false,
        name,
        pass: true,
        kind: "file".to_string(),
        weight: check.weight,
        message: format!("OK ({} bytes)", metadata.len()),
    }
}

/// Built-in, high-confidence secret patterns: known credential prefixes and
/// key-material markers only. Deliberately conservative — a secret gate that
/// cries wolf gets disabled. Reports name the pattern and line, never the
/// matched content.
const SECRET_PATTERNS: [(&str, &str); 10] = [
    ("aws-access-key", r"AKIA[0-9A-Z]{16}"),
    ("github-token", r"ghp_[A-Za-z0-9]{36}"),
    ("github-fine-grained-token", r"github_pat_[A-Za-z0-9_]{82}"),
    ("gitlab-token", r"glpat-[A-Za-z0-9\-_]{20,}"),
    ("slack-token", r"xox[baprs]-[A-Za-z0-9\-]{10,}"),
    ("stripe-key", r"(sk|rk)_(live|test)_[A-Za-z0-9]{20,}"),
    ("google-api-key", r"AIza[0-9A-Za-z_\-]{35}"),
    (
        "private-key-block",
        r"-----BEGIN (RSA |EC |OPENSSH |PGP |DSA )?PRIVATE KEY( BLOCK)?-----",
    ),
    (
        "jwt",
        r"eyJ[A-Za-z0-9_\-]{15,}\.eyJ[A-Za-z0-9_\-]{15,}\.[A-Za-z0-9_\-]{15,}",
    ),
    (
        "assigned-secret",
        r#"(?i)\b(password|passwd|secret|api_key|apikey|token)\b["']?\s*[:=]\s*["'][^"']{8,}["']"#,
    ),
];

fn validate_secrets(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    let fail = |message: String| {
        Some(CheckResult {
            skipped: false,
            name: name.to_string(),
            pass: false,
            kind: "file".to_string(),
            weight: check.weight,
            message,
        })
    };

    let mut compiled: Vec<(&str, Regex)> = Vec::with_capacity(SECRET_PATTERNS.len());
    for (label, pattern) in SECRET_PATTERNS {
        match Regex::new(pattern) {
            Ok(regex) => compiled.push((label, regex)),
            Err(error) => {
                return fail(format!(
                    "internal error: built-in secret pattern '{}' is invalid: {}",
                    label, error
                ));
            }
        }
    }

    for (label, regex) in &compiled {
        if let Some(mat) = regex.find(content) {
            let line = content[..mat.start()].matches('\n').count() + 1;
            return fail(format!(
                "matched secret pattern \"{}\" on line {}. Suggestion: remove the secret, \
                 rotate the credential, and purge it from repository history.",
                label, line
            ));
        }
    }
    None
}

fn missing_file(check: &FileCheck, name: String, path: &Path) -> CheckResult {
    if check.required {
        CheckResult {
            skipped: false,
            name,
            pass: false,
            kind: "file".to_string(),
            weight: check.weight,
            message: format!(
                "required file does not exist: {}. Suggestion: create the file or remove this check.",
                path.display()
            ),
        }
    } else {
        CheckResult {
            skipped: false,
            name,
            pass: true,
            kind: "file".to_string(),
            weight: check.weight,
            message: "optional file missing, ignored; set required=true if it must exist"
                .to_string(),
        }
    }
}

fn validate_size(check: &FileCheck, name: &str, size: u64) -> Option<CheckResult> {
    if let Some(max) = check.max_size_bytes {
        if size > max {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "size {} bytes exceeds max {} bytes. Suggestion: reduce file size or increase max_size_bytes.",
                    size, max
                ),
            });
        }
    }
    if let Some(min) = check.min_size_bytes {
        if size < min {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "size {} bytes is below min {} bytes. Suggestion: add more content or reduce min_size_bytes.",
                    size, min
                ),
            });
        }
    }
    None
}

fn validate_encoding(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if let Some(encoding) = &check.encoding {
        let valid = match encoding.to_lowercase().as_str() {
            "utf-8" | "utf8" => {
                // Check if content is valid UTF-8 (it should be since we read it as string)
                content.is_ascii() || !content.contains('\u{FFFD}')
            }
            "ascii" => content.is_ascii(),
            _ => {
                // Unknown encoding, skip validation
                return None;
            }
        };

        if !valid {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "file does not match encoding '{}'. Suggestion: convert file to {} encoding or adjust the encoding requirement.",
                    encoding, encoding
                ),
            });
        }
    }
    None
}

fn validate_hashes(check: &FileCheck, name: &str, path: &Path) -> Option<CheckResult> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Some(CheckResult {
                skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!("cannot read file for hash verification: {}", error),
            });
        }
    };

    if let Some(expected_sha256) = &check.sha256 {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual_sha256 = format!("{:x}", hasher.finalize());

        if actual_sha256 != *expected_sha256 {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "SHA-256 hash mismatch: expected {}, got {}. Suggestion: update the file or the expected hash.",
                    expected_sha256, actual_sha256
                ),
            });
        }
    }

    if let Some(expected_blake3) = &check.blake3 {
        let actual_blake3 = blake3::hash(&bytes).to_hex().to_string();

        if actual_blake3 != *expected_blake3 {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "BLAKE3 hash mismatch: expected {}, got {}. Suggestion: update the file or the expected hash.",
                    expected_blake3, actual_blake3
                ),
            });
        }
    }

    None
}

fn validate_license_header(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if let Some(expected_header) = &check.require_license_header {
        let first_lines: Vec<&str> = content.lines().take(10).collect();
        let header = first_lines.join("\n");

        if !header.contains(expected_header) {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "file does not contain required license header '{}'. Suggestion: add the license header to the file or adjust the requirement.",
                    expected_header
                ),
            });
        }
    }
    None
}

fn validate_json_schema(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if let Some(schema_str) = &check.json_schema {
        // Parse the schema
        let schema_value: serde_json::Value = match serde_json::from_str(schema_str) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
                    skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "invalid JSON schema: {}. Suggestion: fix the schema syntax in the spec.",
                        error
                    ),
                });
            }
        };

        // Compile the schema
        let schema = match JSONSchema::compile(&schema_value) {
            Ok(s) => s,
            Err(error) => {
                return Some(CheckResult {
                    skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "cannot compile JSON schema: {}. Suggestion: fix the schema structure.",
                        error
                    ),
                });
            }
        };

        // Parse the file content as JSON
        let content_value: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "file is not valid JSON: {}. Suggestion: fix the JSON syntax or remove json_schema check.",
                        error
                    ),
                });
            }
        };

        // Validate against schema
        let result = schema.validate(&content_value);
        if let Err(errors) = result {
            let error_messages: Vec<String> = errors
                .map(|e| format!("{}: {}", e.instance_path, e))
                .collect();
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "JSON schema validation failed: {}. Suggestion: fix the JSON structure to match the schema.",
                    error_messages.join("; ")
                ),
            });
        }
    }
    None
}

fn validate_toml_keys(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if !check.require_toml_keys.is_empty() {
        let toml_value: toml::Value = match toml::from_str(content) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "file is not valid TOML: {}. Suggestion: fix the TOML syntax or remove require_toml_keys check.",
                        error
                    ),
                });
            }
        };

        let missing_keys: Vec<String> = check
            .require_toml_keys
            .iter()
            .filter(|key| !has_toml_key(&toml_value, key))
            .cloned()
            .collect();

        if !missing_keys.is_empty() {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "TOML file missing required keys: {}. Suggestion: add these keys to the TOML file or remove them from require_toml_keys.",
                    missing_keys.join(", ")
                ),
            });
        }
    }
    None
}

fn has_toml_key(value: &toml::Value, key_path: &str) -> bool {
    let parts: Vec<&str> = key_path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            toml::Value::Table(table) => {
                if let Some(next) = table.get(part) {
                    current = next;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn validate_json_keys(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if !check.require_json_keys.is_empty() {
        let json_value: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "file is not valid JSON: {}. Suggestion: fix the JSON syntax or remove require_json_keys check.",
                        error
                    ),
                });
            }
        };

        let missing_keys: Vec<String> = check
            .require_json_keys
            .iter()
            .filter(|key| !has_json_key(&json_value, key))
            .cloned()
            .collect();

        if !missing_keys.is_empty() {
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "JSON file missing required keys: {}. Suggestion: add these keys to the JSON file or remove them from require_json_keys.",
                    missing_keys.join(", ")
                ),
            });
        }
    }
    None
}

fn has_json_key(value: &serde_json::Value, key_path: &str) -> bool {
    let parts: Vec<&str> = key_path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            serde_json::Value::Object(map) => {
                if let Some(next) = map.get(part) {
                    current = next;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn validate_yaml_schema(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    if let Some(schema_str) = &check.yaml_schema {
        // Parse the schema (YAML or JSON)
        let schema_value: serde_json::Value = match serde_yaml::from_str(schema_str) {
            Ok(v) => v,
            Err(_) => {
                // Try JSON parsing if YAML fails
                match serde_json::from_str(schema_str) {
                    Ok(v) => v,
                    Err(error) => {
                        return Some(CheckResult {
            skipped: false,
                            name: name.to_string(),
                            pass: false,
                            kind: "file".to_string(),
                            weight: check.weight,
                            message: format!(
                                "invalid YAML/JSON schema: {}. Suggestion: fix the schema syntax in the spec.",
                                error
                            ),
                        });
                    }
                }
            }
        };

        // Compile the schema
        let schema = match JSONSchema::compile(&schema_value) {
            Ok(s) => s,
            Err(error) => {
                return Some(CheckResult {
                    skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "cannot compile YAML schema: {}. Suggestion: fix the schema structure.",
                        error
                    ),
                });
            }
        };

        // Parse the file content as YAML
        let yaml_value: serde_yaml::Value = match serde_yaml::from_str(content) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "file is not valid YAML: {}. Suggestion: fix the YAML syntax or remove yaml_schema check.",
                        error
                    ),
                });
            }
        };

        // Convert YAML to JSON for validation
        let json_value: serde_json::Value = match serde_yaml::from_value(yaml_value) {
            Ok(v) => v,
            Err(error) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "cannot convert YAML to JSON: {}. Suggestion: check for unsupported YAML types.",
                        error
                    ),
                });
            }
        };

        // Validate against schema
        let result = schema.validate(&json_value);
        if let Err(errors) = result {
            let error_messages: Vec<String> = errors
                .map(|e| format!("{}: {}", e.instance_path, e))
                .collect();
            return Some(CheckResult {
            skipped: false,
                name: name.to_string(),
                pass: false,
                kind: "file".to_string(),
                weight: check.weight,
                message: format!(
                    "YAML schema validation failed: {}. Suggestion: fix the YAML structure to match the schema.",
                    error_messages.join("; ")
                ),
            });
        }
    }
    None
}

fn validate_references(
    check: &FileCheck,
    name: &str,
    content: &str,
    base: &Path,
) -> Option<CheckResult> {
    let file_ext = check.path.split('.').next_back().unwrap_or("");
    let mut missing_refs = Vec::new();

    // Detect references based on file type
    match file_ext {
        "rs" => {
            // Rust: mod, use, include!, include_str!, include_bytes!
            let mod_pattern = Regex::new(r"mod\s+([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
            for cap in mod_pattern.captures_iter(content) {
                let module_name = &cap[1];
                let possible_paths = [
                    format!("{}.rs", module_name),
                    format!("{}/mod.rs", module_name),
                    format!("{}/{}.rs", module_name, module_name),
                ];
                let found = possible_paths.iter().any(|p| base.join(p).exists());
                if !found {
                    missing_refs.push(format!("module {}", module_name));
                }
            }

            let include_pattern = Regex::new(r#"include_?[a-z]*!\s*\(["']([^"']+)["']\)"#).unwrap();
            for cap in include_pattern.captures_iter(content) {
                let ref_path = &cap[1];
                if !base.join(ref_path).exists() {
                    missing_refs.push(format!("include file {}", ref_path));
                }
            }
        }
        "py" => {
            // Python: import, from ... import
            let import_pattern = Regex::new(r"from\s+([a-zA-Z_][a-zA-Z0-9_.]*)\s+import").unwrap();
            for cap in import_pattern.captures_iter(content) {
                let module_path = &cap[1].replace('.', "/");
                let possible_paths = [
                    format!("{}.py", module_path),
                    format!("{}/__init__.py", module_path),
                ];
                let found = possible_paths.iter().any(|p| base.join(p).exists());
                if !found {
                    missing_refs.push(format!("Python module {}", &cap[1]));
                }
            }
        }
        "js" | "ts" | "jsx" | "tsx" => {
            // JavaScript/TypeScript: import, require
            let import_pattern = Regex::new(r#"from\s+["']([^"']+)["']"#).unwrap();
            for cap in import_pattern.captures_iter(content) {
                let ref_path = &cap[1];
                // Skip node_modules and absolute paths
                if !ref_path.starts_with('.') && !ref_path.starts_with('/') {
                    continue;
                }
                if !base.join(ref_path).exists() {
                    missing_refs.push(format!("import {}", ref_path));
                }
            }
        }
        "c" | "cpp" | "h" | "hpp" => {
            // C/C++: #include
            let include_pattern = Regex::new(r#"#include\s*[<"]([^>"]+)[>"]"#).unwrap();
            for cap in include_pattern.captures_iter(content) {
                let ref_path = &cap[1];
                // Skip system headers (angled brackets)
                if content.contains(&format!("#include <{}>", ref_path)) {
                    continue;
                }
                if !base.join(ref_path).exists() {
                    missing_refs.push(format!("include file {}", ref_path));
                }
            }
        }
        _ => {}
    }

    if !missing_refs.is_empty() {
        return Some(CheckResult {
            skipped: false,
            name: name.to_string(),
            pass: false,
            kind: "file".to_string(),
            weight: check.weight,
            message: format!(
                "file references missing files: {}. Suggestion: create the referenced files or remove the references.",
                missing_refs.join(", ")
            ),
        });
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
            skipped: false,
        name: name.to_string(),
        pass: false,
        kind: "file".to_string(),
        weight: check.weight,
        message: format!(
            "file has {} lines, required {}. Suggestion: add more lines or reduce require_line_count.",
            lines, min_lines
        ),
    })
}

fn validate_patterns(check: &FileCheck, name: &str, content: &str) -> Option<CheckResult> {
    for pattern in &check.forbid_regex {
        match Regex::new(pattern) {
            Ok(regex) if regex.is_match(content) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "content matches forbidden regex: '{}'. Suggestion: remove the matching content or adjust the regex.",
                        pattern
                    ),
                });
            }
            Ok(_) => {}
            Err(error) => {
                return Some(CheckResult {
                    skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "invalid forbid regex '{}': {}. Suggestion: fix the regex syntax.",
                        pattern, error
                    ),
                });
            }
        }
    }

    for pattern in &check.require_regex {
        match Regex::new(pattern) {
            Ok(regex) if !regex.is_match(content) => {
                return Some(CheckResult {
            skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "content does not match required regex: '{}'. Suggestion: add matching content or adjust the regex.",
                        pattern
                    ),
                });
            }
            Ok(_) => {}
            Err(error) => {
                return Some(CheckResult {
                    skipped: false,
                    name: name.to_string(),
                    pass: false,
                    kind: "file".to_string(),
                    weight: check.weight,
                    message: format!(
                        "invalid require regex '{}': {}. Suggestion: fix the regex syntax.",
                        pattern, error
                    ),
                });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileCheck;
    use tempfile::TempDir;

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
            forbid_secrets: false,
            weight: None,
            when: None,
        }
    }

    fn write_and_run(dir: &TempDir, content: &str) -> CheckResult {
        std::fs::write(dir.path().join("a.txt"), content).unwrap();
        let check = FileCheck {
            forbid_secrets: true,
            ..base_check("a.txt")
        };
        validate_file(&check, dir.path())
    }

    #[test]
    fn all_builtin_secret_patterns_compile() {
        for (label, pattern) in SECRET_PATTERNS {
            Regex::new(pattern).unwrap_or_else(|e| panic!("pattern {} invalid: {}", label, e));
        }
    }

    #[test]
    fn secret_detection_fails_with_pattern_name_and_line_but_not_content() {
        let dir = TempDir::new().unwrap();
        let content = "let name = \"harmless\";\nlet key = \"AKIAABCDEFGHIJKLMNOP\";\n";
        let result = write_and_run(&dir, content);
        assert!(!result.pass);
        assert!(
            result.message.contains("aws-access-key"),
            "{}",
            result.message
        );
        assert!(result.message.contains("line 2"), "{}", result.message);
        assert!(
            !result.message.contains("AKIAABCDEFGHIJKLMNOP"),
            "the matched secret value must never appear in the report"
        );
    }

    #[test]
    fn each_secret_pattern_family_is_detected() {
        let cases = [
            (
                "github-token",
                "token: ghp_abcdef0123456789abcdef0123456789abcdef01",
            ),
            (
                "github-fine-grained-token",
                &format!("github_pat_{}", "A1_".repeat(28)),
            ),
            ("gitlab-token", "glpat-AbCdEf1234567890-_abc"),
            ("slack-token", "xoxb-123456789012-abcdef"),
            // Assembled at runtime: a literal fixture in Stripe's live-key
            // format trips GitHub push protection, though it is synthetic.
            (
                "stripe-key",
                &format!("sk_{}_{}", "live", "a1b2c3d4e5f6g7h8i9j0k1l2"),
            ),
            ("google-api-key", "AIzaSyA1234567890abcdefghijklmnopqrstuv"),
            ("private-key-block", "-----BEGIN RSA PRIVATE KEY-----"),
            (
                "jwt",
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJ",
            ),
            ("assigned-secret", "password = \"hunter2hunter2\""),
        ];
        for (label, content) in cases {
            let dir = TempDir::new().unwrap();
            let result = write_and_run(&dir, content);
            assert!(
                !result.pass,
                "content for {} must be detected: {}",
                label, result.message
            );
            assert!(
                result.message.contains(label),
                "{} message must name the pattern: {}",
                label,
                result.message
            );
        }
    }

    #[test]
    fn innocuous_content_passes_secret_scan() {
        let dir = TempDir::new().unwrap();
        let content =
            "token_count = 42\npassword_hashing_algorithm = \"argon2\"\nAKIAs are not keys\n";
        let result = write_and_run(&dir, content);
        assert!(result.pass, "{}", result.message);
    }

    #[test]
    fn forbid_secrets_disabled_by_default() {
        let dir = TempDir::new().unwrap();
        let check = base_check("a.txt");
        std::fs::write(dir.path().join("a.txt"), "AKIAABCDEFGHIJKLMNOP").unwrap();
        let result = validate_file(&check, dir.path());
        assert!(result.pass);
    }
}
