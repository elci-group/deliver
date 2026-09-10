//! `defail` — run the reference scenarios from the command line, and manage
//! the persisted knowledge bank.

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use defail::demo::{self, ScenarioOutput};
use defail::json;
use defail::knowledge::KnowledgeBase;
use defail::store::{Backend, KnowledgeStore, StoreError};

const USAGE: &str = "\
defail — Deterministic Embedded Failure Addressing Inference Logic

USAGE:
  defail demo [baker|provider] [FLAGS]
  defail kb show <PATH>

SCENARIOS:
  demo                run all reference scenarios
  demo baker          baker; baking soda found missing early (window open)
  demo baker --late   baker; found missing after the batter is committed
  demo provider       provider rate limit; fallback recovers
  demo provider --degraded
                      provider B degraded too; escalates and gates downstream

FLAGS:
  --late             discover the baker failure after the window expires
  --degraded         make provider B unusable as well
  --json             after the human report, print each failure report as
                     versioned JSON (defail-report/1, trace included);
                     with `kb show`, print the bank as defail-kb/2 JSON
  --save-kb <PATH>   persist the knowledge learned by the scenarios; PATH
                     must be non-empty and stay inside the current directory
                     (no `..` above it); path creation uses bank, with a
                     PATH-free std::fs fallback

KNOWLEDGE:
  kb show <PATH>     print the records stored in a knowledge bank";

struct Flags {
    late: bool,
    degraded: bool,
    json: bool,
    save_kb: Option<PathBuf>,
}

fn main() -> ExitCode {
    let mut flags = Flags {
        late: false,
        degraded: false,
        json: false,
        save_kb: None,
    };
    let mut positional: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--late" => flags.late = true,
            "--degraded" => flags.degraded = true,
            "--json" => flags.json = true,
            "--save-kb" => match args.next() {
                Some(path) => match checked_save_path(&path) {
                    Ok(path) => flags.save_kb = Some(path),
                    Err(reason) => {
                        eprintln!("--save-kb: {reason}\n\n{USAGE}");
                        return ExitCode::from(2);
                    }
                },
                None => {
                    eprintln!("--save-kb requires a path\n\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" | "help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => positional.push(other.to_string()),
        }
    }

    let parsed: Vec<&str> = positional.iter().map(String::as_str).collect();
    let scenarios: Vec<(&str, ScenarioOutput)> = match parsed.as_slice() {
        [] | ["demo"] => vec![
            (
                "baker — failure discovered early (substitution window open)",
                demo::run_baker(false),
            ),
            (
                "baker — failure discovered late (substitution window expired)",
                demo::run_baker(true),
            ),
            (
                "provider — rate-limited (recovers via fallback)",
                demo::run_provider(true),
            ),
            (
                "provider — degraded (escalates, gate blocks downstream)",
                demo::run_provider(false),
            ),
        ],
        ["demo", "baker"] => {
            let out = demo::run_baker(flags.late);
            vec![(scenario_name_baker(flags.late), out)]
        }
        ["demo", "provider"] => {
            let out = demo::run_provider(!flags.degraded);
            vec![(scenario_name_provider(flags.degraded), out)]
        }
        ["kb", "show", path] => return kb_show(path, flags.json),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    for (name, out) in &scenarios {
        println!("== {name} ==");
        println!("{out}");
        if flags.json {
            for step in &out.steps {
                if let Some(report) = &step.report {
                    println!(
                        "{}",
                        json::object(&[
                            ("step", json::quote(&step.address)),
                            ("report", report.to_json()),
                        ])
                    );
                }
            }
        }
    }

    if let Some(path) = &flags.save_kb {
        match save_knowledge(path, scenarios.iter().map(|(_, out)| &out.kb)) {
            Ok(backend) => println!("Knowledge bank saved to {} via {backend}", path.display()),
            Err(err) => {
                eprintln!("failed to save knowledge bank to {}: {err}", path.display());
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn scenario_name_baker(late: bool) -> &'static str {
    if late {
        "baker — failure discovered late (substitution window expired)"
    } else {
        "baker — failure discovered early (substitution window open)"
    }
}

fn scenario_name_provider(degraded: bool) -> &'static str {
    if degraded {
        "provider — degraded (escalates, gate blocks downstream)"
    } else {
        "provider — rate-limited (recovers via fallback)"
    }
}

fn save_knowledge<'a>(
    path: &Path,
    banks: impl IntoIterator<Item = &'a KnowledgeBase>,
) -> Result<Backend, StoreError> {
    let mut merged = KnowledgeBase::new();
    for bank in banks {
        merged.absorb(bank);
    }
    KnowledgeStore::new(path).save(&merged)
}

/// Path discipline for `--save-kb`: the target must be non-empty and its
/// normalized form must stay inside the current directory — no `..` above
/// it, no absolute path outside it. (This is lexical: a symlink inside the
/// tree pointing outside is followed by the OS, not by this check.)
fn checked_save_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("path is empty".into());
    }
    let cwd = std::env::current_dir()
        .map_err(|err| format!("cannot resolve the current directory: {err}"))?;
    let path = Path::new(raw);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let normalized = normalize(&absolute);
    if !normalized.starts_with(&cwd) {
        return Err(format!("`{raw}` escapes the current directory"));
    }
    Ok(normalized)
}

/// Lexically resolve `.` and `..` components (without touching the filesystem).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn kb_show(path: &str, json_format: bool) -> ExitCode {
    match KnowledgeStore::new(path).load() {
        Ok(kb) => {
            if json_format {
                println!("{}", kb.to_json());
            } else if kb.is_empty() {
                println!("(knowledge bank at {path} is empty)");
            } else {
                for record in kb.to_records() {
                    println!("{record}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("failed to load knowledge bank from {path}: {err}");
            ExitCode::from(1)
        }
    }
}
