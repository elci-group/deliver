use clap::{Parser as ClapParser, ValueEnum};
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(name = "deliver")]
#[command(about = "Deterministic deliverable validator for agent workflows")]
#[command(
    long_about = "deliver verifies files, command results, and Git cleanliness from a small TOML or JSON spec. It is designed for agents that need deterministic proof that expected deliverables exist and that quality gates still pass."
)]
pub struct Args {
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
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
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
