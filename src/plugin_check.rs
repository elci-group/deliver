//! Plugin checks (`[[plugin]]`): a narrow, subprocess-protocol extension
//! point for organization-specific checks, without forking `deliver`.
//!
//! Protocol v1: the runner spawns the plugin with its working directory set
//! to the check's resolved `cwd`, writes one JSON request line to stdin, and
//! closes stdin. The plugin answers with one JSON object on stdout before
//! exiting:
//!
//! ```text
//! request:  {"protocol": 1, "name": "...", "base": "/abs/path"}
//! response: {"pass": true, "message": "why"}
//! ```
//!
//! `pass` must be a boolean and the process must exit 0; anything else is a
//! plugin failure whose message quotes the plugin's stderr (bounded). Extra
//! response fields are ignored so the protocol can evolve. Plugins are
//! arbitrary code and are gated by `--allow-commands` like command checks.

use crate::{CheckResult, CommandSpec, PluginCheck};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The plugin protocol version this build speaks (currently `1`). Part of
/// the public contract: it is sent in every request and only increases
/// with an incompatible wire change.
pub const PROTOCOL_VERSION: u64 = 1;

pub fn validate_plugin(check: &PluginCheck, base: &Path) -> CheckResult {
    let fail = |message: String| CheckResult {
        name: check.name.clone(),
        pass: false,
        kind: "plugin".to_string(),
        weight: check.weight,
        message,
        skipped: false,
    };

    let (program, args) = match command_parts(check) {
        Ok(parts) => parts,
        Err(message) => return fail(message),
    };

    let cwd = match crate::path_guard::resolve(base, &check.cwd) {
        Ok(path) => path,
        Err(message) => return fail(message),
    };

    let mut command = Command::new(&program);
    command
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return fail(format!(
                "failed to spawn plugin '{}': {}. Suggestion: check if the plugin exists and is executable.",
                program, error
            ));
        }
    };

    let request = serde_json::json!({
        "protocol": PROTOCOL_VERSION,
        "name": check.name,
        "base": base.canonicalize().unwrap_or_else(|_| base.to_path_buf()).to_string_lossy(),
    });
    // The request is far smaller than the OS pipe buffer, so this write
    // cannot block; dropping stdin signals end-of-request to the plugin.
    // A plugin that exits without reading stdin turns this write into
    // EPIPE — that is the plugin's answer, not a delivery failure: the
    // wait below surfaces its real exit status and stdout.
    {
        let mut stdin = child.stdin.take().expect("plugin stdin was piped");
        let _ = writeln!(stdin, "{}", request);
    }

    let start = std::time::Instant::now();
    let result = match crate::command::wait_with_timeout(
        child,
        std::time::Duration::from_secs(check.timeout_secs),
    ) {
        Ok(result) => result,
        Err(error) => {
            return fail(if error == "timeout" {
                format!(
                        "plugin timed out after {}s. Suggestion: increase timeout_secs or optimize the plugin.",
                        check.timeout_secs
                    )
            } else {
                format!("plugin execution failed: {error}")
            });
        }
    };
    let elapsed = start.elapsed().as_millis();

    if result.stdout.exceeded || result.stderr.exceeded {
        return fail(format!(
            "plugin output exceeded {} bytes per stream; reduce output before validating it",
            crate::command::MAX_CAPTURE_BYTES
        ));
    }

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr.bytes);
        let stderr = stderr.trim();
        let stderr = if stderr.len() > 500 {
            let mut truncated = stderr[..500].to_string();
            truncated.push_str("... (truncated)");
            truncated
        } else {
            stderr.to_string()
        };
        return fail(format!(
            "plugin exited with code {:?} ({} ms){}",
            result.status.code(),
            elapsed,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&result.stdout.bytes);
    let response: Value = match serde_json::from_str(stdout.trim()) {
        Ok(value) => value,
        Err(error) => {
            return fail(format!(
                "plugin did not answer with one JSON object on stdout: {}",
                error
            ));
        }
    };
    if !response.is_object() {
        return fail("plugin response must be a JSON object".to_string());
    }

    let pass = match response.get("pass").and_then(|value| value.as_bool()) {
        Some(pass) => pass,
        None => {
            return fail(
                "plugin response missing boolean \"pass\". Suggestion: update the plugin to answer {\"pass\": true|false, \"message\": \"...\"}."
                    .to_string(),
            );
        }
    };
    let plugin_message = response
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    if pass {
        CheckResult {
            name: check.name.clone(),
            pass: true,
            kind: "plugin".to_string(),
            weight: check.weight,
            message: if plugin_message.is_empty() {
                format!("OK ({} ms)", elapsed)
            } else {
                format!("OK ({} ms): {}", elapsed, plugin_message)
            },
            skipped: false,
        }
    } else {
        CheckResult {
            name: check.name.clone(),
            pass: false,
            kind: "plugin".to_string(),
            weight: check.weight,
            message: if plugin_message.is_empty() {
                format!("plugin reported failure ({} ms)", elapsed)
            } else {
                plugin_message.to_string()
            },
            skipped: false,
        }
    }
}

