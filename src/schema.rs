use serde_json::{json, Value};

/// JSON Schema (draft-07) describing the `deliver` spec format, kept by hand
/// in step with the `Spec`/`FileCheck`/`CommandCheck`/`DirectoryCheck`/`GitCheck`
/// structs in `lib.rs`. Lets editors and agents validate or autocomplete a
/// `deliver.toml`/`deliver.json` spec before ever running `deliver`.
pub fn spec_json_schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "deliver spec",
        "type": "object",
        "properties": {
            "extends": {
                "type": "string",
                "description": "Path to a parent spec to inherit from, resolved relative to this spec file. Checks accumulate; git flags combine with OR."
            },
            "file": { "type": "array", "items": { "$ref": "#/definitions/file_check" } },
            "files": { "type": "array", "items": { "$ref": "#/definitions/file_check" } },
            "command": { "type": "array", "items": { "$ref": "#/definitions/command_check" } },
            "commands": { "type": "array", "items": { "$ref": "#/definitions/command_check" } },
            "plugin": { "type": "array", "items": { "$ref": "#/definitions/plugin_check" } },
            "plugins": { "type": "array", "items": { "$ref": "#/definitions/plugin_check" } },
            "directory": { "type": "array", "items": { "$ref": "#/definitions/directory_check" } },
            "directories": { "type": "array", "items": { "$ref": "#/definitions/directory_check" } },
            "git": { "$ref": "#/definitions/git_check" },
            "sbom": { "$ref": "#/definitions/sbom_check" },
            "metadata": { "type": "object", "additionalProperties": { "type": "string" } }
        },
        "additionalProperties": false,
        "definitions": {
            "file_check": {
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "required": { "type": "boolean", "default": true },
                    "max_size_bytes": { "type": "integer", "minimum": 0 },
                    "min_size_bytes": { "type": "integer", "minimum": 0 },
                    "forbid_regex": { "type": "array", "items": { "type": "string" } },
                    "require_regex": { "type": "array", "items": { "type": "string" } },
                    "require_line_count": { "type": "integer", "minimum": 0 },
                    "encoding": { "type": "string", "enum": ["utf-8", "utf8", "ascii"] },
                    "forbid_symlinks": { "type": "boolean", "default": false },
                    "sha256": { "type": "string", "pattern": "^[0-9a-fA-F]{64}$" },
                    "blake3": { "type": "string", "pattern": "^[0-9a-fA-F]{64}$" },
                    "require_license_header": { "type": "string" },
                    "json_schema": { "type": "string", "description": "A JSON Schema document, as a string, validated against the file's parsed JSON content." },
                    "yaml_schema": { "type": "string", "description": "A YAML or JSON Schema document, as a string, validated against the file's parsed YAML content." },
                    "require_toml_keys": { "type": "array", "items": { "type": "string" } },
                    "require_json_keys": { "type": "array", "items": { "type": "string" } },
                    "check_references": {
                        "type": "boolean",
                        "default": false,
                        "description": "For .rs/.py/.js/.ts/.jsx/.tsx/.c/.cpp/.h/.hpp files, fail if a local mod/import/#include reference does not exist."
                    },
                    "forbid_secrets": {
                        "type": "boolean",
                        "default": false,
                        "description": "Fail when the file matches a built-in, high-confidence secret pattern (AWS keys, provider tokens, private key blocks, ...). Reports name the pattern and line, never the matched content."
                    },
                    "weight": { "type": "number", "description": "Relative importance in weighted completion reporting; unweighted checks count as 1.0." },
                    "when": { "$ref": "#/definitions/when_clause" }
                }
            },
            "command_check": {
                "type": "object",
                "required": ["name", "cmd"],
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "cmd": {
                        "anyOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "cwd": { "type": "string", "default": "." },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "expect_exit": { "type": "integer", "default": 0 },
                    "timeout_secs": { "type": "integer", "minimum": 0, "default": 300 },
                    "stdout_contains": { "type": "array", "items": { "type": "string" } },
                    "stderr_contains": { "type": "array", "items": { "type": "string" } },
                    "weight": { "type": "number", "description": "Relative importance in weighted completion reporting; unweighted checks count as 1.0." },
                    "when": { "$ref": "#/definitions/when_clause" }
                }
            },
            "directory_check": {
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "required": { "type": "boolean", "default": true },
                    "min_files": { "type": "integer", "minimum": 0 },
                    "max_files": { "type": "integer", "minimum": 0 },
                    "forbid_empty": { "type": "boolean", "default": false },
                    "weight": { "type": "number", "description": "Relative importance in weighted completion reporting; unweighted checks count as 1.0." },
                    "when": { "$ref": "#/definitions/when_clause" }
                }
            },
            "git_check": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "no_uncommitted_changes": { "type": "boolean", "default": false },
                    "no_untracked_files": { "type": "boolean", "default": false },
                    "require_branch_regex": { "type": "string" },
                    "require_commit_message_regex": { "type": "string" }
                }
            },
            "when_clause": {
                "type": "object",
                "additionalProperties": false,
                "description": "Skip conditions; the check is skipped rather than failed when any declared condition does not hold. Declared conditions combine with AND.",
                "properties": {
                    "file_exists": {
                        "type": "string",
                        "description": "Base-relative path that must exist for the check to run."
                    },
                    "os": {
                        "type": "string",
                        "description": "Operating system that must match the running one (linux, macos, or windows; case-insensitive)."
                    }
                }
            },
            "sbom_check": {
                "type": "object",
                "additionalProperties": false,
                "description": "SBOM presence and shape check: searches *.spdx, *.spdx.json, *.cdx.json, and bom.json under --base for SPDX or CycloneDX documents with their core fields present.",
                "properties": {
                    "required": {
                        "type": "boolean",
                        "default": true,
                        "description": "Fail when no SBOM is found. With false, absence passes but a malformed SBOM still fails."
                    },
                    "formats": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["spdx", "cyclonedx"] },
                        "description": "Accepted formats; empty accepts any supported format."
                    },
                    "weight": { "type": "number", "description": "Relative importance in weighted completion reporting; unweighted checks count as 1.0." }
                }
            },
            "plugin_check": {
                "type": "object",
                "required": ["name", "cmd"],
                "additionalProperties": false,
                "description": "Subprocess-protocol plugin check (v1): the runner writes {\"protocol\":1,\"name\":...,\"base\":...} to the plugin's stdin and expects {\"pass\":true|false,\"message\":\"...\"} on stdout with exit 0. Plugins are arbitrary code and require --allow-commands.",
                "properties": {
                    "name": { "type": "string" },
                    "cmd": {
                        "anyOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "cwd": { "type": "string", "default": "." },
                    "timeout_secs": { "type": "integer", "minimum": 0, "default": 300 },
                    "weight": { "type": "number", "description": "Relative importance in weighted completion reporting; unweighted checks count as 1.0." },
                    "when": { "$ref": "#/definitions/when_clause" }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema::JSONSchema;

    #[test]
    fn schema_compiles_as_valid_json_schema() {
        let schema = spec_json_schema();
        JSONSchema::compile(&schema).expect("schema must compile");
    }

    #[test]
    fn schema_accepts_a_real_spec() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let instance = json!({
            "file": [{"path": "README.md", "required": true, "min_size_bytes": 10, "weight": 1.0, "forbid_secrets": true}],
            "command": [{"name": "test", "cmd": ["cargo", "test"], "weight": 4.0}],
            "plugin": [{"name": "org gate", "cmd": ["org-gate"], "timeout_secs": 60}],
            "directory": [{"path": "src", "forbid_empty": true, "weight": 2.5}],
            "git": {"no_uncommitted_changes": true},
            "sbom": {"required": true, "formats": ["spdx", "cyclonedx"]}
        });
        assert!(compiled.is_valid(&instance));
    }

    #[test]
    fn schema_rejects_non_numeric_weight() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let instance = json!({"file": [{"path": "a.txt", "weight": "heavy"}]});
        assert!(!compiled.is_valid(&instance));
    }

    #[test]
    fn schema_rejects_unknown_top_level_key() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let instance = json!({"not_a_real_field": true});
        assert!(!compiled.is_valid(&instance));
    }

    #[test]
    fn schema_accepts_when_clauses() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let instance = json!({
            "file": [{"path": "Cargo.lock", "when": {"file_exists": "Cargo.toml", "os": "linux"}}],
            "command": [{"name": "test", "cmd": "cargo test", "when": {"file_exists": "Cargo.toml"}}],
            "directory": [{"path": "src", "when": {"os": "macos"}}]
        });
        assert!(compiled.is_valid(&instance));
    }

    #[test]
    fn schema_rejects_unknown_when_keys_and_bad_types() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        assert!(!compiled.is_valid(&json!({
            "file": [{"path": "a.txt", "when": {"file_existsx": "Cargo.toml"}}]
        })));
        assert!(!compiled.is_valid(&json!({
            "file": [{"path": "a.txt", "when": {"os": 1}}]
        })));
    }
}
