use crate::{CheckResult, CommandCheck, CommandSpec};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn validate_command(check: &CommandCheck, base: &Path) -> CheckResult {
    let (program, args) = match command_parts(check) {
        Ok(parts) => parts,
        Err(message) => {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                message,
            }
        }
    };

    let cwd = base.join(&check.cwd);
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in &check.env {
        cmd.env(key, value);
    }

    let start = Instant::now();
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                message: format!("failed to spawn '{}': {}", program, error),
            }
        }
    };

    let timeout = Duration::from_secs(check.timeout_secs);
    let result = match wait_with_timeout(child, timeout) {
        Ok(result) => result,
        Err(error) => {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                message: format!("command timed out after {}s: {}", check.timeout_secs, error),
            }
        }
    };

    check_command_result(check, &result, start.elapsed().as_millis())
}

fn command_parts(check: &CommandCheck) -> Result<(String, Vec<String>), String> {
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

fn check_command_result(
    check: &CommandCheck,
    result: &ProcessResult,
    elapsed: u128,
) -> CheckResult {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    if result.status.code() != Some(check.expect_exit) {
        return CheckResult {
            name: check.name.clone(),
            pass: false,
            kind: "command".to_string(),
            message: format!(
                "exit code {:?}, expected {} ({} ms)",
                result.status.code(),
                check.expect_exit,
                elapsed
            ),
        };
    }

    for expected in &check.stdout_contains {
        if !stdout.contains(expected) {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                message: format!(
                    "stdout missing expected: {}; update stdout_contains or fix the command output",
                    expected
                ),
            };
        }
    }

    for expected in &check.stderr_contains {
        if !stderr.contains(expected) {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                message: format!(
                    "stderr missing expected: {}; update stderr_contains or fix the command output",
                    expected
                ),
            };
        }
    }

    CheckResult {
        name: check.name.clone(),
        pass: true,
        kind: "command".to_string(),
        message: format!("OK (exit {}, {} ms)", check.expect_exit, elapsed),
    }
}

#[derive(Debug)]
struct ProcessResult {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<ProcessResult, String> {
    let start = Instant::now();
    let stdout_handle = child.stdout.take().map(read_pipe);
    let stderr_handle = child.stderr.take().map(read_pipe);

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".to_string());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("wait error: {}; retry the command", error)),
        }
    };

    let stdout = stdout_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    Ok(ProcessResult {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe<R>(mut pipe: R) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        buffer
    })
}
