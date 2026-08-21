mod cli;
mod report;

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell as ClapShell};
use cli::{Args, Commands, OutputFormat, Shell};
use deliver::{expand_paths, quick_check_files, Spec};
use report::{render_junit, render_sarif, render_text, Progress, TextStyle};
use std::{fs, io, process};

fn main() {
    let args = Args::parse();

    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            Commands::Init { output } => {
                handle_init(&output);
                return;
            }
            Commands::Validate { spec } => {
                handle_validate(&spec);
                return;
            }
            Commands::Completions { shell } => {
                handle_completions(shell);
                return;
            }
            Commands::Schema => {
                handle_schema();
                return;
            }
        }
    }

    let progress = Progress::start(args.progress, args.format, "checking deliverables");

    let validation = build_report(&args);
    progress.finish();

    let validation_report = match validation {
        Ok(report) => report,
        Err(error) => {
            eprintln!("deliver: {}", error);
            process::exit(2);
        }
    };

    match args.format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&validation_report).unwrap()
            );
        }
        OutputFormat::Text => {
            render_text(&validation_report, &TextStyle::detect(args.color));
        }
        OutputFormat::Sarif => {
            println!("{}", render_sarif(&validation_report));
        }
        OutputFormat::Junit => {
            print!("{}", render_junit(&validation_report));
        }
    }

    if args.strict && !validation_report.pass {
        process::exit(1);
    }
}

fn handle_init(output: &std::path::Path) {
    let default_spec = r#"# Deliver spec file
# This file defines validation checks for your project

# File checks
[[file]]
path = "README.md"
required = true
min_size_bytes = 100
encoding = "utf-8"
forbid_symlinks = false
require_license_header = "MIT"

# Command checks
[[command]]
name = "cargo test"
cmd = ["cargo", "test"]
expect_exit = 0
timeout_secs = 300

# Directory checks
[[directory]]
path = "src"
required = true
forbid_empty = true

# Git checks
[git]
no_uncommitted_changes = false
no_untracked_files = false
"#;

    if output.exists() {
        eprintln!("Error: Spec file '{}' already exists", output.display());
        process::exit(1);
    }

    if let Err(e) = fs::write(output, default_spec) {
        eprintln!("Error writing spec file: {}", e);
        process::exit(1);
    }

    println!("Created spec file: {}", output.display());
    println!("Edit it to define your validation checks, then run:");
    println!("  deliver --spec {}", output.display());
}

fn handle_validate(spec_path: &std::path::Path) {
    match Spec::load_file(spec_path) {
        Ok(spec) => {
            println!(
                "✓ spec is valid ({} file, {} command, {} directory check(s))",
                spec.files.len(),
                spec.commands.len(),
                spec.directories.len()
            );
        }
        Err(e) => {
            eprintln!("✗ Spec validation failed: {}", e);
            process::exit(1);
        }
    }
}

fn handle_schema() {
    println!(
        "{}",
        serde_json::to_string_pretty(&deliver::spec_json_schema()).unwrap()
    );
}

fn handle_completions(shell: Shell) {
    let clap_shell = match shell {
        Shell::Bash => ClapShell::Bash,
        Shell::Zsh => ClapShell::Zsh,
        Shell::Fish => ClapShell::Fish,
    };

    let mut cmd = Args::command();
    generate(clap_shell, &mut cmd, "deliver", &mut io::stdout());
}

fn resolve_jobs(requested: usize) -> usize {
    if requested == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        requested
    }
}

fn build_report(args: &Args) -> Result<deliver::Report, String> {
    let jobs = resolve_jobs(args.jobs);

    if let Some(json) = &args.json {
        let spec = Spec::from_json(json)
            .map_err(|e| format!("failed to parse JSON spec: {}; check --json syntax", e))?;
        return Ok(spec.validate_with_jobs(&args.base, jobs));
    }

    if let Some(spec_path) = &args.spec {
        let spec = Spec::load_file(spec_path)?;
        return Ok(spec.validate_with_jobs(&args.base, jobs));
    }

    if !args.files.is_empty() {
        let paths = args
            .files
            .iter()
            .flat_map(|pattern| expand_paths(pattern, &args.base))
            .collect::<Vec<_>>();
        let checks = quick_check_files(&paths, &args.base);
        let pass = checks.iter().all(|check| check.pass);
        return Ok(deliver::Report {
            pass,
            checks,
            duration_ms: 0,
        });
    }

    Err("nothing to check. Provide --spec, --json, or --file.".to_string())
}
