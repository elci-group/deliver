use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(name = "deliver")]
#[command(about = "Deterministic deliverable validator for agent workflows")]
#[command(
    long_about = "deliver verifies files, command results, and Git cleanliness from a small TOML or JSON spec. It is designed for agents that need deterministic proof that expected deliverables exist and that quality gates still pass."
)]
#[command(version)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to a TOML or JSON validation spec
    #[arg(short, long, value_name = "PATH")]
    pub spec: Option<PathBuf>,

    /// JSON spec string; overrides --spec when provided
    #[arg(long, value_name = "JSON")]
    pub json: Option<String>,

    /// Quick file existence check(s); supports simple * and ? glob patterns
    #[arg(long = "file", num_args = 1.., value_name = "PATTERN")]
    pub files: Vec<String>,

    /// Base directory for all relative paths
    #[arg(short, long, default_value = ".", value_name = "DIR")]
    pub base: PathBuf,

    /// Output format
    #[arg(short, long, value_enum, default_value = "text")]
    pub format: OutputFormat,

    /// ANSI color policy for text output
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorMode,

    /// Progress spinner policy; ignored for JSON output unless set to always
    #[arg(long, value_enum, default_value = "auto")]
    pub progress: ProgressMode,

    /// Exit non-zero if any check fails
    #[arg(long)]
    pub strict: bool,

    /// Run file/command/directory checks with up to this many worker
    /// threads (git checks always run sequentially, after). Default 1
    /// (sequential); pass e.g. 4 or 0 (all CPUs) to speed up specs with many
    /// independent checks. Concurrent command checks that share mutable
    /// state (e.g. the same build directory) may contend with each other.
    #[arg(short, long, default_value = "1", value_name = "N")]
    pub jobs: usize,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a new deliver spec file
    Init {
        /// Output path for the spec file
        #[arg(short, long, default_value = "deliver.toml", value_name = "PATH")]
        output: PathBuf,
    },
    /// Validate a spec file without running checks
    Validate {
        /// Path to the spec file to validate
        #[arg(value_name = "PATH")]
        spec: PathBuf,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Print the JSON Schema for the spec format
    Schema,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    /// SARIF 2.1.0, for GitHub code scanning and other SARIF-consuming CI annotators. Only failed checks are emitted as results.
    Sarif,
    /// JUnit XML, for CI test-result dashboards. Every check is emitted as a testcase.
    Junit,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}
