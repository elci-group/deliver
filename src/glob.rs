use std::path::{Path, PathBuf};

/// Expand a glob-like pattern into concrete file paths relative to base.
pub fn expand_paths(pattern: &str, base: &Path) -> Vec<PathBuf> {
    if pattern.contains('*') || pattern.contains('?') {
        expand_pattern(pattern, base)
    } else {
        vec![base.join(pattern)]
    }
}

fn expand_pattern(pattern: &str, base: &Path) -> Vec<PathBuf> {
    let full_pattern = base.join(pattern);
    let mut matches = Vec::new();

    if let Ok(glob_results) = glob::glob(&full_pattern.to_string_lossy()) {
        for entry in glob_results.flatten() {
            if entry.is_file() {
                matches.push(entry);
            }
        }
    }

    matches
}
