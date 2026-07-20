mod cli;
mod report;

use clap::Parser;
use cli::{Args, OutputFormat};
use deliver::{expand_paths, quick_check_files, Spec};
use report::{render_text, Progress, TextStyle};
use std::{fs, process};

fn main() {
    let args = Args::parse();
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
    }

    if args.strict && !validation_report.pass {
        process::exit(1);
    }
}

fn build_report(args: &Args) -> Result<deliver::Report, String> {
    if let Some(json) = &args.json {
        let spec = Spec::from_json(json)
            .map_err(|e| format!("failed to parse JSON spec: {}; check --json syntax", e))?;
        return Ok(spec.validate(&args.base));
    }

    if let Some(spec_path) = &args.spec {
        let text = fs::read_to_string(spec_path)
            .map_err(|e| format!("failed to read spec {}: {}", spec_path.display(), e))?;
        let spec = if spec_path.extension().and_then(|e| e.to_str()) == Some("json") {
            Spec::from_json(&text)
                .map_err(|e| format!("failed to parse JSON spec: {}; check the spec file", e))?
        } else {
            Spec::from_toml(&text)
                .map_err(|e| format!("failed to parse TOML spec: {}; check the spec file", e))?
        };
        return Ok(spec.validate(&args.base));
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
