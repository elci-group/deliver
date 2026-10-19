use crate::cli::{ColorMode, OutputFormat, ProgressMode};
use deliver::{CheckResult, Report};
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

/// Percentage (0..=100) of the total check weight carried by passing
/// checks, or `None` when no check declares a weight. Checks without a
/// weight count as 1.0, so mixed specs blend weighted and unweighted
/// checks. A spec whose weights sum to zero reports 0%.
fn weighted_completion_pct(report: &Report) -> Option<f64> {
    if !report.checks.iter().any(|check| check.weight.is_some()) {
        return None;
    }
    let weight_of = |check: &CheckResult| check.weight.unwrap_or(1.0);
    let total: f64 = report.checks.iter().map(weight_of).sum();
    let passed: f64 = report
        .checks
        .iter()
        .filter(|check| check.pass)
        .map(weight_of)
        .sum();
    Some(if total > 0.0 {
        passed / total * 100.0
    } else {
        0.0
    })
}

pub fn render_text(report: &Report, style: &TextStyle) {
    let passed = report.checks.iter().filter(|check| check.pass).count();
    let failed = report.checks.len().saturating_sub(passed);

    let mut summary = format!(
        "{} checks: {} passed, {} failed in {} ms",
        report.checks.len(),
        passed,
        failed,
        report.duration_ms
    );
    if let Some(pct) = weighted_completion_pct(report) {
        summary = format!("{} · {:.0}% complete by weight", summary, pct);
    }

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
    println!("{}", style.paint(DIM, summary));
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

/// Render a SARIF 2.1.0 log. Only failed checks become `results`, matching
/// how SARIF-consuming tools (GitHub code scanning, etc.) treat entries as
/// findings to annotate rather than a full pass/fail ledger.
pub fn render_sarif(report: &Report) -> String {
    let rule_ids: Vec<&str> = {
        let mut kinds: Vec<&str> = report.checks.iter().map(|c| c.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    };

    let rules: Vec<String> = rule_ids
        .iter()
        .map(|kind| {
            format!(
                r#"{{"id":{},"name":{}}}"#,
                json_string(kind),
                json_string(&format!("{}-check", kind))
            )
        })
        .collect();

    let results: Vec<String> = report
        .checks
        .iter()
        .filter(|check| !check.pass)
        .map(|check| {
            format!(
                r#"{{"ruleId":{},"level":"error","message":{{"text":{}}}}}"#,
                json_string(&check.kind),
                json_string(&format!("{}: {}", check.name, check.message))
            )
        })
        .collect();

    format!(
        r#"{{"$schema":"https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{{"tool":{{"driver":{{"name":"deliver","informationUri":"https://github.com/","version":{},"rules":[{}]}}}},"results":[{}]}}]}}"#,
        json_string(env!("CARGO_PKG_VERSION")),
        rules.join(","),
        results.join(",")
    )
}

/// Render a JUnit XML report. Every check becomes a `<testcase>`; failing
/// checks additionally carry a `<failure>` child, per the de facto JUnit XML
/// schema most CI dashboards consume.
pub fn render_junit(report: &Report) -> String {
    let failures = report.checks.iter().filter(|c| !c.pass).count();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites>\n  <testsuite name=\"deliver\" tests=\"{}\" failures=\"{}\" time=\"{:.3}\">\n",
        report.checks.len(),
        failures,
        report.duration_ms as f64 / 1000.0
    ));
    for check in &report.checks {
        out.push_str(&format!(
            "    <testcase classname=\"{}\" name=\"{}\">\n",
            escape_xml(&check.kind),
            escape_xml(&check.name)
        ));
        if !check.pass {
            out.push_str(&format!(
                "      <failure message=\"{}\">{}</failure>\n",
                escape_xml(&check.message),
                escape_xml(&check.message)
            ));
        }
        out.push_str("    </testcase>\n");
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(pass: bool) -> Report {
        Report {
            pass,
            duration_ms: 42,
            checks: vec![
                CheckResult {
                    name: "file: README.md".to_string(),
                    pass: true,
                    kind: "file".to_string(),
                    weight: None,
                    message: "OK (10 bytes)".to_string(),
                },
                CheckResult {
                    name: "cargo test".to_string(),
                    pass,
                    kind: "command".to_string(),
                    weight: None,
                    message: if pass {
                        "OK".to_string()
                    } else {
                        "exit code mismatch \"quoted\" & <weird>".to_string()
                    },
                },
            ],
        }
    }

    #[test]
    fn sarif_only_includes_failures() {
        let report = sample_report(false);
        let sarif = render_sarif(&report);
        let value: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        let results = value["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["ruleId"], "command");
    }

    #[test]
    fn sarif_is_valid_json_when_all_pass() {
        let report = sample_report(true);
        let sarif = render_sarif(&report);
        let value: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        assert!(value["runs"][0]["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn junit_escapes_special_characters_and_counts_failures() {
        let report = sample_report(false);
        let xml = render_junit(&report);
        assert!(xml.contains("tests=\"2\" failures=\"1\""));
        assert!(xml.contains("&quot;quoted&quot;"));
        assert!(xml.contains("&lt;weird&gt;"));
        assert!(xml.contains("<failure"));
    }

    #[test]
    fn junit_omits_failure_element_when_all_pass() {
        let report = sample_report(true);
        let xml = render_junit(&report);
        assert!(xml.contains("tests=\"2\" failures=\"0\""));
        assert!(!xml.contains("<failure"));
    }

    fn report_with_weights(weights: &[Option<f64>], pass: bool) -> Report {
        Report {
            pass,
            duration_ms: 1,
            checks: weights
                .iter()
                .enumerate()
                .map(|(i, weight)| CheckResult {
                    name: format!("check {i}"),
                    pass,
                    kind: "file".to_string(),
                    message: "m".to_string(),
                    weight: *weight,
                })
                .collect(),
        }
    }

    #[test]
    fn weighted_completion_is_none_when_no_check_has_weight() {
        assert_eq!(weighted_completion_pct(&sample_report(true)), None);
    }

    #[test]
    fn weighted_completion_all_pass_is_100() {
        let report = report_with_weights(&[Some(1.0), Some(4.0)], true);
        assert_eq!(weighted_completion_pct(&report), Some(100.0));
    }

    #[test]
    fn weighted_completion_all_fail_is_0() {
        let report = report_with_weights(&[Some(1.0), Some(4.0)], false);
        assert_eq!(weighted_completion_pct(&report), Some(0.0));
    }

    #[test]
    fn weighted_completion_counts_unweighted_checks_as_one() {
        let mut report = report_with_weights(&[Some(3.0), None], true);
        report.checks[1].pass = false;
        report.pass = false;
        assert_eq!(weighted_completion_pct(&report), Some(75.0));
    }

    #[test]
    fn weighted_completion_zero_total_weight_is_0() {
        let report = report_with_weights(&[Some(0.0)], true);
        assert_eq!(weighted_completion_pct(&report), Some(0.0));
    }
}
