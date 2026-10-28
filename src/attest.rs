//! Attestation bundles (`--attest`, `deliver keygen`, `deliver attest
//! verify`).
//!
//! A bundle is a self-contained receipt that a specific spec, at a specific
//! commit and time, produced a passing report: canonical payload JSON
//! (version, timestamp, base, spec and report hashes and tallies, git
//! commit), its SHA-256, and an optional Ed25519 signature over the payload.
//! `--attest` writes bundles only for passing runs, so a bundle's existence
//! is itself the claim; verification checks payload integrity always and
//! authenticates the signature when a public key is supplied.

use deliver::Report;
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Write an attestation bundle for a passing run to `path`.
///
/// `spec` describes the spec input: `Some((source, path_or_none, text))`
/// hashed into the payload, or `None` for `--file` runs.
pub fn write_bundle(
    report: &Report,
    base: &Path,
    spec: &SpecInput,
    git_commit: Option<&str>,
    path: &Path,
    signing_key: Option<&ed25519_dalek::SigningKey>,
) -> Result<(), String> {
    if !report.pass {
        return Err("attestation skipped: the run did not pass".to_string());
    }

    let report_json = serde_json::to_string(report).expect("Report serializes");
    let (spec_source, spec_path, spec_text) = match spec {
        SpecInput::File { path, text } => (
            "file",
            Some(path.display().to_string()),
            Some(text.as_str()),
        ),
        SpecInput::Inline(json) => ("inline", None, Some(json.as_str())),
        SpecInput::None => ("none", None, None),
    };

    let payload = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "created_at": now_rfc3339_utc(),
        "base": base.to_string_lossy(),
        "spec": {
            "source": spec_source,
            "path": spec_path,
            "sha256": spec_text.map(|text| sha256_hex(text.as_bytes())),
        },
        "report": {
            "pass": report.pass,
            "checks": report.checks.len(),
            "passed": report.checks.iter().filter(|c| c.pass && !c.skipped).count(),
            "failed": report.checks.iter().filter(|c| !c.pass).count(),
            "skipped": report.checks.iter().filter(|c| c.skipped).count(),
            "duration_ms": report.duration_ms,
            "sha256": sha256_hex(report_json.as_bytes()),
        },
        "git": {
            "commit": git_commit,
        },
    });

    write_payload_bundle(payload, path, signing_key)
}

/// Serialize, hash, optionally sign, and persist a payload.
fn write_payload_bundle(
    payload: Value,
    path: &Path,
    signing_key: Option<&ed25519_dalek::SigningKey>,
) -> Result<(), String> {
    // serde_json serializes Values with sorted keys, so this canonical
    // serialization is byte-identical when the bundle is verified later.
    let payload_bytes = serde_json::to_string(&payload).expect("payload serializes");
    let payload_sha256 = sha256_hex(payload_bytes.as_bytes());

    let signature = signing_key.map(|key| {
        let signature = key.sign(payload_bytes.as_bytes());
        json!({
            "algorithm": "ed25519",
            "public_key": hex(&key.verifying_key().to_bytes()),
            "signature": hex(&signature.to_bytes()),
        })
    });

    let bundle = json!({
        "payload": payload,
        "payload_sha256": payload_sha256,
        "signature": signature,
    });

    let text = serde_json::to_string_pretty(&bundle).expect("bundle serializes");
    std::fs::write(path, text + "\n")
        .map_err(|e| format!("failed to write attestation {}: {}", path.display(), e))
}

