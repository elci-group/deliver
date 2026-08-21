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
            "directory": { "type": "array", "items": { "$ref": "#/definitions/directory_check" } },
            "directories": { "type": "array", "items": { "$ref": "#/definitions/directory_check" } },
            "git": { "$ref": "#/definitions/git_check" },
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
                    }
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
                    "stderr_contains": { "type": "array", "items": { "type": "string" } }
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
                    "forbid_empty": { "type": "boolean", "default": false }
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
            "file": [{"path": "README.md", "required": true, "min_size_bytes": 10}],
            "command": [{"name": "test", "cmd": ["cargo", "test"]}],
            "directory": [{"path": "src", "forbid_empty": true}],
            "git": {"no_uncommitted_changes": true}
        });
        assert!(compiled.is_valid(&instance));
    }

    #[test]
    fn schema_rejects_unknown_top_level_key() {
        let schema = spec_json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let instance = json!({"not_a_real_field": true});
        assert!(!compiled.is_valid(&instance));
    }
}