fn command_parts(check: &PluginCheck) -> Result<(String, Vec<String>), String> {
    match &check.cmd {
        CommandSpec::String(command) => {
            let mut parts = command.split_whitespace();
            let program = parts
                .next()
                .ok_or_else(|| "empty command".to_string())?
                .to_string();
            Ok((program, parts.map(ToString::to_string).collect()))
        }
        CommandSpec::Array(parts) => {
            if parts.is_empty() {
                Err("empty command array".to_string())
            } else {
                Ok((parts[0].clone(), parts[1..].to_vec()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn base_check(name: &str, cmd: &[&str]) -> PluginCheck {
        PluginCheck {
            name: name.to_string(),
            cmd: CommandSpec::Array(cmd.iter().map(|s| s.to_string()).collect()),
            cwd: ".".into(),
            timeout_secs: 10,
            weight: None,
            when: None,
        }
    }

    #[test]
    fn passing_plugin_is_recorded_with_its_message() {
        let dir = TempDir::new().unwrap();
        let check = base_check(
            "good plugin",
            &[
                "sh",
                "-c",
                "cat > /dev/null; echo '{\"pass\":true,\"message\":\"all good\"}'",
            ],
        );
        let result = validate_plugin(&check, dir.path());
        assert!(result.pass, "{}", result.message);
        assert_eq!(result.kind, "plugin");
        assert!(result.message.contains("all good"), "{}", result.message);
    }

    #[test]
    fn plugin_receives_the_protocol_request_on_stdin() {
        let dir = TempDir::new().unwrap();
        // Echo the request back inside the message (quotes stripped so the
        // wrapping JSON stays valid); the runner must have sent one JSON
        // line with protocol, name, and base.
        let script = "read request; stripped=$(printf '%s' \"$request\" | tr -d '\"'); printf '{\"pass\":true,\"message\":\"%s\"}' \"$stripped\"";
        let check = base_check("echo plugin", &["sh", "-c", script]);
        let result = validate_plugin(&check, dir.path());
        assert!(result.pass, "{}", result.message);
        assert!(result.message.contains("protocol:1"), "{}", result.message);
        assert!(result.message.contains("echo plugin"), "{}", result.message);
    }

    #[test]
    fn failing_plugin_pass_message_through() {
        let dir = TempDir::new().unwrap();
        let check = base_check(
            "bad plugin",
            &[
                "sh",
                "-c",
                "cat > /dev/null; echo '{\"pass\":false,\"message\":\"docs drift\"}'",
            ],
        );
        let result = validate_plugin(&check, dir.path());
        assert!(!result.pass);
        assert_eq!(result.message, "docs drift");
    }

    #[test]
    fn invalid_responses_fail_with_actionable_messages() {
        let dir = TempDir::new().unwrap();
        let cases = [
            (
                &["sh", "-c", "echo 'not json'"][..],
                "did not answer with one JSON object",
            ),
            (
                &["sh", "-c", "echo '{\"message\":\"no pass field\"}'"][..],
                "missing boolean",
            ),
            (&["sh", "-c", "echo '[1,2]'"][..], "must be a JSON object"),
            (&["sh", "-c", "exit 3"][..], "exited with code"),
        ];
        for (cmd, expected) in cases {
            let check = base_check("broken plugin", cmd);
            let result = validate_plugin(&check, dir.path());
            assert!(!result.pass, "case {expected} must fail");
            assert!(
                result.message.contains(expected),
                "case {expected}: {}",
                result.message
            );
        }
    }

    #[test]
    fn nonzero_exit_stderr_is_quoted_and_bounded() {
        let dir = TempDir::new().unwrap();
        let check = base_check("loud plugin", &["sh", "-c", "echo boom >&2; exit 2"]);
        let result = validate_plugin(&check, dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("boom"), "{}", result.message);
        assert!(result.message.contains("2"), "{}", result.message);
    }

    #[test]
    fn timeout_kills_stuck_plugins() {
        let dir = TempDir::new().unwrap();
        let check = PluginCheck {
            timeout_secs: 1,
            ..base_check("stuck plugin", &["sh", "-c", "sleep 30"])
        };
        let result = validate_plugin(&check, dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("timed out"), "{}", result.message);
    }

    #[test]
    fn cwd_must_stay_under_base() {
        let dir = TempDir::new().unwrap();
        let check = PluginCheck {
            cwd: "../outside".into(),
            ..base_check("escaping plugin", &["true"])
        };
        let result = validate_plugin(&check, dir.path());
        assert!(!result.pass);
        assert!(result.message.contains("--base") || result.message.contains("outside"));
    }

    #[test]
    fn spawn_failure_names_the_program() {
        let dir = TempDir::new().unwrap();
        let check = base_check("missing plugin", &["definitely-not-a-real-plugin-binary"]);
        let result = validate_plugin(&check, dir.path());
        assert!(!result.pass);
        assert!(result
            .message
            .contains("definitely-not-a-real-plugin-binary"));
    }

    #[test]
    fn empty_stdin_write_is_tolerated_by_plugins_that_ignore_it() {
        let dir = TempDir::new().unwrap();
        let check = base_check("blind plugin", &["sh", "-c", "echo '{\"pass\":true}'"]);
        let result = validate_plugin(&check, dir.path());
        assert!(result.pass, "{}", result.message);
        assert!(
            result.message.starts_with("OK ("),
            "no plugin message means plain OK: {}",
            result.message
        );
    }

    #[test]
    fn spec_round_trip_accepts_plugin_sections() {
        let spec = crate::Spec::from_toml(
            r#"
            [[plugin]]
            name = "org gate"
            cmd = ["org-gate", "--strict"]
            timeout_secs = 60
            weight = 2.0

            [[plugin]]
            name = "skipped one"
            cmd = "org-gate --fast"
            when = { os = "nonexistent-os" }
            "#,
        )
        .unwrap();
        assert_eq!(spec.plugins.len(), 2);
        assert_eq!(spec.plugins[0].timeout_secs, 60);
        assert_eq!(spec.plugins[1].weight, None);

        assert!(crate::Spec::from_toml(
            "[[plugin]]\nname = \"x\"\ncmd = [\"x\"]\nunknown_field = 1\n"
        )
        .is_err());
        let _ = fs::metadata("/").unwrap();
    }
}
