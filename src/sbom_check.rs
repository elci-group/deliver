//! SBOM presence and shape validation for the `[sbom]` spec section.
//!
//! Standard SBOM locations under `--base` are searched for SPDX (JSON or
//! tag-value) and CycloneDX (JSON) documents; a found document is valid
//! when it carries the core fields of its declared format. Detection is
//! deliberately shallow: this gate proves a well-formed SBOM exists, not
//! that its contents are accurate.

use crate::{CheckResult, SbomCheck};
use std::fs;
use std::path::{Path, PathBuf};

/// Candidate locations searched under `--base`, in order.
const CANDIDATE_PATTERNS: [&str; 4] = ["*.spdx", "*.spdx.json", "*.cdx.json", "bom.json"];

const SUPPORTED_FORMATS: [&str; 2] = ["spdx", "cyclonedx"];

/// One candidate SBOM document that parsed into a supported shape.
struct Found {
    path: PathBuf,
    detail: String,
    format: &'static str,
}

pub fn validate_sbom(check: &SbomCheck, base: &Path) -> CheckResult {
    let name = "sbom".to_string();
    let build = |pass: bool, message: String| CheckResult {
        name: name.clone(),
        pass,
        kind: "sbom".to_string(),
        weight: check.weight,
        message,
        skipped: false,
    };

    for format in &check.formats {
        if !SUPPORTED_FORMATS.contains(&format.as_str()) {
            return build(
                false,
                format!(
                    "unknown sbom format \"{}\". Supported formats: {}.",
                    format,
                    SUPPORTED_FORMATS.join(", ")
                ),
            );
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for pattern in CANDIDATE_PATTERNS {
        // Literal patterns (bom.json) come back unconditionally from
        // expand_paths; only existing files are candidates.
        candidates.extend(
            crate::glob::expand_paths(pattern, base)
                .into_iter()
                .filter(|path| path.is_file()),
        );
    }
    // Deterministic report order regardless of filesystem iteration order.
    candidates.sort();
    candidates.dedup();

    if candidates.is_empty() {
        return build(
            !check.required,
            if check.required {
                "no SBOM found. Suggestion: publish an SPDX or CycloneDX document (searched: \
                 *.spdx, *.spdx.json, *.cdx.json, bom.json)."
                    .to_string()
            } else {
                "no SBOM found (not required by this spec)".to_string()
            },
        );
    }

    let mut found: Vec<Found> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    for candidate in &candidates {
        match sniff(candidate) {
            Some((format, detail)) => found.push(Found {
                path: candidate.clone(),
                detail,
                format,
            }),
            None => rejected.push(display_path(base, candidate)),
        }
    }

    let found: Vec<Found> = found
        .into_iter()
        .filter(|item| check.formats.is_empty() || check.formats.contains(&item.format.to_string()))
        .collect();

    if found.is_empty() {
        let mut message =
            String::from("SBOM file(s) found but none with a valid, accepted shape:\n");
        for reason in &rejected {
            message.push_str(&format!("  - {}: unrecognized SBOM shape\n", reason));
        }
        if !check.formats.is_empty() {
            message.push_str(&format!(
                "  - accepted formats: {}\n",
                check.formats.join(", ")
            ));
        }
        message.push_str("Suggestion: export the SBOM in SPDX or CycloneDX JSON format.");
        return build(false, message);
    }

    let first = &found[0];
    let mut message = format!(
        "found {} at {}",
        first.detail,
        display_path(base, &first.path)
    );
    if found.len() > 1 {
        message.push_str(&format!(" (+{} more)", found.len() - 1));
    }
    build(true, message)
}

/// Classify one file: `Some((format, detail))` when it parses into a
/// supported SBOM shape with its core fields present.
fn sniff(path: &Path) -> Option<(&'static str, String)> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();
    if trimmed.starts_with('{') {
        let value: serde_json::Value = serde_json::from_str(&content).ok()?;
        let bom_format = value.get("bomFormat").and_then(|v| v.as_str());
        if bom_format == Some("CycloneDX") {
            let spec_version = value.get("specVersion").and_then(|v| v.as_str())?;
            let components = value.get("components").and_then(|v| v.as_array())?;
            return Some((
                "cyclonedx",
                format!(
                    "CycloneDX {} ({} components)",
                    spec_version,
                    components.len()
                ),
            ));
        }
        if let Some(spdx_version) = value.get("spdxVersion").and_then(|v| v.as_str()) {
            value.get("SPDXID")?;
            let packages = value.get("packages").and_then(|v| v.as_array())?;
            return Some((
                "spdx",
                format!("SPDX {} ({} packages)", spdx_version, packages.len()),
            ));
        }
        return None;
    }
    // SPDX tag-value: a spec-level `SPDXVersion:` record must be present.
    if content.lines().any(|line| line.starts_with("SPDXVersion:")) {
        return Some(("spdx", "SPDX tag-value".to_string()));
    }
    None
}

fn display_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn check(required: bool, formats: &[&str]) -> SbomCheck {
        SbomCheck {
            required,
            formats: formats.iter().map(|f| f.to_string()).collect(),
            weight: None,
        }
    }

    const CYCLONEDX: &str = r#"{
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "components": [{"type": "library", "name": "x"}]
    }"#;

    const SPDX_JSON: &str = r#"{
        "spdxVersion": "SPDX-2.3",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "doc",
        "packages": [{"SPDXID": "SPDXRef-Package", "name": "x"}]
    }"#;

    #[test]
    fn cyclonedx_json_passes() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bom.json"), CYCLONEDX).unwrap();
        let result = validate_sbom(&check(true, &[]), dir.path());
        assert!(result.pass);
        assert!(result.message.contains("CycloneDX 1.5 (1 components)"));
        assert!(result.message.contains("bom.json"));
    }

    #[test]
    fn spdx_json_and_tag_value_pass() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("doc.spdx.json"), SPDX_JSON).unwrap();
        assert!(validate_sbom(&check(true, &[]), dir.path()).pass);

        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("doc.spdx"),
            "SPDXVersion: SPDX-2.3\nSPDXID: SPDXRef-DOCUMENT\n",
        )
        .unwrap();
        let result = validate_sbom(&check(true, &["spdx"]), dir.path());
        assert!(result.pass);
        assert!(result.message.contains("SPDX tag-value"));
    }

    #[test]
    fn missing_sbom_fails_when_required_and_passes_when_optional() {
        let dir = TempDir::new().unwrap();
        let result = validate_sbom(&check(true, &[]), dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("no SBOM found"));

        let result = validate_sbom(&check(false, &[]), dir.path());
        assert!(result.pass);
    }

    #[test]
    fn unrecognized_shape_fails_even_when_optional() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bom.json"), r#"{"hello": "world"}"#).unwrap();
        let result = validate_sbom(&check(false, &[]), dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("unrecognized SBOM shape"));
    }

    #[test]
    fn format_filter_rejects_other_formats() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bom.json"), CYCLONEDX).unwrap();
        let result = validate_sbom(&check(true, &["spdx"]), dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("accepted formats: spdx"));

        assert!(validate_sbom(&check(true, &["cyclonedx"]), dir.path()).pass);
    }

    #[test]
    fn unknown_format_name_fails_closed() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bom.json"), CYCLONEDX).unwrap();
        let result = validate_sbom(&check(true, &["swid"]), dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("unknown sbom format"));
    }

    #[test]
    fn multiple_candidates_report_first_with_count() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bom.json"), CYCLONEDX).unwrap();
        fs::write(dir.path().join("other.cdx.json"), CYCLONEDX).unwrap();
        let result = validate_sbom(&check(true, &[]), dir.path());
        assert!(result.pass);
        assert!(result.message.contains("(+1 more)"));
    }
}