/// Verify a bundle. Always checks payload integrity against
/// `payload_sha256`; when a bundle is signed, additionally checks the
/// signature against the supplied public key file, or against the embedded
/// public key (self-consistency) when none is supplied.
pub fn verify_bundle(path: &Path, pub_key_path: Option<&Path>) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read attestation {}: {}", path.display(), e))?;
    let bundle: Value = serde_json::from_str(&text)
        .map_err(|e| format!("attestation {} is not valid JSON: {}", path.display(), e))?;

    let payload = bundle
        .get("payload")
        .ok_or_else(|| "bundle has no payload".to_string())?;
    let payload_bytes = serde_json::to_string(payload).expect("payload re-serializes");
    let expected_hash = bundle
        .get("payload_sha256")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "bundle has no payload_sha256".to_string())?;
    if sha256_hex(payload_bytes.as_bytes()) != expected_hash {
        return Err(
            "payload hash mismatch: the bundle has been modified after signing".to_string(),
        );
    }

    match bundle.get("signature") {
        None | Some(Value::Null) => Ok(format!(
            "integrity verified (unsigned bundle, payload {})",
            &expected_hash[..16.min(expected_hash.len())]
        )),
        Some(signature) => {
            let embedded_key = signature
                .get("public_key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "signature has no public_key".to_string())?;
            let signature_hex = signature
                .get("signature")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "signature has no signature value".to_string())?;

            let (verifying_key, authenticated) = match pub_key_path {
                Some(path) => (load_verifying_key(path)?, true),
                None => (parse_verifying_key(embedded_key)?, false),
            };

            let signature_bytes = parse_hex_bytes(signature_hex, 64)
                .map_err(|_| "signature value is not 64 hex-encoded bytes".to_string())?;
            let signature = Signature::from_bytes(signature_bytes.as_slice().try_into().unwrap());
            verifying_key
                .verify_strict(payload_bytes.as_bytes(), &signature)
                .map_err(|_| "signature verification failed".to_string())?;

            Ok(if authenticated {
                format!(
                    "signature verified against supplied key (payload {})",
                    &expected_hash[..16.min(expected_hash.len())]
                )
            } else {
                format!(
                    "integrity verified; signature is self-consistent with its embedded key \
                     (payload {}). Supply --pub to authenticate the signer.",
                    &expected_hash[..16.min(expected_hash.len())]
                )
            })
        }
    }
}

/// The spec input a run was driven by, hashed into the attestation payload.
pub enum SpecInput {
    File {
        path: std::path::PathBuf,
        text: String,
    },
    Inline(String),
    None,
}

/// Generate an Ed25519 keypair and write `<prefix>.key` (hex seed, mode
/// 0600 on Unix) plus `<prefix>.pub` (hex public key). Refuses to overwrite.
pub fn keygen(prefix: &str) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let key_path = std::path::PathBuf::from(format!("{}.key", prefix));
    let pub_path = std::path::PathBuf::from(format!("{}.pub", prefix));
    for path in [&key_path, &pub_path] {
        if path.exists() {
            return Err(format!("refusing to overwrite {}", path.display()));
        }
    }

    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("failed to gather entropy: {}", e))?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);

    std::fs::write(&key_path, hex(&seed) + "\n")
        .map_err(|e| format!("failed to write {}: {}", key_path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to restrict {}: {}", key_path.display(), e))?;
    }
    std::fs::write(
        &pub_path,
        hex(&signing_key.verifying_key().to_bytes()) + "\n",
    )
    .map_err(|e| format!("failed to write {}: {}", pub_path.display(), e))?;

    Ok((key_path, pub_path))
}

pub fn load_signing_key(path: &Path) -> Result<ed25519_dalek::SigningKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read signing key {}: {}", path.display(), e))?;
    let seed = parse_hex_bytes(text.trim(), 32).map_err(|_| {
        format!(
            "signing key {} must be 32 hex-encoded bytes",
            path.display()
        )
    })?;
    Ok(ed25519_dalek::SigningKey::from_bytes(
        seed.as_slice().try_into().unwrap(),
    ))
}

fn load_verifying_key(path: &Path) -> Result<VerifyingKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read public key {}: {}", path.display(), e))?;
    parse_verifying_key(text.trim())
}

fn parse_verifying_key(text: &str) -> Result<VerifyingKey, String> {
    let bytes = parse_hex_bytes(text, 32)
        .map_err(|_| "public key must be 32 hex-encoded bytes".to_string())?;
    VerifyingKey::from_bytes(bytes.as_slice().try_into().unwrap())
        .map_err(|e| format!("invalid public key: {}", e))
}

/// Current wall-clock time as RFC3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn now_rfc3339_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs();
    format_rfc3339_utc(secs)
}

