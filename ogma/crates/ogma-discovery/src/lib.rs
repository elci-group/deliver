//! Deterministic repository traversal and file classification for Ogma.
//!
//! [`scan`] walks a repository without following symlinks, applies a minimal
//! subset of `.gitignore` semantics, classifies every regular file, hashes its
//! contents (FNV-1a 64-bit, first [`MAX_HASH_BYTES`]), and returns a
//! [`RepositoryIndex`] whose `files` vector is always sorted by path so that
//! identical inputs produce byte-identical serialisations.
//!
//! Classification is strictly first-match-wins in the order documented on
//! [`FileClass`]. Notable consequences:
//!
//! - Files with unknown or missing extensions that are not hidden are
//!   classified as [`FileClass::Ignore`] to keep noise out of the index.
//! - `.toml`/`.yaml`/`.yml`/`.json`/`.xml`/`.ini` files are
//!   [`FileClass::Resource`] before [`FileClass::Config`] is considered, so a
//!   manifest such as `package.json` indexes as `Resource`; use
//!   [`RepositoryIndex::manifests`] to find manifests regardless of class.
//! - Symlink entries are skipped entirely (never followed, never indexed), so
//!   a symlinked file appears zero times rather than being deduplicated.
//! - Header extension `.h` is reported as [`ProgLanguage::C`]; it may equally
//!   be a C++ or Objective-C header, which static extension matching cannot
//!   distinguish.
//! - `.gitignore` negation patterns (`!`) are unsupported and treated as
//!   ignored lines.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

/// Upper bound on bytes sniffed from a file for generated-code marker
/// detection.
pub const MAX_SNIFF_BYTES: usize = 4096;

/// Upper bound on bytes fed to the content hash for files larger than 1 MiB.
pub const MAX_HASH_BYTES: usize = 1_048_576;

/// Repository content class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    Source,
    Test,
    Resource,
    Asset,
    Config,
    Generated,
    Vendor,
    Documentation,
    Ignore,
}

/// Detected programming language of a source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Java,
    Kotlin,
    Swift,
    ObjectiveC,
    C,
    Cpp,
    CSharp,
    Go,
    Php,
    Ruby,
    Dart,
    Shell,
    Html,
    Css,
    Template,
    Config,
    Unknown,
}

/// One classified file in a scanned repository.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexedFile {
    /// Repository-relative path (POSIX-style separators, relative to scan root).
    pub path: PathBuf,
    pub class: FileClass,
    pub language: ProgLanguage,
    pub size: u64,
    /// FNV-1a 64-bit hash of file contents (first MiB only for files > 1 MiB).
    pub hash: u64,
}

/// The full deterministic index of a repository.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RepositoryIndex {
    pub root: PathBuf,
    /// Always sorted by `path`; deterministic order.
    pub files: Vec<IndexedFile>,
}

/// Error returned by [`scan`] when the root does not exist or is not a
/// directory. Individual unreadable files are skipped, not errors.
#[derive(Clone, Debug)]
pub struct ScanError(pub String);

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scan error: {}", self.0)
    }
}

impl std::error::Error for ScanError {}

impl RepositoryIndex {
    /// Files of the given class, sorted by path.
    pub fn by_class(&self, class: FileClass) -> Vec<&IndexedFile> {
        self.files.iter().filter(|f| f.class == class).collect()
    }

    /// Files of the given programming language, sorted by path.
    pub fn with_language(&self, language: ProgLanguage) -> Vec<&IndexedFile> {
        self.files.iter().filter(|f| f.language == language).collect()
    }

    /// Manifest files (package.json, Cargo.toml, pubspec.yaml, Gemfile,
    /// Gemfile.lock, composer.json, go.mod, go.sum, pom.xml, build.gradle,
    /// *.csproj, *.sln) at any depth, sorted by path.
    pub fn manifests(&self) -> Vec<&IndexedFile> {
        self.files.iter().filter(|f| is_manifest_name(&file_name(&f.path))).collect()
    }
}

