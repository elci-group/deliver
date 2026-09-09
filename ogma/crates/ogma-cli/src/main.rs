//! Command-line interface for the Ogma deterministic multilingual
//! conformance engine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use ogma_model::{codes, AuditResult, Confidence, LocaleReport, OverallStatus, Policy, Severity};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "ogma",
    version = "0.1.0",
    about = "Ogma — deterministic multilingual conformance engine"
)]
struct Cli {
    /// Policy file (default: ./ogma.toml when present, else built-in defaults)
    #[arg(long, global = true, value_name = "FILE")]
    policy: Option<PathBuf>,

    /// Repository root to audit
    #[arg(long, global = true, default_value = ".", value_name = "DIR")]
    root: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write an ogma.toml policy file into the root
    Init {
        /// Overwrite an existing ogma.toml
        #[arg(long)]
        force: bool,
    },
    /// Print a condensed architecture summary
    Detect,
    /// Print the full audit report
    Audit {
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        format: ReportFormat,
        /// Write the report to a file instead of stdout
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Enforcement mode: exit non-zero when the policy is violated
    Check {
        #[arg(long, value_enum, default_value_t = CheckFormat::Text)]
        format: CheckFormat,
    },
    /// Explain a violation code or a finding from the last audit
    Explain { target: String },
    /// Inspect or validate the effective policy
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Print the locale topology table
    Locales,
    /// Print the linguistic surface inventory
    Strings,
    /// Print a machine-readable report (json or sarif)
    Report {
        #[arg(long, value_enum, default_value_t = MachineFormat::Json)]
        format: MachineFormat,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Install the ogma pre-commit git hook
    InstallHook,
    /// Print a GitHub Actions CI workflow running `ogma check`
    CiTemplate,
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Print the effective policy file (or built-in defaults)
    Show,
    /// Load and validate the policy against defaults
    Validate,
    /// Alias of `init`: write an ogma.toml policy file
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReportFormat {
    Text,
    Json,
    Sarif,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CheckFormat {
    Text,
    Json,
    Sarif,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MachineFormat {
    Json,
    Sarif,
}

type CliResult = Result<u8, (String, u8)>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(code) => ExitCode::from(code),
        Err((message, code)) => {
            eprintln!("ogma: {message}");
            ExitCode::from(code)
        }
    }
}

fn dispatch(cli: &Cli) -> CliResult {
    match &cli.command {
        Command::Init { force } => cmd_init(cli, *force),
        Command::Detect => cmd_detect(cli),
        Command::Audit { format, out } => cmd_audit(cli, *format, out.as_deref()),
        Command::Check { format } => cmd_check(cli, *format),
        Command::Explain { target } => cmd_explain(cli, target),
        Command::Policy { action } => match action {
            PolicyAction::Show => cmd_policy_show(cli),
            PolicyAction::Validate => cmd_policy_validate(cli),
            PolicyAction::Init { force } => cmd_init(cli, *force),
        },
        Command::Locales => cmd_locales(cli),
        Command::Strings => cmd_strings(cli),
        Command::Report { format, out } => cmd_report(cli, *format, out.as_deref()),
        Command::InstallHook => cmd_install_hook(cli),
        Command::CiTemplate => {
            print!("{}", ogma_integrations::ci_workflow_yaml());
            Ok(0)
        }
    }
}

/// Resolve the effective policy: explicit `--policy`, then `./ogma.toml`,
/// then built-in defaults. Policy problems are exit code 3.
fn resolve_policy(cli: &Cli) -> Result<Policy, (String, u8)> {
    if let Some(path) = &cli.policy {
        let text = fs::read_to_string(path).map_err(|e| {
            (
                format!("cannot read policy file {}: {e}", path.display()),
                3,
            )
        })?;
        return ogma_policy::parse_policy(&text)
            .map_err(|e| (e.message, 3));
    }
    let default_path = cli.root.join("ogma.toml");
    if default_path.is_file() {
        return ogma_policy::load_policy(&default_path)
            .map_err(|e| (e.message, 3));
    }
    Ok(Policy::default())
}

fn policy_file(cli: &Cli) -> Option<PathBuf> {
    if let Some(path) = &cli.policy {
        return Some(path.clone());
    }
    let default_path = cli.root.join("ogma.toml");
    default_path.is_file().then_some(default_path)
}

fn run_audit(cli: &Cli) -> Result<(Policy, AuditResult), (String, u8)> {
    let policy = resolve_policy(cli)?;
    let config = ogma_core::AuditConfig {
        policy: policy.clone(),
        ..Default::default()
    };
    ogma_core::run_audit(&cli.root, &config)
        .map(|result| (policy, result))
        .map_err(|e| (e.to_string(), 2))
}

/// Always write `<root>/.ogma/last-audit.json` (pretty JSON).
fn write_last_audit(root: &Path, result: &AuditResult) -> Result<(), (String, u8)> {
    let dir = root.join(".ogma");
    fs::create_dir_all(&dir)
        .map_err(|e| (format!("cannot create {}: {e}", dir.display()), 2))?;
    let path = dir.join("last-audit.json");
    fs::write(&path, ogma_report::render_json(result))
        .map_err(|e| (format!("cannot write {}: {e}", path.display()), 2))
}

fn write_output(out: Option<&Path>, rendered: &str) -> Result<(), (String, u8)> {
    match out {
        Some(path) => {
            fs::write(path, rendered)
                .map_err(|e| (format!("cannot write {}: {e}", path.display()), 2))?;
            Ok(())
        }
        None => {
            print!("{rendered}");
            Ok(())
        }
    }
}

fn cmd_init(cli: &Cli, force: bool) -> CliResult {
    let path = cli.root.join("ogma.toml");
    if path.exists() && !force {
        return Err((
            format!("{} already exists (use --force to overwrite)", path.display()),
            2,
        ));
    }
    fs::write(&path, ogma_policy::DEFAULT_POLICY_TOML)
        .map_err(|e| (format!("cannot write {}: {e}", path.display()), 2))?;
    println!("wrote {}", path.display());
    Ok(0)
}

fn cmd_detect(cli: &Cli) -> CliResult {
    let (_policy, result) = run_audit(cli)?;
    let mut out = String::new();
    out.push_str("Application\n");
    field(&mut out, "Name:", result.application_name.as_deref().unwrap_or("(unnamed)"));
    let (baseline, confidence) = match result.baseline.language.as_ref() {
        Some(lang) => (lang.code().to_string(), confidence_name(result.baseline.confidence)),
        None => ("undetected".to_string(), confidence_name(Confidence::Unknown)),
    };
    field(&mut out, "Baseline:", &format!("{baseline} ({confidence})"));
    let frameworks = if result.frameworks.is_empty() {
        "-".to_string()
    } else {
        result.frameworks.join(", ")
    };
    field(&mut out, "Frameworks:", &frameworks);
    out.push_str("\nLocales\n");
    let width = result
        .locales
        .iter()
        .map(|l| l.locale.canonical.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let mut locales: Vec<&LocaleReport> = result.locales.iter().collect();
    locales.sort_by(|a, b| a.locale.canonical.cmp(&b.locale.canonical));
    for report in locales {
        out.push_str(&format!(
            "  {:<width$}  {:>6.1}%\n",
            report.locale.canonical,
            report.coverage * 100.0,
            width = width
        ));
    }
    if result.locales.is_empty() {
        out.push_str("  (none detected)\n");
    }
    out.push_str("\nSurface\n");
    field(
        &mut out,
        "Units:",
        &result.linguistic_surface.total_units.to_string(),
    );
    field(
        &mut out,
        "User-facing:",
        &result.linguistic_surface.user_facing_occurrences.to_string(),
    );
    field(
        &mut out,
        "Hard-coded:",
        &result.linguistic_surface.hard_coded_occurrences.to_string(),
    );
    print!("{out}");
    Ok(0)
}

fn cmd_audit(cli: &Cli, format: ReportFormat, out: Option<&Path>) -> CliResult {
    let (policy, result) = run_audit(cli)?;
    write_last_audit(&cli.root, &result)?;
    let rendered = match format {
        ReportFormat::Text => ogma_report::render_text(&result, &policy),
        ReportFormat::Json => ogma_report::render_json(&result),
        ReportFormat::Sarif => ogma_report::render_sarif(&result),
    };
    write_output(out, &rendered)?;
    Ok(0)
}

fn cmd_report(cli: &Cli, format: MachineFormat, out: Option<&Path>) -> CliResult {
    let (_policy, result) = run_audit(cli)?;
    write_last_audit(&cli.root, &result)?;
    let rendered = match format {
        MachineFormat::Json => ogma_report::render_json(&result),
        MachineFormat::Sarif => ogma_report::render_sarif(&result),
    };
    write_output(out, &rendered)?;
    Ok(0)
}

fn all_violations(result: &AuditResult) -> Vec<&ogma_model::Violation> {
    result
        .violations
        .iter()
        .chain(result.locales.iter().flat_map(|l| l.violations.iter()))
        .collect()
}

fn cmd_check(cli: &Cli, format: CheckFormat) -> CliResult {
    let (policy, result) = run_audit(cli)?;
    write_last_audit(&cli.root, &result)?;

    let has_error = all_violations(&result)
        .iter()
        .any(|v| v.severity == Severity::Error);
    let compliant = result.overall == OverallStatus::Pass && !has_error;

    match format {
        CheckFormat::Text => {
            if compliant {
                println!("ogma: PASS");
            } else {
                print!("{}", ogma_report::render_text(&result, &policy));
            }
        }
        CheckFormat::Json => println!("{}", ogma_report::render_json(&result)),
        CheckFormat::Sarif => println!("{}", ogma_report::render_sarif(&result)),
    }

    Ok(if compliant { 0 } else { 1 })
}

fn cmd_explain(cli: &Cli, target: &str) -> CliResult {
    if target.starts_with("OGMA-") {
        print!("{}", ogma_report::explain(target));
        return Ok(0);
    }

    let audit_path = cli.root.join(".ogma").join("last-audit.json");
    let text = match fs::read_to_string(&audit_path) {
        Ok(text) => text,
        Err(_) => {
            eprintln!("no matching finding in .ogma/last-audit.json (run `ogma audit` first)");
            return Ok(2);
        }
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| (format!("cannot parse {}: {e}", audit_path.display()), 2))?;

    let (file_part, line_part) = split_path_line(target);
    let mut findings: Vec<(&str, &Value)> = Vec::new();
    collect_findings(&value, &mut findings);

    let mut printed = 0u32;
    for (locale, violation) in findings {
        let Some(vfile) = violation["file"].as_str() else {
            continue;
        };
        let path_matches = vfile == file_part
            || vfile.starts_with(&format!("{file_part}/"))
            || file_part.starts_with(&format!("{vfile}/"));
        if !path_matches {
            continue;
        }
        if let Some(line) = line_part {
            if violation["line"].as_u64() != Some(line as u64) {
                continue;
            }
        }
        let code = violation["code"].as_str().unwrap_or("?");
        let severity = violation["severity"].as_str().unwrap_or("?");
        let message = violation["message"].as_str().unwrap_or("");
        println!("{code}  {severity}  {}  {}", locale, message);
        if let Some(details) = violation["details"].as_array() {
            for detail in details {
                if let Some(d) = detail.as_str() {
                    println!("  {d}");
                }
            }
        }
        println!();
        print!("{}", ogma_report::explain(code));
        printed += 1;
    }

    if printed == 0 {
        eprintln!("no matching finding in .ogma/last-audit.json (run `ogma audit` first)");
        return Ok(2);
    }
    Ok(0)
}

fn split_path_line(target: &str) -> (String, Option<usize>) {
    if let Some((head, tail)) = target.rsplit_once(':') {
        if let Ok(line) = tail.parse::<usize>() {
            if !head.is_empty() {
                return (head.to_string(), Some(line));
            }
        }
    }
    (target.to_string(), None)
}

fn collect_findings<'a>(value: &'a Value, out: &mut Vec<(&'a str, &'a Value)>) {
    if let Some(violations) = value["violations"].as_array() {
        for violation in violations {
            out.push(("-", violation));
        }
    }
    if let Some(locales) = value["locales"].as_array() {
        for report in locales {
            let canonical = report["locale"]["canonical"].as_str().unwrap_or("-");
            if let Some(violations) = report["violations"].as_array() {
                for violation in violations {
                    out.push((canonical, violation));
                }
            }
        }
    }
}

fn cmd_policy_show(cli: &Cli) -> CliResult {
    match policy_file(cli) {
        Some(path) => {
            let text = fs::read_to_string(&path)
                .map_err(|e| (format!("cannot read {}: {e}", path.display()), 3))?;
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            Ok(0)
        }
        None => {
            println!(
                "# No policy file ({} / --policy); built-in defaults apply:",
                cli.root.join("ogma.toml").display()
            );
            print!("{}", ogma_policy::DEFAULT_POLICY_TOML);
            Ok(0)
        }
    }
}

fn cmd_policy_validate(cli: &Cli) -> CliResult {
    let policy = resolve_policy(cli)?;
    let _ = ogma_policy::merge(Policy::default(), policy);
    println!("policy OK");
    Ok(0)
}

fn cmd_locales(cli: &Cli) -> CliResult {
    let (_policy, result) = run_audit(cli)?;
    let mut locales = result.locales.clone();
    locales.sort_by(|a, b| a.locale.canonical.cmp(&b.locale.canonical));

    let width = locales
        .iter()
        .map(|l| l.locale.canonical.len())
        .max()
        .unwrap_or(6)
        .max(6);
    println!(
        "{:<width$}  {:>8}  {:<6}  {:<24}  {}",
        "Locale",
        "Coverage",
        "Native",
        "Fallbacks",
        "Violations",
        width = width
    );
    for report in &locales {
        println!(
            "{:<width$}  {:>7.1}%  {:<6}  {:<24}  {}",
            report.locale.canonical,
            report.coverage * 100.0,
            if report.native { "YES" } else { "NO" },
            fallback_chain(report),
            report.violations.len(),
            width = width
        );
    }
    if locales.is_empty() {
        println!("(no locales detected)");
    }
    Ok(0)
}

fn fallback_chain(report: &LocaleReport) -> String {
    let mut chain = vec![report.locale.canonical.clone()];
    let mut current = report.locale.canonical.clone();
    loop {
        let next = report
            .fallback_dependencies
            .iter()
            .find(|r| r.requested.canonical == current)
            .map(|r| r.target.canonical.clone());
        match next {
            Some(target) if !chain.contains(&target) => {
                chain.push(target.clone());
                current = target;
            }
            _ => break,
        }
    }
    if chain.len() == 1 {
        "-".to_string()
    } else {
        chain.join(" → ")
    }
}

fn cmd_strings(cli: &Cli) -> CliResult {
    let (_policy, result) = run_audit(cli)?;
    let surface = &result.linguistic_surface;

    println!("Linguistic surface\n");
    field_stdout("Units:", &surface.total_units.to_string());
    field_stdout(
        "User-facing occurrences:",
        &surface.user_facing_occurrences.to_string(),
    );
    field_stdout(
        "Hard-coded occurrences:",
        &surface.hard_coded_occurrences.to_string(),
    );

    println!("\nBy kind\n");
    if surface.by_kind.is_empty() {
        println!("  (none)");
    } else {
        for (kind, count) in &surface.by_kind {
            println!("  {kind:<20}{count}");
        }
    }

    let mut by_file: BTreeMap<String, usize> = BTreeMap::new();
    for violation in all_violations(&result) {
        if violation.code == codes::HARD_CODED_STRING {
            if let Some(file) = &violation.file {
                *by_file.entry(file.to_string_lossy().into_owned()).or_default() += 1;
            }
        }
    }
    println!("\nTop files by hard-coded count\n");
    if by_file.is_empty() {
        println!("  (none)");
    } else {
        let mut ranked: Vec<(&String, &usize)> = by_file.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (file, count) in ranked.iter().take(10) {
            println!("  {count:>5}  {file}");
        }
    }

    println!(
        "\n(per-catalogue unit counts and per-extraction-rule breakdown are not part of the audit summary)"
    );
    Ok(0)
}

fn cmd_install_hook(cli: &Cli) -> CliResult {
    match ogma_integrations::install_pre_commit_hook(&cli.root) {
        Ok(path) => {
            println!("installed pre-commit hook at {}", path.display());
            Ok(0)
        }
        Err(e) => Err((e.0, 2)),
    }
}

fn field(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("  {label:<14}{value}\n"));
}

fn field_stdout(label: &str, value: &str) {
    println!("  {label:<24}{value}");
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Proven => "PROVEN",
        Confidence::Likely => "LIKELY",
        Confidence::Unknown => "UNKNOWN",
    }
}