/// Format seconds since the Unix epoch as RFC3339 UTC.
fn format_rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Days since the Unix epoch to a civil (year, month, day), valid for the
/// proleptic Gregorian calendar (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex(&digest)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn parse_hex_bytes(text: &str, expected_len: usize) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if text.len() != expected_len * 2 {
        return Err(format!("expected {} hex-encoded bytes", expected_len));
    }
    let mut out = Vec::with_capacity(expected_len);
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex at byte {}", i))?;
        let lo = (chunk[1] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex at byte {}", i))?;
        out.push((hi * 16 + lo) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deliver::CheckResult;

    /// A tiny passing/failing report for bundle tests.
    fn sample_report(pass: bool) -> Report {
        Report {
            pass,
            duration_ms: 42,
            checks: vec![CheckResult {
                name: "file: a.txt".to_string(),
                pass,
                kind: "file".to_string(),
                weight: None,
                message: "OK (5 bytes)".to_string(),
                skipped: false,
            }],
        }
    }

    #[test]
    fn rfc3339_formats_known_timestamps() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(
            format_rfc3339_utc(1_709_164_800),
            "2024-02-29T00:00:00Z",
            "leap day must format correctly"
        );
        assert_eq!(format_rfc3339_utc(1_793_137_799), "2026-10-27T21:49:59Z");
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0xfe, 0xff];
        assert_eq!(hex(&bytes), "0001feff");
        let parsed = parse_hex_bytes("0001feff", 4).unwrap();
        assert_eq!(parsed, vec![0, 1, 0xfe, 0xff]);
        assert!(
            parse_hex_bytes("0", 1).is_err(),
            "odd or short hex must fail"
        );
        assert!(
            parse_hex_bytes("0001fe", 4).is_err(),
            "wrong length must fail"
        );
        assert!(parse_hex_bytes("zz001122", 4).is_err(), "non-hex must fail");
    }

    #[test]
    fn keygen_writes_key_and_pub_and_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("k");
        let (key, public) = keygen(prefix.to_str().unwrap()).unwrap();
        assert!(key.exists() && public.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "private key must be owner-only");
        }

        let error = keygen(prefix.to_str().unwrap()).unwrap_err();
        assert!(error.contains("refusing to overwrite"));
    }

    #[test]
    fn bundle_round_trips_signed_and_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let report = sample_report(true);
        let path = dir.path().join("attest.json");
        let signing_key = {
            let mut seed = [7u8; 32];
            getrandom::getrandom(&mut seed).unwrap();
            ed25519_dalek::SigningKey::from_bytes(&seed)
        };

        write_bundle(
            &report,
            Path::new("/base"),
            &SpecInput::Inline("{\"file\":[]}".to_string()),
            Some("abc123"),
            &path,
            Some(&signing_key),
        )
        .unwrap();

        let verdict = verify_bundle(&path, None).expect("self-consistent bundle must verify");
        assert!(verdict.contains("self-consistent"));

        let pub_path = dir.path().join("k.pub");
        std::fs::write(&pub_path, hex(&signing_key.verifying_key().to_bytes())).unwrap();
        let verdict = verify_bundle(&path, Some(&pub_path)).expect("supplied key must verify");
        assert!(verdict.contains("supplied key"));

        // Tamper with the payload: hash must catch it before signature.
        let mut bundle: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        bundle["payload"]["report"]["passed"] = json!(999);
        std::fs::write(&path, serde_json::to_string_pretty(&bundle).unwrap()).unwrap();
        let error = verify_bundle(&path, Some(&pub_path)).unwrap_err();
        assert!(error.contains("hash mismatch"));

        // A re-signed bundle with a different key must fail the supplied key.
        let fresh = dir.path().join("fresh.json");
        let other_key = {
            let mut seed = [9u8; 32];
            getrandom::getrandom(&mut seed).unwrap();
            ed25519_dalek::SigningKey::from_bytes(&seed)
        };
        write_bundle(
            &sample_report(true),
            Path::new("/base"),
            &SpecInput::None,
            None,
            &fresh,
            Some(&other_key),
        )
        .unwrap();
        assert!(verify_bundle(&fresh, Some(&pub_path)).is_err());
    }

    #[test]
    fn refusing_to_attest_failed_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let error = write_bundle(
            &sample_report(false),
            Path::new("/base"),
            &SpecInput::None,
            None,
            &path,
            None,
        )
        .unwrap_err();
        assert!(error.contains("did not pass"));
        assert!(!path.exists(), "failed runs must not produce bundles");
    }
}