/// Directories never descended into during traversal.
const SKIP_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "node_modules", "target", "dist", "out", ".next", ".nuxt",
    "coverage", "__pycache__", ".idea",
];

/// Scan `root`, returning a deterministically ordered index of all regular
/// files. Symlinks are never followed; entries under VCS/dependency/build
/// directories and matched by `.gitignore` rules are pruned. Files that
/// cannot be read are skipped. Errors only when `root` is missing or not a
/// directory.
pub fn scan(root: &Path) -> Result<RepositoryIndex, ScanError> {
    let meta = fs::metadata(root)
        .map_err(|e| ScanError(format!("cannot access {}: {e}", root.display())))?;
    if !meta.is_dir() {
        return Err(ScanError(format!("{} is not a directory", root.display())));
    }

    let patterns = load_ignore_patterns(root);
    let mut files = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| keep_entry(e, root, &patterns));

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.depth() == 0 || !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| entry.path().to_path_buf());

        let sniff = match read_file_limited(entry.path(), MAX_SNIFF_BYTES) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let hash = match hash_file(entry.path()) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let class = classify(&rel, Some(&sniff));
        let language = if class == FileClass::Source {
            detect_language(&rel, sniff.lines().next())
        } else {
            ProgLanguage::Unknown
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        files.push(IndexedFile { path: rel, class, language, size, hash });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);

    Ok(RepositoryIndex { root: root.to_path_buf(), files })
}

/// Language detection from extension and optional shebang first line.
pub fn detect_language(path: &Path, first_line: Option<&str>) -> ProgLanguage {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lang = match ext.to_ascii_lowercase().as_str() {
            "rs" => ProgLanguage::Rust,
            "js" | "jsx" | "mjs" | "cjs" => ProgLanguage::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => ProgLanguage::TypeScript,
            "py" => ProgLanguage::Python,
            "java" => ProgLanguage::Java,
            "kt" | "kts" => ProgLanguage::Kotlin,
            "swift" => ProgLanguage::Swift,
            "m" | "mm" => ProgLanguage::ObjectiveC,
            "c" => ProgLanguage::C,
            "h" => ProgLanguage::C,
            "cc" | "cpp" | "cxx" => ProgLanguage::Cpp,
            "cs" => ProgLanguage::CSharp,
            "go" => ProgLanguage::Go,
            "php" => ProgLanguage::Php,
            "rb" => ProgLanguage::Ruby,
            "dart" => ProgLanguage::Dart,
            "sh" | "bash" | "zsh" => ProgLanguage::Shell,
            "html" | "htm" => ProgLanguage::Html,
            "css" | "scss" | "sass" | "less" => ProgLanguage::Css,
            "vue" | "svelte" | "erb" | "hbs" | "handlebars" | "mustache" | "twig"
            | "jinja" | "jinja2" | "tpl" => ProgLanguage::Template,
            "json" | "yaml" | "yml" | "toml" | "xml" | "ini" => ProgLanguage::Config,
            _ => ProgLanguage::Unknown,
        };
        if lang != ProgLanguage::Unknown {
            return lang;
        }
    }

    if let Some(line) = first_line {
        if let Some(rest) = line.strip_prefix("#!") {
            let mut tokens = rest.trim().split_whitespace();
            let mut base = tokens
                .next()
                .and_then(|t| t.rsplit('/').next())
                .unwrap_or("");
            if base == "env" {
                base = tokens.next().unwrap_or("");
            }
            let base = base.rsplit('/').next().unwrap_or(base);
            return match base {
                "python" | "python2" | "python3" => ProgLanguage::Python,
                "node" => ProgLanguage::JavaScript,
                "bash" | "sh" | "zsh" | "dash" | "ksh" => ProgLanguage::Shell,
                "ruby" => ProgLanguage::Ruby,
                "php" => ProgLanguage::Php,
                _ => ProgLanguage::Unknown,
            };
        }
    }

    ProgLanguage::Unknown
}

