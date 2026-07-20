use crate::cli::{ColorMode, OutputFormat, ProgressMode};
use deliver::Report;
use std::io::{self, IsTerminal, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub struct TextStyle {
    enabled: bool,
}

impl TextStyle {
    pub fn detect(mode: ColorMode) -> Self {
        let enabled = match mode {
            ColorMode::Auto => io::stdout().is_terminal(),
            ColorMode::Always => true,
            ColorMode::Never => false,
        };
        Self { enabled }
    }

    fn paint(&self, color: &str, text: impl AsRef<str>) -> String {
        if self.enabled {
            format!("{}{}{}", color, text.as_ref(), RESET)
        } else {
            text.as_ref().to_string()
        }
    }

    fn status(&self, pass: bool, text: &str) -> String {
        self.paint(if pass { GREEN } else { RED }, text)
    }
}

pub struct Progress {
    active: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Progress {
    pub fn start(mode: ProgressMode, format: OutputFormat, label: &'static str) -> Self {
        let enabled = match mode {
            ProgressMode::Auto => format == OutputFormat::Text && io::stderr().is_terminal(),
            ProgressMode::Always => true,
            ProgressMode::Never => false,
        };

        if !enabled {
            return Self {
                active: Arc::new(AtomicBool::new(false)),
                handle: None,
            };
        }

        let active = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&active);
        let handle = thread::spawn(move || {
            let frames = ["-", "\\", "|", "/"];
            let mut index = 0;
            while worker_active.load(Ordering::Relaxed) {
                eprint!("\r{} {}", frames[index % frames.len()], label);
                let _ = io::stderr().flush();
                index += 1;
                thread::sleep(Duration::from_millis(90));
            }
            eprint!("\r\x1b[2K");
            let _ = io::stderr().flush();
        });

        Self {
            active,
            handle: Some(handle),
        }
    }

    pub fn finish(mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn render_text(report: &Report, style: &TextStyle) {
    let passed = report.checks.iter().filter(|check| check.pass).count();
    let failed = report.checks.len().saturating_sub(passed);

    println!(
        "{}",
        style.paint(
            BOLD,
            if report.pass {
                "deliver report: PASS"
            } else {
                "deliver report: FAIL"
            }
        )
    );
    println!(
        "{}",
        style.paint(
            DIM,
            format!(
                "{} checks: {} passed, {} failed in {} ms",
                report.checks.len(),
                passed,
                failed,
                report.duration_ms
            )
        )
    );
    println!();

    for check in &report.checks {
        let symbol = if check.pass { "✓" } else { "✗" };
        let status = style.status(check.pass, symbol);
        let kind = style.paint(BLUE, format!("[{}]", check.kind));
        let name = style.paint(BOLD, &check.name);
        let message = if check.pass {
            style.paint(DIM, &check.message)
        } else {
            style.paint(YELLOW, &check.message)
        };
        println!("{} {} {} {}", status, name, kind, message);
    }
}
