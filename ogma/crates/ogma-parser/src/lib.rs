//! Framework adapters and deterministic, line-based source analysis for Ogma.
//!
//! This crate is a lightweight rule engine, not a full parser: every finding
//! comes from fixed-order, line-oriented lexical rules with stable rule IDs.
//! All outputs are sorted deterministically and every file read is bounded
//! via [`ogma_discovery::read_file_limited`].
//!
//! Extraction scope (v1): translated-call rules record string-literal key
//! arguments; hard-coded rules record string literals passed directly to UI /
//! CLI / error APIs. `format!` / `write!` / `printf`-style internal builders
//! are intentionally NOT extracted — they are internal until their result is
//! passed to a UI rule, which is out of scope for v1.

use std::path::{Path, PathBuf};

use ogma_discovery::{read_file_limited, FileClass, ProgLanguage, RepositoryIndex};
use ogma_model::{codes, Confidence, Language, LocaleId, Severity, StringKind, StringOccurrence, Violation};

/// Per-file read bound for content analysis (1 MiB).
const MAX_READ_BYTES: usize = 1_048_576;

/// A detected application framework / adapter.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameworkInfo {
    /// Lower-case name: "react", "vue", "angular", "flutter", "django", "rails",
    /// "laravel", "rust-gui", "generic".
    pub name: String,
    /// i18n entry-point call prefixes this framework's translations use,
    /// e.g. ["t(", "$t(", "<Trans"].
    pub i18n_hooks: Vec<String>,
    pub confidence: Confidence,
}

/// A default-locale or fallback-locale declaration found in configuration.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocaleConfigHint {
    /// Repository-relative path of the file the hint came from.
    pub path: PathBuf,
    /// Dotted configuration key, e.g. "fallbackLng", "LANGUAGE_CODE".
    pub key: String,
    pub locale: LocaleId,
    /// true = primary/default locale declaration; false = fallback configuration.
    pub is_default: bool,
}

// ---------------------------------------------------------------------------
// Framework detection
// ---------------------------------------------------------------------------