/// Read a file with a byte bound (for pathological files); returns full
/// content when smaller than `max_bytes`, else the first `max_bytes` as
/// lossy UTF-8.
pub fn read_file_limited(path: &Path, max_bytes: usize) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; max_bytes + 1];
    let mut filled = 0usize;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    let truncated = filled > max_bytes;
    buf.truncate(if truncated { max_bytes } else { filled });
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// FNV-1a 64-bit over at most [`MAX_HASH_BYTES`] of the file's contents.
fn hash_file(path: &Path) -> io::Result<u64> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_HASH_BYTES as u64).read_to_end(&mut bytes)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in &bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(hash)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn lower_segments(path: &Path) -> Vec<String> {
    path.iter()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').and_then(|(_, e)| if e.is_empty() { None } else { Some(e) })
}

fn any_segment(segments: &[String], names: &[&str]) -> bool {
    segments.iter().any(|s| names.contains(&s.as_str()))
}

/// Classify a repository-relative path. `sniff` is the first
/// [`MAX_SNIFF_BYTES`] of file content, used only for generated-code marker
/// detection. First match wins, in `FileClass` declaration order.
fn classify(rel: &Path, sniff: Option<&str>) -> FileClass {
    let segments = lower_segments(rel);
    let name = file_name(rel).to_ascii_lowercase();
    let ext = ext_of(&name).unwrap_or("").to_string();

    if any_segment(&segments, &["vendor", "third_party", "third-party", "extern", "external", "deps"]) {
        return FileClass::Vendor;
    }

    if ["md", "rst", "txt", "adoc"].contains(&ext.as_str())
        || any_segment(&segments, &["docs", "doc"])
        || ["readme", "license", "licence", "changelog", "contributing"]
            .iter()
            .any(|p| name.starts_with(p))
    {
        return FileClass::Documentation;
    }

    if any_segment(&segments, &["tests", "test", "__tests__", "spec"])
        || name.contains(".test.")
        || name.contains(".spec.")
        || (name.starts_with("test_") && ext == "py")
        || name.ends_with("_test.go")
        || name.ends_with("_tests.rs")
        || (name.starts_with("test_") && ext == "rs")
    {
        return FileClass::Test;
    }

    if name.contains(".generated.")
        || name.contains("_generated.")
        || name.ends_with(".pb.go")
        || name.ends_with(".pb.cc")
        || name.ends_with(".pb.h")
        || sniff.is_some_and(has_generated_marker)
    {
        return FileClass::Generated;
    }

    if [
        "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "bmp", "mp3", "mp4", "wav", "woff",
        "woff2", "ttf", "otf", "eot", "pdf",
    ]
    .contains(&ext.as_str())
    {
        return FileClass::Asset;
    }

    if [
        "json", "yaml", "yml", "toml", "po", "pot", "arb", "resx", "strings", "stringsdict",
        "properties", "csv", "xliff", "xlf", "xml", "ini",
    ]
    .contains(&ext.as_str())
        || any_segment(
            &segments,
            &[
                "locales", "locale", "i18n", "l10n", "translations", "translation", "langs",
                "lang", "resources", "res", "strings",
            ],
        )
    {
        return FileClass::Resource;
    }

    if ["toml", "yaml", "yml", "ini", "cfg", "conf", "lock"].contains(&ext.as_str())
        || is_manifest_name(&name)
        || any_segment(&segments, &["config", "configs", ".config"])
        || name.starts_with('.')
    {
        return FileClass::Config;
    }

    const SOURCE_EXTS: &[&str] = &[
        "rs", "js", "jsx", "ts", "tsx", "mjs", "cjs", "py", "java", "kt", "kts", "swift", "m",
        "mm", "c", "h", "cc", "cpp", "cxx", "cs", "go", "php", "rb", "dart", "sh", "bash",
        "zsh", "html", "htm", "css", "scss", "sass", "less", "vue", "svelte", "erb", "hbs",
        "handlebars", "mustache", "twig", "jinja", "jinja2", "tpl", "lua", "r", "pl", "pm", "tcl",
    ];
    if SOURCE_EXTS.contains(&ext.as_str()) {
        return FileClass::Source;
    }

    FileClass::Ignore
}

