use crate::{CheckResult, CommandCheck, CommandSpec};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const PIPE_CLOSE_GRACE: Duration = Duration::from_secs(1);

pub fn validate_command(check: &CommandCheck, base: &Path) -> CheckResult {
    let (program, args) = match command_parts(check) {
        Ok(parts) => parts,
        Err(message) => {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                weight: check.weight,
                message,
            }
        }
    };

    let cwd = match crate::path_guard::resolve(base, &check.cwd) {
        Ok(path) => path,
        Err(message) => {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                weight: check.weight,
                message,
            }
        }
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Each check gets its own process group so a timeout can stop ordinary
    // descendants as well as the direct child on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

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
                weight: check.weight,
                message: format!(
                    "failed to spawn '{}': {}. Suggestion: check if the command exists and is executable.",
                    program, error
                ),
            }
        }
    };

    let timeout = Duration::from_secs(check.timeout_secs);
    let result = match wait_with_timeout(child, timeout) {
        Ok(result) => result,
        Err(error) => {
            let message = if error == "timeout" {
                format!(
                    "command timed out after {}s. Suggestion: increase timeout_secs or optimize the command.",
                    check.timeout_secs
                )
            } else {
                format!("command execution failed: {error}")
            };
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                weight: check.weight,
                message,
            };
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
    if result.stdout.exceeded || result.stderr.exceeded {
        return CheckResult {
            name: check.name.clone(),
            pass: false,
            kind: "command".to_string(),
            weight: check.weight,
            message: format!(
                "command output exceeded {} bytes per stream; reduce output before validating it",
                MAX_CAPTURE_BYTES
            ),
        };
    }
    let stdout = String::from_utf8_lossy(&result.stdout.bytes);
    let stderr = String::from_utf8_lossy(&result.stderr.bytes);

    if result.status.code() != Some(check.expect_exit) {
        return CheckResult {
            name: check.name.clone(),
            pass: false,
            kind: "command".to_string(),
            weight: check.weight,
            message: format!(
                "exit code {:?}, expected {} ({} ms). Suggestion: fix the command or adjust expect_exit.",
                result.status.code(),
                check.expect_exit,
                elapsed
            ),
        };
    }

    for (index, expected) in check.stdout_contains.iter().enumerate() {
        if !stdout.contains(expected) {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                weight: check.weight,
                message: format!(
                    "stdout missing expected value at stdout_contains[{}]. Suggestion: update the spec or fix the command output.",
                    index
                ),
            };
        }
    }

    for (index, expected) in check.stderr_contains.iter().enumerate() {
        if !stderr.contains(expected) {
            return CheckResult {
                name: check.name.clone(),
                pass: false,
                kind: "command".to_string(),
                weight: check.weight,
                message: format!(
                    "stderr missing expected value at stderr_contains[{}]. Suggestion: update the spec or fix the command output.",
                    index
                ),
            };
        }
    }

    CheckResult {
        name: check.name.clone(),
        pass: true,
        kind: "command".to_string(),
        weight: check.weight,
        message: format!("OK (exit {}, {} ms)", check.expect_exit, elapsed),
    }
}

#[derive(Debug)]
struct ProcessResult {
    status: std::process::ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

#[derive(Debug)]
struct CapturedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<ProcessResult, String> {
    let start = Instant::now();
    let stdout_rx = child.stdout.take().map(read_pipe);
    let stderr_rx = child.stderr.take().map(read_pipe);

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_process_group(&child);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".to_string());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                kill_process_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait error: {error}"));
            }
        }
    };

    let stdout = receive_pipe(stdout_rx)?;
    let stderr = receive_pipe(stderr_rx)?;

    Ok(ProcessResult {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe<R>(mut pipe: R) -> Receiver<std::io::Result<CapturedOutput>>
where
    R: Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut exceeded = false;
        let result = loop {
            match pipe.read(&mut chunk) {
                Ok(0) => {
                    break Ok(CapturedOutput {
                        bytes: buffer,
                        exceeded,
                    })
                }
                Ok(n) => {
                    let available = MAX_CAPTURE_BYTES.saturating_sub(buffer.len());
                    buffer.extend_from_slice(&chunk[..n.min(available)]);
                    exceeded |= n > available;
                }
                Err(error) => break Err(error),
            }
        };
        let _ = tx.send(result);
    });
    rx
}

fn receive_pipe(
    rx: Option<Receiver<std::io::Result<CapturedOutput>>>,
) -> Result<CapturedOutput, String> {
    let rx = rx.ok_or_else(|| "command output pipe unavailable".to_string())?;
    rx.recv_timeout(PIPE_CLOSE_GRACE)
        .map_err(|_| "command output pipe remained open after exit".to_string())?
        .map_err(|error| format!("cannot read command output: {error}"))
}

#[cfg(unix)]
fn kill_process_group(child: &std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // The child was started as group leader via process_group(0). Negative
    // pid targets that group. Failure still falls back to child.kill().
    unsafe {
        kill(-(child.id() as i32), 9);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_child: &std::process::Child) {}