/// Detect frameworks from manifests and source signatures. Always includes a
/// "generic" adapter (confidence Unknown) when any Source files exist.
/// Sorted by name; deterministic.
pub fn detect_frameworks(index: &RepositoryIndex, root: &Path) -> Vec<FrameworkInfo> {
    let mut out: Vec<FrameworkInfo> = Vec::new();

    // Manifest content, merged per manifest kind (any depth).
    let mut pkg = String::new();
    let mut pubspec = String::new();
    let mut gemfile = String::new();
    let mut composer = String::new();
    let mut cargo = String::new();
    for m in index.manifests() {
        let Some(content) = read_bounded(root, &m.path) else { continue };
        match file_name(&m.path).as_str() {
            "package.json" => {
                pkg.push('\n');
                pkg.push_str(&content);
            }
            "pubspec.yaml" => {
                pubspec.push('\n');
                pubspec.push_str(&content);
            }
            "gemfile" => {
                gemfile.push('\n');
                gemfile.push_str(&content);
            }
            "composer.json" => {
                composer.push('\n');
                composer.push_str(&content);
            }
            "cargo.toml" => {
                cargo.push('\n');
                cargo.push_str(&content);
            }
            _ => {}
        }
    }

    let any_source = index.files.iter().any(|f| f.class == FileClass::Source);
    let has_xliff = index.files.iter().any(|f| {
        matches!(
            f.path.extension().and_then(|e| e.to_str()).unwrap_or(""),
            "xliff" | "xlf"
        )
    });

    // Source-signature scans (bounded reads, early exit on first hit).
    let mut dollar_t_seen = false;
    let mut rust_tr_seen = false;
    if any_source {
        for f in index.by_class(FileClass::Source) {
            let Some(content) = read_bounded(root, &f.path) else { continue };
            if !dollar_t_seen && content.contains("$t(") {
                dollar_t_seen = true;
            }
            let ext = f.path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !rust_tr_seen && ext == "rs" && content.contains("tr!(") {
                rust_tr_seen = true;
            }
            if dollar_t_seen && rust_tr_seen {
                break;
            }
        }
    }

    // Django: settings.py mentioning django, or requirements.txt / pyproject.toml
    // mentioning django (neither is a manifest in the discovery index, so they
    // are located by file name across all classes).
    let mut django_seen = false;
    for f in &index.files {
        let name = file_name(&f.path);
        if name == "settings.py" || name == "requirements.txt" || name == "pyproject.toml" {
            if let Some(content) = read_bounded(root, &f.path) {
                if content.contains("django") {
                    django_seen = true;
                    break;
                }
            }
        }
    }

    let has = |hay: &str, needle: &str| hay.contains(needle);

    // react
    if has(&pkg, "\"react\"") && (has(&pkg, "i18next") || has(&pkg, "react-intl") || has(&pkg, "@lingui")) {
        out.push(FrameworkInfo {
            name: "react".to_string(),
            i18n_hooks: vec!["t(".into(), "i18n.t(".into(), "<Trans".into(), "useTranslation".into()],
            confidence: Confidence::Proven,
        });
    }
    // vue
    let vue_manifest = has(&pkg, "\"vue\"");
    if vue_manifest && (has(&pkg, "vue-i18n") || dollar_t_seen) {
        let confidence = if has(&pkg, "vue-i18n") {
            Confidence::Proven
        } else {
            Confidence::Likely
        };
        out.push(FrameworkInfo {
            name: "vue".to_string(),
            i18n_hooks: vec!["$t(".into(), "t(".into(), "v-t".into()],
            confidence,
        });
    }
    // angular
    if has(&pkg, "@angular/core") && (has(&pkg, "@ngx-translate") || has_xliff) {
        out.push(FrameworkInfo {
            name: "angular".to_string(),
            i18n_hooks: vec!["translate.instant(".into(), "setDefaultLang(".into()],
            confidence: Confidence::Proven,
        });
    }
    // flutter
    if has(&pubspec, "flutter:") && (has(&pubspec, "flutter_localizations") || has(&pubspec, "intl")) {
        out.push(FrameworkInfo {
            name: "flutter".to_string(),
            i18n_hooks: vec!["AppLocalizations".into(), "Intl.message".into()],
            confidence: Confidence::Proven,
        });
    }
    // django
    if django_seen {
        out.push(FrameworkInfo {
            name: "django".to_string(),
            i18n_hooks: vec!["_(".into(), "gettext".into(), "ngettext".into(), "{% trans".into(), "{% translate".into()],
            confidence: Confidence::Proven,
        });
    }
    // rails
    if has(&gemfile, "rails") {
        out.push(FrameworkInfo {
            name: "rails".to_string(),
            i18n_hooks: vec!["I18n.t(".into(), "t(".into()],
            confidence: Confidence::Proven,
        });
    }
    // laravel
    if has(&composer, "laravel/framework") {
        out.push(FrameworkInfo {
            name: "laravel".to_string(),
            i18n_hooks: vec!["__(".into(), "@lang".into(), "trans(".into()],
            confidence: Confidence::Proven,
        });
    }
    // rust-gui
    let rust_gui_base = ["egui", "dioxus", "gtk", "fltk", "iced"].iter().any(|d| has(&cargo, d));
    if rust_gui_base {
        let i18n_dep = ["rust-i18n", "fluent", "i18n-embed"].iter().any(|d| has(&cargo, d));
        if i18n_dep || rust_tr_seen {
            let confidence = if i18n_dep { Confidence::Proven } else { Confidence::Likely };
            out.push(FrameworkInfo {
                name: "rust-gui".to_string(),
                i18n_hooks: vec!["tr!(".into(), "t!(".into(), "fl!".into()],
                confidence,
            });
        }
    }
    // i18next without react/vue
    if has(&pkg, "i18next") && !out.iter().any(|f| f.name == "react") && !out.iter().any(|f| f.name == "vue") {
        out.push(FrameworkInfo {
            name: "i18next".to_string(),
            i18n_hooks: vec!["t(".into(), "i18next".into()],
            confidence: Confidence::Proven,
        });
    }
    // generic adapter
    if any_source {
        out.push(FrameworkInfo {
            name: "generic".to_string(),
            i18n_hooks: vec!["t(".into(), "gettext(".into(), "tr!(".into(), "__(".into()],
            confidence: Confidence::Unknown,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

// ---------------------------------------------------------------------------
// String extraction
// ---------------------------------------------------------------------------

/// Translated-call hooks for rule "i18n/t-call", longest first so the most
/// specific prefix wins at each position.
const T_CALL_HOOKS: &[&str] = &[
    "intl.formatMessage({ id:",
    "Intl.message(",
    "i18n.t(",
    "this.$t(",
    "this.t(",
    "I18n.t(",
    "$t(",
    "translate(",
    "gettext(",
    "ngettext(",
    "tr!(",
    "t!(",
    "t(",
    "__(",
    "_(",
];

/// Hard-coded set-text-ish call prefixes for rule "ui/set-text".
const SET_TEXT_HOOKS: &[&str] = &[
    ".set_placeholder(",
    ".setPlaceholder(",
    ".set_text(",
    ".setText(",
    ".set_label(",
    ".setLabel(",
    ".set_title(",
    ".setTitle(",
    ".config(text=",
    "label:",
];

/// Extract string occurrences from all Source-classified files.
/// Sorted by (file, line); deterministic.
pub fn extract_strings(index: &RepositoryIndex, root: &Path) -> Vec<StringOccurrence> {
    let mut out = Vec::new();
    for file in index.by_class(FileClass::Source) {
        let Some(content) = read_bounded(root, &file.path) else { continue };
        let ext = extension(&file.path);
        // Full markup handling for markup languages and JSX/TSX. For plain
        // .js/.ts files (which may still contain JSX) attribute rules always
        // apply, while the `>text<` node rule is gated on the line containing
        // a closing tag (`</`) so comparisons like `a > min < max` in plain
        // JavaScript are not misread as text nodes.
        let markup = matches!(ext.as_str(), "html" | "htm" | "jsx" | "tsx")
            || matches!(file.language, ProgLanguage::Html | ProgLanguage::Template);
        let markup_attrs_only = !markup
            && matches!(ext.as_str(), "js" | "mjs" | "cjs" | "ts" | "mts" | "cts");
        let mut in_embedded = false;
        for (lineno, line) in content.lines().enumerate() {
            let lineno = lineno + 1;
            let blocked = in_embedded || line.contains("<script") || line.contains("<style");
            extract_line(
                &mut out,
                &file.path,
                file.language,
                &ext,
                line,
                lineno,
                markup && !blocked,
                markup_attrs_only && !blocked,
            );
            if line.contains("<script") || line.contains("<style") {
                in_embedded = true;
            }
            if line.contains("</script>") || line.contains("</style>") {
                in_embedded = false;
            }
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    out
}

#[allow(clippy::too_many_arguments)]
fn extract_line(
    out: &mut Vec<StringOccurrence>,
    path: &Path,
    lang: ProgLanguage,
    ext: &str,
    line: &str,
    lineno: usize,
    markup: bool,
    markup_lite: bool,
) {
    let trimmed = line.trim_start();
    if trimmed.starts_with("import ")
        || trimmed.starts_with("import(")
        || trimmed.starts_with("require(")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("#import")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("using ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("package ")
        || trimmed.starts_with("include ")
        || trimmed.starts_with("extern ")
    {
        return;
    }

    // -- translated rules ---------------------------------------------------

    // i18n/t-call: literal first string argument of a translation entry point.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(hook) = T_CALL_HOOKS.iter().find(|h| bytes[i..].starts_with(h.as_bytes())) {
            let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            if prev_ok {
                if let Some((text, _, end)) = literal_after(line, i + hook.len()) {
                    if !text.is_empty() {
                        push(out, path, lineno, text, true, "i18n/t-call", StringKind::UserFacing, Confidence::Proven);
                    }
                    i = end.max(i + hook.len());
                    continue;
                }
            }
            i += hook.len();
            continue;
        }
        i += 1;
    }

    // i18n/trans-component: <Trans i18nKey="..." /> and `| translate` pipes.
    let mut pos = 0;
    while let Some(p) = find_token(bytes, b"i18nKey", pos) {
        if let Some((text, _, _)) = literal_after(line, p + b"i18nKey".len()) {
            if !text.is_empty() {
                push(out, path, lineno, text, true, "i18n/trans-component", StringKind::UserFacing, Confidence::Proven);
            }
        }
        pos = p + b"i18nKey".len();
    }
    for (text, end) in all_literals(line) {
        if !text.is_empty() && after_is_translate_pipe(&line[end..]) {
            push(out, path, lineno, text, true, "i18n/trans-component", StringKind::UserFacing, Confidence::Proven);
        }
    }

    // i18n/django-template: {% trans "..." %} / {% translate "..." %}.
    for token in ["{% translate", "{% trans"] {
        let mut pos = 0;
        while let Some(p) = find_token(bytes, token.as_bytes(), pos) {
            if let Some((text, _, _)) = literal_after(line, p + token.len()) {
                if !text.is_empty() {
                    push(out, path, lineno, text, true, "i18n/django-template", StringKind::UserFacing, Confidence::Proven);
                }
            }
            pos = p + token.len();
        }
    }

    // i18n/swift-nslocalized: NSLocalizedString("...", ...)
    if let Some(p) = find_token(bytes, b"NSLocalizedString(", 0) {
        if let Some((text, _, _)) = literal_after(line, p + b"NSLocalizedString(".len()) {
            if !text.is_empty() {
                push(out, path, lineno, text, true, "i18n/swift-nslocalized", StringKind::UserFacing, Confidence::Proven);
            }
        }
    }

    // -- hard-coded rules ---------------------------------------------------

    // ui/set-text
    let mut i = 0;
    while i < bytes.len() {
        if let Some(hook) = SET_TEXT_HOOKS.iter().find(|h| bytes[i..].starts_with(h.as_bytes())) {
            let boundary = hook.starts_with('.')
                || i == 0
                || !is_ident_byte(bytes[i - 1]);
            if boundary {
                if let Some((text, _, end)) = literal_after(line, i + hook.len()) {
                    push_hard(out, path, lineno, text, "ui/set-text", StringKind::UserFacing);
                    i = end.max(i + hook.len());
                    continue;
                }
            }
            i += hook.len();
            continue;
        }
        i += 1;
    }
    // SwiftUI Text("...") / Button("...", ...) — Swift only, not member calls.
    if lang == ProgLanguage::Swift {
        for token in ["Text(", "Button("] {
            let mut pos = 0;
            while let Some(p) = find_token(bytes, token.as_bytes(), pos) {
                let prev_ok = p == 0 || (!is_ident_byte(bytes[p - 1]) && bytes[p - 1] != b'.' && bytes[p - 1] != b'$');
                if prev_ok {
                    if let Some((text, _, _)) = literal_after(line, p + token.len()) {
                        push_hard(out, path, lineno, text, "ui/set-text", StringKind::UserFacing);
                    }
                }
                pos = p + token.len();
            }
        }
    }

    // ui/dialog — JS/TS alert/confirm/prompt with a literal.
    if matches!(lang, ProgLanguage::JavaScript | ProgLanguage::TypeScript) {
        for token in ["alert(", "confirm(", "prompt("] {
            let mut pos = 0;
            while let Some(p) = find_token(bytes, token.as_bytes(), pos) {
                let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
                if prev_ok {
                    if let Some((text, _, _)) = literal_after(line, p + token.len()) {
                        push_hard(out, path, lineno, text, "ui/dialog", StringKind::UserFacing);
                    }
                }
                pos = p + token.len();
            }
        }
    }

    // cli/print — language-gated print-family calls with a literal.
    let print_hooks: &[&str] = match lang {
        ProgLanguage::Rust => &["eprintln!(", "eprint!(", "println!(", "print!("],
        ProgLanguage::Go => &["fmt.Println(", "fmt.Print("],
        ProgLanguage::Python => &["print("],
        ProgLanguage::Shell | ProgLanguage::Php => &["echo "],
        _ => &[],
    };
    for hook in print_hooks {
        let mut pos = 0;
        while let Some(p) = find_token(bytes, hook.as_bytes(), pos) {
            let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
            if prev_ok {
                if let Some((text, _, _)) = literal_after(line, p + hook.len()) {
                    push_hard(out, path, lineno, text, "cli/print", StringKind::UserFacing);
                }
            }
            pos = p + hook.len();
        }
    }

    // ui/html-text (+ ui/aria-label). Full markup: text nodes + attributes.
    // Lite (plain .js/.ts that may contain JSX): attributes always; text nodes
    // only when the line carries a closing tag (`</`), which plain JavaScript
    // essentially never contains.
    if markup || markup_lite {
        let text_nodes = markup || (markup_lite && line.contains("</"));
        if text_nodes {
        let templated = line.contains("{{") || line.contains("{%") || line.contains("<%");
        if !templated {
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'>' {
                    let rest = &line[i + 1..];
                    let lt = rest.find('<').unwrap_or(rest.len());
                    let span = rest[..lt].trim();
                    if !span.is_empty()
                        && span.chars().filter(|c| c.is_alphabetic()).count() >= 2
                        && !span.contains('{')
                        && !span.contains('}')
                        && is_probably_linguistic(span)
                    {
                        push(out, path, lineno, span.to_string(), false, "ui/html-text", StringKind::UserFacing, Confidence::Likely);
                    }
                    i += 1 + lt;
                    continue;
                }
                i += 1;
            }
        }
        }
        for (attr, rule) in [
            ("placeholder", "ui/html-text"),
            ("alt", "ui/html-text"),
            ("title", "ui/html-text"),
            ("aria-label", "ui/aria-label"),
        ] {
            let mut pos = 0;
            while let Some(p) = find_token(bytes, attr.as_bytes(), pos) {
                let next_ok = p + attr.len() >= bytes.len() || !is_ident_byte(bytes[p + attr.len()]);
                if next_ok {
                    let mut q = p + attr.len();
                    while q < bytes.len() && bytes[q].is_ascii_whitespace() {
                        q += 1;
                    }
                    if q < bytes.len() && bytes[q] == b'=' {
                        if let Some((text, _, _)) = literal_after(line, q + 1) {
                            push_hard(out, path, lineno, text, rule, StringKind::UserFacing);
                        }
                    }
                }
                pos = p + attr.len();
            }
        }
    }

    // err/exception — DeveloperFacing, Proven (deterministic syntactic fact).
    let err_hooks: &[&str] = match lang {
        ProgLanguage::Rust => &["panic!(", "assert_eq!(", "assert!(", "bail!(", "anyhow!(", "format_err!("],
        _ => &[],
    };
    for hook in err_hooks {
        let mut pos = 0;
        while let Some(p) = find_token(bytes, hook.as_bytes(), pos) {
            let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
            if prev_ok {
                if let Some((text, _, _)) = literal_after(line, p + hook.len()) {
                    push_hard_conf(out, path, lineno, text, "err/exception", StringKind::DeveloperFacing, Confidence::Proven);
                }
            }
            pos = p + hook.len();
        }
    }
    // `throw new X("..."` — JS/Java/C#/C++.
    let mut pos = 0;
    while let Some(p) = find_token(bytes, b"throw new", pos) {
        let mut q = p + b"throw new".len();
        while q < bytes.len() && bytes[q].is_ascii_whitespace() {
            q += 1;
        }
        while q < bytes.len() && (is_ident_byte(bytes[q]) || bytes[q] == b'.') {
            q += 1;
        }
        if q < bytes.len() && bytes[q] == b'(' {
            if let Some((text, _, _)) = literal_after(line, q + 1) {
                push_hard_conf(out, path, lineno, text, "err/exception", StringKind::DeveloperFacing, Confidence::Proven);
            }
        }
        pos = p + b"throw new".len();
    }
    // `raise X("..."` — Python.
    if lang == ProgLanguage::Python {
        let mut pos = 0;
        while let Some(p) = find_token(bytes, b"raise ", pos) {
            let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
            if prev_ok {
                let mut q = p + b"raise ".len();
                while q < bytes.len() && (is_ident_byte(bytes[q]) || bytes[q] == b'.') {
                    q += 1;
                }
                if q < bytes.len() && bytes[q] == b'(' {
                    if let Some((text, _, _)) = literal_after(line, q + 1) {
                        push_hard_conf(out, path, lineno, text, "err/exception", StringKind::DeveloperFacing, Confidence::Proven);
                    }
                }
            }
            pos = p + b"raise ".len();
        }
    }
    // `Exception("...")` constructor form.
    let mut pos = 0;
    while let Some(p) = find_token(bytes, b"Exception(", pos) {
        let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
        if prev_ok {
            if let Some((text, _, _)) = literal_after(line, p + b"Exception(".len()) {
                push_hard_conf(out, path, lineno, text, "err/exception", StringKind::DeveloperFacing, Confidence::Proven);
            }
        }
        pos = p + b"Exception(".len();
    }

    let _ = ext;
}

fn push(
    out: &mut Vec<StringOccurrence>,
    path: &Path,
    line: usize,
    text: String,
    translated: bool,
    rule: &str,
    kind: StringKind,
    confidence: Confidence,
) {
    out.push(StringOccurrence {
        file: path.to_path_buf(),
        line,
        text,
        kind,
        confidence,
        translated,
        rule: rule.to_string(),
    });
}

fn push_hard(out: &mut Vec<StringOccurrence>, path: &Path, line: usize, text: String, rule: &str, kind: StringKind) {
    if is_probably_linguistic(&text) {
        push(out, path, line, text, false, rule, kind, Confidence::Likely);
    }
}

fn push_hard_conf(
    out: &mut Vec<StringOccurrence>,
    path: &Path,
    line: usize,
    text: String,
    rule: &str,
    kind: StringKind,
    confidence: Confidence,
) {
    if is_probably_linguistic(&text) {
        push(out, path, line, text, false, rule, kind, confidence);
    }
}

/// True when `text` looks like human language rather than a key, path, URL,
/// identifier, or encoded blob. Deliberately conservative: false positives
/// erode trust, and absence is how "unknown" is reported.
fn is_probably_linguistic(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() == 1 {
        return false;
    }
    if !t.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    if t.contains("://") || t.starts_with("www.") {
        return false;
    }
    if t.contains('/') && t.contains('.') && !t.contains(' ') {
        return false;
    }
    if !t.contains(' ')
        && t.chars().any(|c| c.is_ascii_alphabetic())
        && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return false; // env-var-looking UPPER_SNAKE
    }
    if !t.contains(' ')
        && t.contains('_')
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return false; // pure snake_case identifier (a KEY, not a message)
    }
    if t.len() >= 16 && !t.contains(' ') && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return false; // hex blob
    }
    if t.len() >= 24
        && !t.contains(' ')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    {
        return false; // base64 blob
    }
    true
}

// ---------------------------------------------------------------------------
// Locale-semantics analysis
// ---------------------------------------------------------------------------

/// Locale-semantics findings: locale-insensitive date/number formatting, RTL.
/// `rtl_required`: true when the policy requires an RTL locale.
/// Sorted by (file, line, code); deterministic.
pub fn analyse_semantics(index: &RepositoryIndex, root: &Path, rtl_required: bool) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in index.by_class(FileClass::Source) {
        let Some(content) = read_bounded(root, &file.path) else { continue };
        for (lineno, line) in content.lines().enumerate() {
            let lineno = lineno + 1;
            if let Some(snippet) = date_finding(line) {
                out.push(violation(codes::LOCALE_INSENSITIVE_DATE, &file.path, lineno, format!("locale-insensitive date formatting: {snippet}")));
            }
            if let Some(snippet) = number_finding(line, file.language) {
                out.push(violation(codes::LOCALE_INSENSITIVE_NUMBER, &file.path, lineno, format!("locale-insensitive number formatting: {snippet}")));
            }
        }
    }

    if rtl_required {
        let mut evidence = false;
        for file in index.by_class(FileClass::Source) {
            let Some(content) = read_bounded(root, &file.path) else { continue };
            if has_rtl_evidence(&content) {
                evidence = true;
                break;
            }
        }
        if !evidence {
            out.push(Violation {
                code: codes::RTL_BIDI.to_string(),
                severity: Severity::Warning,
                message: "no direction-aware implementation evidence found; RTL locale required".to_string(),
                file: None,
                line: None,
                locale: None,
                confidence: Confidence::Unknown,
                details: Vec::new(),
            });
        }
    }

    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.code.cmp(&b.code))
    });
    out
}