fn has_generated_marker(sniff: &str) -> bool {
    sniff.lines().any(|line| {
        let t = line.trim_start();
        t.starts_with("// Code generated")
            || t.starts_with("# Code generated")
            || t.starts_with("/* Code generated")
            || t.starts_with("// <auto-generated")
            || t.starts_with("DO NOT EDIT")
    })
}

fn is_manifest_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "package.json"
            | "cargo.toml"
            | "pubspec.yaml"
            | "gemfile"
            | "gemfile.lock"
            | "composer.json"
            | "go.mod"
            | "go.sum"
            | "pom.xml"
            | "build.gradle"
    ) || name.ends_with(".csproj")
        || name.ends_with(".sln")
}

#[derive(Debug)]
struct IgnorePattern {
    pattern: String,
    dir_only: bool,
    anchored: bool,
    has_slash: bool,
    has_wildcard: bool,
}

fn load_ignore_patterns(root: &Path) -> Vec<IgnorePattern> {
    let mut patterns = Vec::new();
    for file in [root.join(".gitignore"), root.join(".git/info/exclude")] {
        if let Ok(content) = fs::read_to_string(&file) {
            for raw in content.lines() {
                let line = raw.trim_end();
                if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                    continue;
                }
                let (line, dir_only) = match line.strip_suffix('/') {
                    Some(l) => (l, true),
                    None => (line, false),
                };
                let (line, anchored) = match line.strip_prefix('/') {
                    Some(l) => (l, true),
                    None => (line, false),
                };
                if line.is_empty() {
                    continue;
                }
                patterns.push(IgnorePattern {
                    has_wildcard: line.contains('*') || line.contains('?'),
                    has_slash: line.contains('/'),
                    pattern: line.to_string(),
                    dir_only,
                    anchored,
                });
            }
        }
    }
    patterns
}

fn keep_entry(entry: &DirEntry, root: &Path, patterns: &[IgnorePattern]) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if entry.file_type().is_dir() && SKIP_DIRS.contains(&name.as_ref()) {
        return false;
    }
    let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
    !is_ignored(rel, entry.file_type().is_dir(), patterns)
}

fn is_ignored(rel: &Path, is_dir: bool, patterns: &[IgnorePattern]) -> bool {
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let segments = lower_segments_keep_case(rel);
    let name = file_name(rel);
    patterns.iter().any(|p| {
        if p.dir_only && !is_dir {
            return false;
        }
        if p.anchored {
            return wildcard_or_exact(&p.pattern, &rel_str);
        }
        if p.has_slash {
            if let Some(base) = p.pattern.strip_suffix("/**") {
                return rel_str == base || rel_str.starts_with(&format!("{base}/"));
            }
            return wildcard_or_exact(&p.pattern, &rel_str);
        }
        if p.has_wildcard {
            return wildcard_or_exact(&p.pattern, &name);
        }
        segments.iter().any(|s| s == &p.pattern)
    })
}

fn lower_segments_keep_case(path: &Path) -> Vec<String> {
    path.iter().map(|s| s.to_string_lossy().into_owned()).collect()
}

fn wildcard_or_exact(pattern: &str, text: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        wildcard(pattern.as_bytes(), text.as_bytes())
    } else {
        pattern == text
    }
}

fn wildcard(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'*') => wildcard(&pat[1..], text) || (!text.is_empty() && wildcard(pat, &text[1..])),
        Some(b'?') => !text.is_empty() && wildcard(&pat[1..], &text[1..]),
        Some(&c) => !text.is_empty() && text[0] == c && wildcard(&pat[1..], &text[1..]),
    }
}
