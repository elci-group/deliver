use regex::Regex;
use std::fs;
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
    let mut matches = Vec::new();
    let matcher = match glob_to_regex(pattern) {
        Ok(matcher) => matcher,
        Err(_) => return matches,
    };

    collect_matching_files(base, base, &matcher, &mut matches);
    matches
}

fn collect_matching_files(root: &Path, dir: &Path, pattern: &Regex, matches: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_matching_files(root, &path, pattern, matches);
        } else if path.is_file() {
            push_if_matching(root, &path, pattern, matches);
        }
    }
}

fn push_if_matching(root: &Path, path: &Path, pattern: &Regex, matches: &mut Vec<PathBuf>) {
    if let Ok(relative) = path.strip_prefix(root) {
        if pattern.is_match(&relative.to_string_lossy()) {
            matches.push(path.to_path_buf());
        }
    }
}

fn glob_to_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let mut regex = String::new();
    regex.push('^');
    for character in pattern.chars() {
        match character {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' => regex.push_str("\\."),
            '/' | '\\' => regex.push('/'),
            '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                regex.push('\\');
                regex.push(character);
            }
            _ => regex.push(character),
        }
    }
    regex.push('$');
    Regex::new(&regex)
}