fn violation(code: &str, path: &Path, line: usize, message: String) -> Violation {
    Violation {
        code: code.to_string(),
        severity: Severity::Warning,
        message,
        file: Some(path.to_path_buf()),
        line: Some(line),
        locale: None,
        confidence: Confidence::Likely,
        details: Vec::new(),
    }
}

/// The matched call snippet starting at byte `p`, closed at the next `)` on
/// the same line or truncated at 60 bytes.
fn call_snippet(line: &str, p: usize) -> String {
    let bytes = line.as_bytes();
    let mut end = p;
    while end < bytes.len() && bytes[end] != b')' && end - p < 60 {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b')' {
        end += 1;
    }
    line[p..end].trim().to_string()
}

fn date_finding(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    // strftime("...") with a literal format argument.
    if let Some(p) = find_token(bytes, b"strftime(", 0) {
        if literal_after(line, p + b"strftime(".len()).is_some() {
            return Some(call_snippet(line, p));
        }
    }
    // Python datetime / Rust chrono naive `.format("...")` with a strftime
    // date directive (Java `String.format` is excluded: its literal carries
    // printf conversions, not date directives, and it is covered by the
    // number rule).
    if let Some(p) = find_token(bytes, b".format(", 0) {
        if let Some((lit, _, _)) = literal_after(line, p + b".format(".len()) {
            if lit.starts_with('%') && contains_strftime_directive(&lit) {
                return Some(call_snippet(line, p));
            }
        }
    }
    for token in [".toDateString()", ".toUTCString()", ".toTimeString()"] {
        if line.contains(token) {
            return Some(token.to_string());
        }
    }
    // Java SimpleDateFormat without a Locale on the same line.
    if line.contains("new SimpleDateFormat(") && !line.contains("Locale") {
        let p = line.find("new SimpleDateFormat(").unwrap();
        return Some(call_snippet(line, p));
    }
    None
}

fn number_finding(line: &str, lang: ProgLanguage) -> Option<String> {
    for token in [".toFixed(", "toPrecision("] {
        if line.contains(token) {
            let p = line.find(token).unwrap();
            return Some(call_snippet(line, p));
        }
    }
    if line.contains("String.format(") && !line.contains("Locale") {
        let p = line.find("String.format(").unwrap();
        return Some(call_snippet(line, p));
    }
    if matches!(lang, ProgLanguage::Python | ProgLanguage::JavaScript | ProgLanguage::TypeScript) {
        for (lit, _) in all_literals(line) {
            if contains_printf_spec(&lit) {
                return Some(format!("\"{lit}\" %"));
            }
        }
    }
    if line.contains("format!(\"{:.") {
        let p = line.find("format!(").unwrap();
        return Some(call_snippet(line, p));
    }
    None
}

/// True when `lit` contains a printf-style conversion specification with an
/// explicit width or precision (`%.2f`, `%05d`, ...). Bare `%d`/`%s` are not
/// matched so that strftime date formats (`%Y-%m-%d`) are not misclassified.
fn contains_printf_spec(lit: &str) -> bool {
    let b = lit.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let mut j = i + 1;
            let width_start = j;
            while j < b.len() && (b[j].is_ascii_digit() || matches!(b[j], b'.' | b'-' | b'+' | b'#' | b' ' | b'*')) {
                j += 1;
            }
            if j > width_start
                && j < b.len()
                && matches!(b[j], b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' | b'c' | b's' | b'p' | b'n')
            {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// True when `lit` contains a strftime date/time directive (`%Y`, `%m`, ...).
fn contains_strftime_directive(lit: &str) -> bool {
    let b = lit.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'%' {
            if matches!(
                b[i + 1],
                b'a' | b'A' | b'b' | b'B' | b'c' | b'C' | b'd' | b'D' | b'e' | b'F' | b'h' | b'H'
                    | b'I' | b'j' | b'm' | b'M' | b'p' | b'r' | b'R' | b'S' | b'T' | b'u' | b'U'
                    | b'V' | b'w' | b'W' | b'x' | b'X' | b'y' | b'Y' | b'z' | b'Z'
            ) {
                return true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

const RTL_TOKENS: &[&str] = &[
    "dir=",
    "direction:",
    "textDirection",
    "Directionality",
    "writing-mode",
    ".rtl",
    "isRtl",
];

fn has_rtl_evidence(content: &str) -> bool {
    for token in RTL_TOKENS {
        if content.contains(token) {
            return true;
        }
    }
    // Case-sensitive `rtl` as a standalone token.
    let b = content.as_bytes();
    let mut i = 0;
    while i + 3 <= b.len() {
        if &b[i..i + 3] == b"rtl"
            && (i == 0 || !is_ident_byte(b[i - 1]))
            && (i + 3 >= b.len() || !is_ident_byte(b[i + 3]))
        {
            return true;
        }
        i += 1;
    }
    false
}

// ---------------------------------------------------------------------------
// Locale configuration detection
// ---------------------------------------------------------------------------

/// Find default/fallback locale declarations in configuration and source.
/// Sorted by (path, key); deterministic.
pub fn detect_locale_config(index: &RepositoryIndex, root: &Path) -> Vec<LocaleConfigHint> {
    let mut hints = Vec::new();

    let mut targets: Vec<(PathBuf, String)> = Vec::new();
    for m in index.manifests() {
        if let Some(content) = read_bounded(root, &m.path) {
            targets.push((m.path.clone(), content));
        }
    }
    for f in index.by_class(FileClass::Source) {
        if let Some(content) = read_bounded(root, &f.path) {
            targets.push((f.path.clone(), content));
        }
    }

    for (path, content) in &targets {
        let name = file_name(path);
        collect_key(&mut hints, path, content, "fallbackLng", false, true);
        if name.contains("next.config") || name.contains("vue.config") {
            collect_key(&mut hints, path, content, "lng", true, false);
        }
        collect_key(&mut hints, path, content, "defaultLocale", true, true);
        collect_key(&mut hints, path, content, "setDefaultLang", true, false);
        collect_key(&mut hints, path, content, "LANGUAGE_CODE", true, false);
        collect_key(&mut hints, path, content, "default_locale", true, false);
        collect_key(&mut hints, path, content, "supportedLocales", false, true);

        // Flutter Locale('fr', 'FR') / Locale("en") constructor calls.
        if let Some(f) = index.files.iter().find(|f| &f.path == path) {
            if f.class == FileClass::Source {
                let bytes = content.as_bytes();
                let mut pos = 0;
                while let Some(p) = find_token(bytes, b"Locale(", pos) {
                    if let Some((lit, _, _)) = literal_after(content, p + b"Locale(".len()) {
                        if let Some(locale) = ogma_locale::parse_locale(&lit) {
                            hints.push(LocaleConfigHint {
                                path: path.clone(),
                                key: "Locale".to_string(),
                                locale,
                                is_default: false,
                            });
                        }
                    }
                    pos = p + b"Locale(".len();
                }
            }
        }
    }

    hints.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.locale.canonical.cmp(&b.locale.canonical))
            .then_with(|| a.is_default.cmp(&b.is_default))
    });
    hints.dedup();
    hints
}

/// Collect locale hints for a `key` token. When `array` is true a `[...]`
/// value collects every parseable entry; otherwise the first quoted literal
/// on the value line is used. Only entries parseable by
/// [`ogma_locale::parse_locale`] are recorded.
fn collect_key(hints: &mut Vec<LocaleConfigHint>, path: &Path, content: &str, key: &str, is_default: bool, array: bool) {
    let bytes = content.as_bytes();
    let mut pos = 0;
    while let Some(p) = find_ident(bytes, key.as_bytes(), pos) {
        let mut q = p + key.len();
        while q < bytes.len() && bytes[q].is_ascii_whitespace() {
            q += 1;
        }
        if q < bytes.len() && matches!(bytes[q], b':' | b'=' | b'(') {
            q += 1;
        }
        let mut r = q;
        while r < bytes.len() && bytes[r].is_ascii_whitespace() && bytes[r] != b'\n' {
            r += 1;
        }
        if array && r < bytes.len() && bytes[r] == b'[' {
            // Collect every quoted literal until the closing bracket.
            let mut s = r + 1;
            while s < bytes.len() && bytes[s] != b']' {
                if bytes[s] == b'"' || bytes[s] == b'\'' {
                    if let Some((lit, _, end)) = scan_literal(content, s) {
                        if let Some(locale) = ogma_locale::parse_locale(&lit) {
                            hints.push(LocaleConfigHint {
                                path: path.to_path_buf(),
                                key: key.to_string(),
                                locale,
                                is_default,
                            });
                        }
                        s = end;
                        continue;
                    }
                }
                s += 1;
            }
        } else {
            // First quoted literal before end of line, within 64 bytes.
            let limit = (q + 64).min(bytes.len());
            let mut s = q;
            while s < limit {
                let b = bytes[s];
                if b == b'\n' {
                    break;
                }
                if b == b'"' || b == b'\'' {
                    if let Some((lit, _, _)) = scan_literal(content, s) {
                        if let Some(locale) = ogma_locale::parse_locale(&lit) {
                            hints.push(LocaleConfigHint {
                                path: path.to_path_buf(),
                                key: key.to_string(),
                                locale,
                                is_default,
                            });
                        }
                    }
                    break;
                }
                s += 1;
            }
        }
        pos = p + key.len();
    }
}

// ---------------------------------------------------------------------------
// Text language guessing
// ---------------------------------------------------------------------------

const SCRIPT_LANGS: [(&str, u32, u32); 10] = [
    ("ru", 0x0400, 0x04FF), // Cyrillic
    ("el", 0x0370, 0x03FF), // Greek
    ("ar", 0x0600, 0x06FF), // Arabic
    ("he", 0x0590, 0x05FF), // Hebrew
    ("ko", 0xAC00, 0xD7AF), // Hangul syllables
    ("ja", 0x3040, 0x309F), // Hiragana (Katakana handled alongside)
    ("zh", 0x4E00, 0x9FFF), // Han
    ("th", 0x0E00, 0x0E7F), // Thai
    ("hy", 0x0530, 0x058F), // Armenian
    ("ka", 0x10A0, 0x10FF), // Georgian
];

/// Deterministic script-based guess of the human language of a text snippet.
/// Cyrillic→ru, Greek→el, Arabic→ar, Hebrew→he, Hangul→ko, Hiragana/Katakana→ja,
/// Han→zh, Thai→th, Armenian→hy, Georgian→ka. Returns None for Latin/ASCII text
/// (indistinguishable between en/fr/de/es/... — never guess).
pub fn guess_text_language(text: &str) -> Option<Language> {
    let mut counts = [0usize; 10];
    for c in text.chars() {
        let cp = c as u32;
        for (idx, (_, lo, hi)) in SCRIPT_LANGS.iter().enumerate() {
            if (*lo..=*hi).contains(&cp) {
                counts[idx] += 1;
                break;
            }
        }
        // Katakana sits next to Hiragana in the table slot for ja.
        if (0x30A0..=0x30FF).contains(&cp) || (0x1100..=0x11FF).contains(&cp) {
            // Hangul Jamo belongs to ko; Katakana to ja. Undo any miscount.
            if (0x1100..=0x11FF).contains(&cp) {
                counts[4] += 1;
            } else {
                counts[5] += 1;
            }
        }
    }
    // Hiragana/Katakana presence forces Japanese even with Han characters.
    if counts[5] > 0 {
        return Some(Language::new("ja"));
    }
    let total: usize = counts.iter().sum();
    if total == 0 {
        return None;
    }
    let max = *counts.iter().max().unwrap();
    let mut winner: Option<usize> = None;
    for (idx, &c) in counts.iter().enumerate() {
        if c == max {
            if winner.is_some() {
                return None; // tie: never guess
            }
            winner = Some(idx);
        }
    }
    winner.map(|i| Language::new(SCRIPT_LANGS[i].0))
}

// ---------------------------------------------------------------------------
// Lexical helpers
// ---------------------------------------------------------------------------

fn read_bounded(root: &Path, rel: &Path) -> Option<String> {
    read_file_limited(&root.join(rel), MAX_READ_BYTES).ok()
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Find the next occurrence of `token` in `bytes` (plain substring search).
fn find_token(bytes: &[u8], token: &[u8], start: usize) -> Option<usize> {
    if token.is_empty() || start >= bytes.len() {
        return None;
    }
    let mut i = start;
    while i + token.len() <= bytes.len() {
        if &bytes[i..i + token.len()] == token {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find the next occurrence of an identifier-like `token` with non-identifier
/// bytes on both sides.
fn find_ident(bytes: &[u8], token: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;
    while let Some(p) = find_token(bytes, token, pos) {
        let prev_ok = p == 0 || !is_ident_byte(bytes[p - 1]);
        let next = p + token.len();
        let next_ok = next >= bytes.len() || !is_ident_byte(bytes[next]);
        if prev_ok && next_ok {
            return Some(p);
        }
        pos = p + 1;
    }
    None
}

/// Scan a string literal whose opening quote is at byte `open`. Returns the
/// unescaped content plus the content span `(content_start, content_end)`.
fn scan_literal(line: &str, open: usize) -> Option<(String, usize, usize)> {
    let bytes = line.as_bytes();
    if open >= bytes.len() {
        return None;
    }
    let quote = bytes[open];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut i = open + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            if i + 1 >= bytes.len() {
                return None;
            }
            match bytes[i + 1] {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'r' => out.push('\r'),
                b'0' => out.push('\0'),
                other => out.push(other as char),
            }
            i += 2;
            continue;
        }
        if b == quote {
            return Some((out, open + 1, i));
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
        if out.len() > 4096 {
            return None; // unbounded-literal guard
        }
    }
    None
}

/// Parse the string literal following `pos`, skipping whitespace and one
/// optional `:` / `=` separator.
fn literal_after(line: &str, mut pos: usize) -> Option<(String, usize, usize)> {
    let bytes = line.as_bytes();
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos < bytes.len() && matches!(bytes[pos], b':' | b'=') {
        pos += 1;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
    }
    scan_literal(line, pos)
}

/// All string literals on a line, as (content, index just past the literal).
fn all_literals(line: &str) -> Vec<(String, usize)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            if let Some((text, _, end)) = scan_literal(line, i) {
                out.push((text, end + 1));
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// True when the text after a string literal starts a `| translate` pipe.
fn after_is_translate_pipe(rest: &str) -> bool {
    let t = rest.trim_start();
    let Some(after_bar) = t.strip_prefix('|') else { return false };
    let after_bar = after_bar.trim_start();
    after_bar.starts_with("translate") && {
        let tail = &after_bar["translate".len()..];
        tail.is_empty() || tail.starts_with(' ') || tail.starts_with(':') || tail.starts_with('<')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogma_model::{Confidence, StringKind};
    use std::fs;

    fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ogmap-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, content) in files {
            let p = dir.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
        }
        dir
    }

    fn scan_fixture(name: &str, files: &[(&str, &str)]) -> (PathBuf, RepositoryIndex) {
        let root = fixture(name, files);
        let index = ogma_discovery::scan(&root).unwrap();
        (root, index)
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    fn strings(root: &Path, index: &RepositoryIndex) -> Vec<StringOccurrence> {
        extract_strings(index, root)
    }

    // -- translated rules ---------------------------------------------------

    #[test]
    fn t_call_js_t() {
        let (root, index) = scan_fixture("tcall-js", &[("src/app.jsx", "const x = t(\"hello_key\");\n")]);
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert!(occ[0].translated);
        assert_eq!(occ[0].rule, "i18n/t-call");
        assert_eq!(occ[0].text, "hello_key");
        assert_eq!(occ[0].confidence, Confidence::Proven);
        assert_eq!(occ[0].kind, StringKind::UserFacing);
        cleanup(&root);
    }

    #[test]
    fn t_call_i18n_dot_t_and_this_hooks() {
        let (root, index) = scan_fixture(
            "tcall-hooks",
            &[("src/app.ts", "a = i18n.t(\"k1\"); b = this.t(\"k2\"); c = this.$t(\"k3\"); d = I18n.t(\"k4\");\n")],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["k1", "k2", "k3", "k4"]);
        assert!(occ.iter().all(|o| o.rule == "i18n/t-call" && o.translated));
        cleanup(&root);
    }

    #[test]
    fn t_call_vue_dollar_t() {
        let (root, index) = scan_fixture("tcall-vue", &[("src/App.vue", "<template>{{ $t('menu.file') }}</template>\n")]);
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].text, "menu.file");
        cleanup(&root);
    }

    #[test]
    fn t_call_gettext_and_underscore() {
        let (root, index) = scan_fixture(
            "tcall-py",
            &[("src/main.py", "a = gettext('hello')\nb = _('world')\nc = ngettext('one', 'many', n)\n")],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["hello", "world", "one"]);
        cleanup(&root);
    }

    #[test]
    fn t_call_php_laravel() {
        let (root, index) = scan_fixture("tcall-php", &[("src/welcome.php", "<?php $x = __(\"greeting.morning\"); $y = gettext(\"other.msg\");\n")]);
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["greeting.morning", "other.msg"]);
        cleanup(&root);
    }

    #[test]
    fn t_call_rust_tr_macro() {
        let (root, index) = scan_fixture("tcall-rs", &[("src/main.rs", "fn f() { let a = tr!(\"file.open\"); let b = t!(\"edit.save\"); }\n")]);
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["file.open", "edit.save"]);
        cleanup(&root);
    }

    #[test]
    fn t_call_intl_format_message() {
        let (root, index) = scan_fixture(
            "tcall-intl",
            &[("src/app.ts", "const m = intl.formatMessage({ id: \"welcome.title\" });\n")],
        );
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].text, "welcome.title");
        cleanup(&root);
    }

    #[test]
    fn t_call_non_literal_arg_skipped() {
        let (root, index) = scan_fixture("tcall-var", &[("src/app.ts", "const x = t(dynamicKey);\n")]);
        assert!(strings(&root, &index).is_empty());
        cleanup(&root);
    }

    #[test]
    fn trans_component_jsx_and_angular_pipe() {
        let (root, index) = scan_fixture(
            "trans-comp",
            &[
                ("src/App.tsx", "const a = <Trans i18nKey=\"welcome.body\" />;\n"),
                ("src/page.html", "<p>{{ 'login.button' | translate }}</p>\n"),
            ],
        );
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 2);
        assert!(occ.iter().all(|o| o.rule == "i18n/trans-component" && o.translated));
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert!(texts.contains(&"welcome.body"));
        assert!(texts.contains(&"login.button"));
        cleanup(&root);
    }

    #[test]
    fn django_template_trans() {
        let (root, index) = scan_fixture(
            "django-tpl",
            &[("templates/page.html", "{% trans \"Hello user\" %}\n{% translate \"Goodbye\" %}\n")],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello user", "Goodbye"]);
        assert!(occ.iter().all(|o| o.rule == "i18n/django-template"));
        cleanup(&root);
    }

    #[test]
    fn swift_nslocalized_string() {
        let (root, index) = scan_fixture(
            "swift-nsloc",
            &[("src/View.swift", "let s = NSLocalizedString(\"welcome.message\", comment: \"\")\n")],
        );
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].rule, "i18n/swift-nslocalized");
        assert_eq!(occ[0].text, "welcome.message");
        cleanup(&root);
    }

    // -- hard-coded rules ---------------------------------------------------

    #[test]
    fn set_text_rust_and_js() {
        let (root, index) = scan_fixture(
            "set-text",
            &[
                ("src/main.rs", "fn f() { btn.set_text(\"Save file\"); win.set_title(\"Options\"); }\n"),
                ("src/app.js", "el.setText(\"Cancel now\"); field.setPlaceholder(\"Type here\");\n"),
            ],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["Cancel now", "Type here", "Save file", "Options"]);
        assert!(occ.iter().all(|o| o.rule == "ui/set-text" && !o.translated && o.confidence == Confidence::Likely));
        cleanup(&root);
    }

    #[test]
    fn swiftui_text_and_button() {
        let (root, index) = scan_fixture(
            "swiftui",
            &[("src/ContentView.swift", "var body: some View { Text(\"Hello world\") ; Button(\"Tap me now\") { } }\n")],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello world", "Tap me now"]);
        assert!(occ.iter().all(|o| o.rule == "ui/set-text"));
        cleanup(&root);
    }

    #[test]
    fn dialog_alert_js() {
        let (root, index) = scan_fixture("dialog", &[("src/app.ts", "alert(\"Are you sure\");\n")]);
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].rule, "ui/dialog");
        cleanup(&root);
    }

    #[test]
    fn cli_print_across_languages() {
        let (root, index) = scan_fixture(
            "cli-print",
            &[
                ("src/main.rs", "fn main() { println!(\"Hello rust\"); }\n"),
                ("src/main.go", "package main\nfunc main() { fmt.Println(\"Hello go\") }\n"),
                ("src/main.py", "print(\"Hello python\")\n"),
                ("src/run.sh", "echo \"Hello shell\"\n"),
                ("src/page.php", "<?php echo \"Hello php\"; \n"),
            ],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello go", "Hello python", "Hello rust", "Hello php", "Hello shell"]);
        assert!(occ.iter().all(|o| o.rule == "cli/print" && o.kind == StringKind::UserFacing));
        cleanup(&root);
    }

    #[test]
    fn jsx_text_and_button_text() {
        let (root, index) = scan_fixture(
            "jsx-text",
            &[("src/App.tsx", "const a = <button>Save changes now</button>;\n")],
        );
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].rule, "ui/html-text");
        assert_eq!(occ[0].text, "Save changes now");
        cleanup(&root);
    }

    #[test]
    fn html_text_title_and_option() {
        let (root, index) = scan_fixture(
            "html-text",
            &[(
                "public/index.html",
                "<html><head><title>My page title</title></head><body>\n<select><option>Choose a country</option></select>\n</body></html>\n",
            )],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert!(texts.contains(&"My page title"));
        assert!(texts.contains(&"Choose a country"));
        assert!(occ.iter().all(|o| o.rule == "ui/html-text"));
        cleanup(&root);
    }

    #[test]
    fn aria_label_attribute() {
        let (root, index) = scan_fixture(
            "aria",
            &[("public/form.html", "<input type=\"text\" aria-label=\"Search the catalog\" placeholder=\"Keywords here\">\n")],
        );
        let occ = strings(&root, &index);
        let by_rule: Vec<(&str, &str)> = occ.iter().map(|o| (o.rule.as_str(), o.text.as_str())).collect();
        assert!(by_rule.contains(&("ui/aria-label", "Search the catalog")));
        assert!(by_rule.contains(&("ui/html-text", "Keywords here")));
        cleanup(&root);
    }

    #[test]
    fn err_exception_rules() {
        let (root, index) = scan_fixture(
            "err",
            &[
                ("src/main.rs", "fn f() { panic!(\"boom today\"); anyhow!(\"context here\"); }\n"),
                ("src/App.java", "throw new IllegalStateException(\"bad state now\");\n"),
                ("src/main.py", "raise ValueError(\"wrong value here\")\n"),
            ],
        );
        let occ = strings(&root, &index);
        let texts: Vec<&str> = occ.iter().map(|o| o.text.as_str()).collect();
        assert!(texts.contains(&"boom today"));
        assert!(texts.contains(&"context here"));
        assert!(texts.contains(&"bad state now"));
        assert!(texts.contains(&"wrong value here"));
        assert!(occ.iter().all(|o| o.rule == "err/exception" && o.kind == StringKind::DeveloperFacing && o.confidence == Confidence::Proven));
        cleanup(&root);
    }

    #[test]
    fn non_linguistic_strings_skipped() {
        let (root, index) = scan_fixture(
            "nonling",
            &[(
                "src/main.rs",
                "fn f() {\n\
                 println!(\"delete_account\");\n\
                 println!(\"API_SECRET_KEY\");\n\
                 println!(\"https://example.com/x.png\");\n\
                 println!(\"a\");\n\
                 println!(\"12345\");\n\
                 println!(\"/usr/local/bin/tool.sh\");\n\
                 println!(\"deadbeefcafe1234\");\n\
                 }\n",
            )],
        );
        assert!(strings(&root, &index).is_empty());
        cleanup(&root);
    }

    #[test]
    fn import_lines_ignored() {
        let (root, index) = scan_fixture(
            "imports",
            &[("src/main.py", "from django.utils.translation import gettext as _\nimport os\nprint(\"real output\")\n")],
        );
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].text, "real output");
        cleanup(&root);
    }

    #[test]
    fn line_numbers_accurate() {
        let (root, index) = scan_fixture("lines", &[("src/main.py", "x = 1\nprint(\"first\")\ny = 2\nprint(\"second\")\n")]);
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].line, 2);
        assert_eq!(occ[1].line, 4);
        cleanup(&root);
    }

    #[test]
    fn deterministic_file_sort_order() {
        let (root, index) = scan_fixture(
            "sort",
            &[
                ("src/b.py", "print(\"from b\")\n"),
                ("src/a.py", "print(\"from a\")\n"),
                ("src/c.py", "print(\"from c\")\n"),
            ],
        );
        let occ = strings(&root, &index);
        let files: Vec<&str> = occ.iter().map(|o| o.file.to_str().unwrap()).collect();
        assert_eq!(files, vec!["src/a.py", "src/b.py", "src/c.py"]);
        cleanup(&root);
    }

    #[test]
    fn escaped_quotes_unescaped() {
        let (root, index) = scan_fixture("escape", &[("src/a.py", "print(\"she said \\\"hi\\\" there\")\n")]);
        let occ = strings(&root, &index);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].text, "she said \"hi\" there");
        cleanup(&root);
    }

    // -- semantics ----------------------------------------------------------

    #[test]
    fn date_semantics_strftime() {
        let (root, index) = scan_fixture("sem-date", &[("src/fmt.py", "s = d.strftime(\"%Y-%m-%d\")\n")]);
        let v = analyse_semantics(&index, &root, false);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].code, codes::LOCALE_INSENSITIVE_DATE);
        assert_eq!(v[0].severity, Severity::Warning);
        assert_eq!(v[0].confidence, Confidence::Likely);
        assert_eq!(v[0].line, Some(1));
        assert!(v[0].message.contains("strftime"));
        cleanup(&root);
    }

    #[test]
    fn date_semantics_js_and_java() {
        let (root, index) = scan_fixture(
            "sem-date2",
            &[
                ("src/app.js", "const s = d.toDateString();\n"),
                ("src/Fmt.java", "var f = new SimpleDateFormat(\"yyyy-MM-dd\");\n"),
                ("src/Ok.java", "var g = new SimpleDateFormat(\"yyyy-MM-dd\", Locale.US);\n"),
            ],
        );
        let v = analyse_semantics(&index, &root, false);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| x.code == codes::LOCALE_INSENSITIVE_DATE));
        cleanup(&root);
    }

    #[test]
    fn number_semantics() {
        let (root, index) = scan_fixture(
            "sem-num",
            &[
                ("src/app.js", "const s = price.toFixed(2);\n"),
                ("src/Fmt.java", "String s = String.format(\"%.2f\", x);\n"),
                ("src/Ok.java", "String t = String.format(Locale.US, \"%.2f\", x);\n"),
                ("src/fmt.py", "print(\"%.2f\" % value)\n"),
                ("src/main.rs", "let s = format!(\"{:.2}\", x);\n"),
            ],
        );
        let v = analyse_semantics(&index, &root, false);
        assert_eq!(v.len(), 4);
        assert!(v.iter().all(|x| x.code == codes::LOCALE_INSENSITIVE_NUMBER));
        assert!(v.iter().all(|x| x.file.as_ref().unwrap().to_str().unwrap() != "src/Ok.java"));
        cleanup(&root);
    }

    #[test]
    fn rtl_required_without_evidence() {
        let (root, index) = scan_fixture("rtl-none", &[("src/main.py", "print(\"hello\")\n")]);
        let v = analyse_semantics(&index, &root, true);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].code, codes::RTL_BIDI);
        assert_eq!(v[0].severity, Severity::Warning);
        assert_eq!(v[0].confidence, Confidence::Unknown);
        assert!(v[0].file.is_none() && v[0].line.is_none());
        assert_eq!(v[0].message, "no direction-aware implementation evidence found; RTL locale required");
        cleanup(&root);
    }

    #[test]
    fn rtl_required_with_evidence() {
        let (root, index) = scan_fixture("rtl-evidence", &[("src/App.tsx", "const d = <div dir=\"rtl\">x</div>;\n")]);
        assert!(analyse_semantics(&index, &root, true).is_empty());
        cleanup(&root);
    }

    #[test]
    fn rtl_not_required_no_check() {
        let (root, index) = scan_fixture("rtl-off", &[("src/main.py", "print(\"hello\")\n")]);
        assert!(analyse_semantics(&index, &root, false).is_empty());
        cleanup(&root);
    }

    // -- framework detection ------------------------------------------------

    #[test]
    fn framework_react_and_generic() {
        let (root, index) = scan_fixture(
            "fw-react",
            &[
                ("package.json", "{ \"dependencies\": { \"react\": \"18.0.0\", \"i18next\": \"23.0.0\" } }\n"),
                ("src/app.jsx", "const x = t(\"k\");\n"),
            ],
        );
        let fw = detect_frameworks(&index, &root);
        let names: Vec<&str> = fw.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["generic", "react"]);
        let react = fw.iter().find(|f| f.name == "react").unwrap();
        assert_eq!(react.confidence, Confidence::Proven);
        assert_eq!(react.i18n_hooks, vec!["t(", "i18n.t(", "<Trans", "useTranslation"]);
        assert_eq!(fw.iter().find(|f| f.name == "generic").unwrap().confidence, Confidence::Unknown);
        cleanup(&root);
    }

    #[test]
    fn framework_django_from_settings() {
        let (root, index) = scan_fixture(
            "fw-django",
            &[
                ("myapp/settings.py", "INSTALLED_APPS = [\"django.contrib.admin\"]\n"),
                ("myapp/views.py", "x = _(\"hello\")\n"),
            ],
        );
        let fw = detect_frameworks(&index, &root);
        assert!(fw.iter().any(|f| f.name == "django" && f.confidence == Confidence::Proven));
        cleanup(&root);
    }

    #[test]
    fn framework_django_from_requirements() {
        let (root, index) = scan_fixture("fw-django2", &[("requirements.txt", "django>=4.0\n"), ("src/tool.py", "x = 1\n")]);
        let fw = detect_frameworks(&index, &root);
        assert!(fw.iter().any(|f| f.name == "django"));
        cleanup(&root);
    }

    #[test]
    fn framework_laravel() {
        let (root, index) = scan_fixture(
            "fw-laravel",
            &[
                ("composer.json", "{ \"require\": { \"laravel/framework\": \"^10.0\" } }\n"),
                ("src/welcome.php", "<?php $x = 1;\n"),
            ],
        );
        let fw = detect_frameworks(&index, &root);
        let laravel = fw.iter().find(|f| f.name == "laravel").unwrap();
        assert_eq!(laravel.confidence, Confidence::Proven);
        assert_eq!(laravel.i18n_hooks, vec!["__(", "@lang", "trans("]);
        cleanup(&root);
    }

    #[test]
    fn framework_i18next_only() {
        let (root, index) = scan_fixture(
            "fw-i18next",
            &[("package.json", "{ \"dependencies\": { \"i18next\": \"23.0.0\" } }\n"), ("src/app.js", "x = 1;\n")],
        );
        let fw = detect_frameworks(&index, &root);
        assert!(fw.iter().any(|f| f.name == "i18next"));
        assert!(!fw.iter().any(|f| f.name == "react"));
        cleanup(&root);
    }

    #[test]
    fn framework_vue_source_signature_likely() {
        let (root, index) = scan_fixture(
            "fw-vue",
            &[
                ("package.json", "{ \"dependencies\": { \"vue\": \"3.0.0\" } }\n"),
                ("src/App.vue", "<template>{{ $t('menu.help') }}</template>\n"),
            ],
        );
        let fw = detect_frameworks(&index, &root);
        let vue = fw.iter().find(|f| f.name == "vue").unwrap();
        assert_eq!(vue.confidence, Confidence::Likely);
        cleanup(&root);
    }

    #[test]
    fn framework_none_without_source() {
        let (root, index) = scan_fixture("fw-empty", &[("README.md", "# hello\n")]);
        let fw = detect_frameworks(&index, &root);
        assert!(fw.is_empty());
        cleanup(&root);
    }

    // -- locale config ------------------------------------------------------

    #[test]
    fn locale_config_fallback_and_default() {
        let (root, index) = scan_fixture(
            "loc-cfg",
            &[
                ("src/config.js", "i18n.init({ fallbackLng: 'fr' });\n"),
                ("next.config.js", "module.exports = { i18n: { defaultLocale: 'en', lng: 'de' } };\n"),
            ],
        );
        let hints = detect_locale_config(&index, &root);
        let got: Vec<(String, String, bool)> = hints
            .iter()
            .map(|h| (h.key.clone(), h.locale.canonical.clone(), h.is_default))
            .collect();
        assert!(got.contains(&("fallbackLng".to_string(), "fr".to_string(), false)));
        assert!(got.contains(&("defaultLocale".to_string(), "en".to_string(), true)));
        assert!(got.contains(&("lng".to_string(), "de".to_string(), true)));
        // sorted by (path, key)
        let paths: Vec<&str> = hints.iter().map(|h| h.path.to_str().unwrap()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        cleanup(&root);
    }

    #[test]
    fn locale_config_django_and_supported() {
        let (root, index) = scan_fixture(
            "loc-cfg2",
            &[
                ("myapp/settings.py", "LANGUAGE_CODE = \"en-us\"\n"),
                ("vue.config.js", "module.exports = { supportedLocales: ['en', 'fr', 'de'] }\n"),
            ],
        );
        let hints = detect_locale_config(&index, &root);
        let got: Vec<(String, String, bool)> = hints
            .iter()
            .map(|h| (h.key.clone(), h.locale.canonical.clone(), h.is_default))
            .collect();
        assert!(got.contains(&("LANGUAGE_CODE".to_string(), "en-US".to_string(), true)));
        assert!(got.contains(&("supportedLocales".to_string(), "en".to_string(), false)));
        assert!(got.contains(&("supportedLocales".to_string(), "fr".to_string(), false)));
        assert!(got.contains(&("supportedLocales".to_string(), "de".to_string(), false)));
        cleanup(&root);
    }

    #[test]
    fn locale_config_flutter_locales() {
        let (root, index) = scan_fixture(
            "loc-flutter",
            &[(
                "lib/main.dart",
                "const supportedLocales = [Locale('fr', 'FR'), Locale(\"en\")];\n",
            )],
        );
        let hints = detect_locale_config(&index, &root);
        let got: Vec<(String, String)> = hints.iter().map(|h| (h.key.clone(), h.locale.canonical.clone())).collect();
        assert!(got.contains(&("Locale".to_string(), "fr".to_string())));
        assert!(got.contains(&("Locale".to_string(), "en".to_string())));
        assert!(!hints.iter().any(|h| h.is_default));
        cleanup(&root);
    }

    #[test]
    fn locale_config_ignores_unparseable() {
        let (root, index) = scan_fixture(
            "loc-bad",
            &[("src/config.js", "i18n.init({ fallbackLng: 'xx-not-real', defaultLocale: 'en' });\n")],
        );
        let hints = detect_locale_config(&index, &root);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].locale.canonical, "en");
        cleanup(&root);
    }

    // -- guess_text_language -------------------------------------------------

    #[test]
    fn guess_scripts() {
        assert_eq!(guess_text_language("Привет мир"), Some(Language::new("ru")));
        assert_eq!(guess_text_language("Γειά σου κόσμε"), Some(Language::new("el")));
        assert_eq!(guess_text_language("مرحبا بالعالم"), Some(Language::new("ar")));
        assert_eq!(guess_text_language("שלום עולם"), Some(Language::new("he")));
        assert_eq!(guess_text_language("안녕하세요"), Some(Language::new("ko")));
        assert_eq!(guess_text_language("こんにちは世界"), Some(Language::new("ja")));
        assert_eq!(guess_text_language("你好世界"), Some(Language::new("zh")));
        assert_eq!(guess_text_language("สวัสดีชาวโลก"), Some(Language::new("th")));
        assert_eq!(guess_text_language("Բարև աշխարհ"), Some(Language::new("hy")));
        assert_eq!(guess_text_language("გამარჯობა"), Some(Language::new("ka")));
    }

    #[test]
    fn guess_kana_forces_ja_over_han() {
        assert_eq!(guess_text_language("漢字かな"), Some(Language::new("ja")));
    }

    #[test]
    fn guess_latin_and_ties_are_none() {
        assert_eq!(guess_text_language("Hello world"), None);
        assert_eq!(guess_text_language(""), None);
        // equal counts of two scripts: never guess
        assert_eq!(guess_text_language("абαβ"), None);
    }
}
