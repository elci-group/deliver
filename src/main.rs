mod attest;
mod cli;
mod report;

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell as ClapShell};
use cli::{Args, AttestCommands, Commands, OutputFormat, Shell};
use deliver::{expand_paths, quick_check_files, Spec};
use report::{
    ndjson_check_line, ndjson_summary_line, render_github, render_junit, render_sarif, render_text,
    Progress, TextStyle,
};
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
            Commands::Keygen { out } => {
                handle_keygen(&out);
                return;
            }
            Commands::Attest { command } => {
                handle_attest(command);
                return;
            }
        }
    }

    let progress = Progress::start(args.progress, args.format, "checking deliverables");

    let validation = match args.format {
        OutputFormat::Ndjson => {
            let on_check = |check: &deliver::CheckResult| {
                println!("{}", ndjson_check_line(check));
            };
            build_report(&args, Some(&on_check))
        }
        _ => build_report(&args, None),
    };
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
        OutputFormat::Ndjson => {
            println!("{}", ndjson_summary_line(&validation_report));
        }
        OutputFormat::Github => {
            print!("{}", render_github(&validation_report));
        }
    }

    if let Some(attest_path) = &args.attest {
        // Attestation status goes to stderr so report output on stdout
        // stays machine-parseable.
        if !validation_report.pass {
            eprintln!("deliver: attestation skipped: the run did not pass");
        } else {
            let signing_key = match &args.attest_key {
                Some(key_path) => match attest::load_signing_key(key_path) {
                    Ok(key) => Some(key),
                    Err(error) => {
                        eprintln!("deliver: {}", error);
                        process::exit(2);
                    }
                },
                None => None,
            };
            let spec_input = match (&args.spec, &args.json) {
                (_, Some(json)) => attest::SpecInput::Inline(json.clone()),
                (Some(path), None) => match fs::read_to_string(path) {
                    Ok(text) => attest::SpecInput::File {
                        path: path.clone(),
                        text,
                    },
                    Err(error) => {
                        eprintln!("deliver: failed to read spec for attestation: {}", error);
                        process::exit(2);
                    }
                },
                _ => attest::SpecInput::None,
            };
            let git_commit = deliver::incremental::head_commit(&args.base).ok();
            if let Err(error) = attest::write_bundle(
                &validation_report,
                &args.base,
                &spec_input,
                git_commit.as_deref(),
                attest_path,
                signing_key.as_ref(),
            ) {
                eprintln!("deliver: {}", error);
                process::exit(2);
            }
            eprintln!("attestation written to {}", attest_path.display());
        }
    }

    if args.strict && !validation_report.pass {
        process::exit(1);
    }
}

fn handle_keygen(prefix: &str) {
    match attest::keygen(prefix) {
        Ok((key_path, pub_path)) => {
            println!("wrote private key: {}", key_path.display());
            println!("wrote public key:  {}", pub_path.display());
            println!(
                "Pass --attest-key {} to sign attestations; share {} with verifiers.",
                key_path.display(),
                pub_path.display()
            );
        }
        Err(error) => {
            eprintln!("deliver: {}", error);
            process::exit(1);
        }
    }
}

fn handle_attest(command: AttestCommands) {
    let AttestCommands::Verify { path, pub_key } = command;
    match attest::verify_bundle(&path, pub_key.as_deref()) {
        Ok(verdict) => {
            println!("✓ {} ({})", verdict, path.display());
        }
        Err(error) => {
            eprintln!("✗ attestation verification failed: {}", error);
            process::exit(1);
        }
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

# A conditional check: skipped (not failed) while the condition
# does not hold, e.g. before Cargo.toml exists.
# [[file]]
# path = "Cargo.lock"
# when = { file_exists = "Cargo.toml" }

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
    println!("  deliver --spec {} --allow-commands", output.display());
}

fn handle_validate(spec_path: &std::path::Path) {
    match Spec::load_file(spec_path) {
        Ok(spec) => {
            println!(
                "✓ spec is valid ({} file, {} command, {} directory, {} plugin, {} sbom check(s))",
                spec.files.len(),
                spec.commands.len(),
                spec.directories.len(),
                spec.plugins.len(),
                usize::from(spec.sbom.is_some())
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

fn build_report(
    args: &Args,
    on_result: Option<&(dyn Fn(&deliver::CheckResult) + Send + Sync)>,
) -> Result<deliver::Report, String> {
    let jobs = resolve_jobs(args.jobs);
    let options = deliver::ValidateOptions {
        incremental: args.incremental,
        fail_fast: args.fail_fast,
        cache_dir: args.cache_dir.clone().or_else(|| {
            if args.incremental {
                deliver::cache::default_dir()
            } else {
                None
            }
        }),
    };

    if let Some(json) = &args.json {
        let spec = Spec::from_json(json)
            .map_err(|e| format!("failed to parse JSON spec: {}; check --json syntax", e))?;
        require_command_trust(&spec, args.allow_commands)?;
        return run_spec(&spec, &args.base, jobs, &options, on_result);
    }

    if let Some(spec_path) = &args.spec {
        let spec = Spec::load_file(spec_path)?;
        require_command_trust(&spec, args.allow_commands)?;
        return run_spec(&spec, &args.base, jobs, &options, on_result);
    }

    if !args.files.is_empty() {
        let paths = args
            .files
            .iter()
            .flat_map(|pattern| expand_paths(pattern, &args.base))
            .collect::<Vec<_>>();
        let checks = quick_check_files(&paths, &args.base);
        if let Some(on_result) = on_result {
            for check in &checks {
                on_result(check);
            }
        }
        let pass = checks.iter().all(|check| check.pass);
        return Ok(deliver::Report {
            pass,
            checks,
            duration_ms: 0,
        });
    }

    Err("nothing to check. Provide --spec, --json, or --file.".to_string())
}

fn run_spec(
    spec: &Spec,
    base: &std::path::Path,
    jobs: usize,
    options: &deliver::ValidateOptions,
    on_result: Option<&(dyn Fn(&deliver::CheckResult) + Send + Sync)>,
) -> Result<deliver::Report, String> {
    match on_result {
        Some(on_result) => spec.validate_with_options_and_events(base, jobs, options, on_result),
        None => spec.validate_with_options(base, jobs, options),
    }
}

fn require_command_trust(spec: &Spec, allow_commands: bool) -> Result<(), String> {
    if !allow_commands && (!spec.commands.is_empty() || !spec.plugins.is_empty()) {
        Err("spec contains command or plugin checks; review the spec and its extends chain, then pass --allow-commands to execute them".to_string())
    } else {
        Ok(())
    }
}
