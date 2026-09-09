//! Hand-rolled per-format catalogue parsers. All parsers are tolerant:
//! structurally broken content yields no units rather than an error, and
//! discovery skips files whose required structure is absent entirely.

use ogma_model::{CatalogueFormat, LocaleId, TranslationUnit};

use crate::placeholders::extract_placeholders;

/// The CLDR plural categories recognised as plural-arm suffixes.
pub(crate) const PLURAL_CATEGORIES: &[&str] = &["zero", "one", "two", "few", "many", "other"];

/// A parsed catalogue before locale inference and unit finalisation.
pub(crate) struct ParsedCatalogue {
    pub embedded_locale: Option<LocaleId>,
    pub units: Vec<TranslationUnit>,
}

/// Parse `content` as `format`. Returns `None` when the content does not
/// satisfy the format's minimum structure (discovered file is skipped).
pub(crate) fn parse_catalogue(format: CatalogueFormat, content: &str) -> Option<ParsedCatalogue> {
    let mut parsed = match format {
        CatalogueFormat::Json => parse_json(content, false),
        CatalogueFormat::Arb => parse_json(content, true),
        CatalogueFormat::Toml => parse_toml(content),
        CatalogueFormat::Yaml => parse_yaml(content),
        CatalogueFormat::Po | CatalogueFormat::Pot => parse_po(content),
        CatalogueFormat::Properties => parse_properties(content),
        CatalogueFormat::Csv => parse_csv(content),
        CatalogueFormat::Xliff => parse_xliff(content),
        CatalogueFormat::Resx => parse_resx(content),
        CatalogueFormat::AppleStrings => parse_strings(content),
        CatalogueFormat::AppleStringsdict => parse_stringsdict(content),
        CatalogueFormat::Custom(_) => None,
    }?;
    for unit in &mut parsed.units {
        unit.placeholders = extract_placeholders(unit.value.as_deref().unwrap_or(""));
    }
    apply_plural_suffixes(&mut parsed.units);
    Some(parsed)
}

/// Rewrite key-suffixed plural arms (`items_one`, `items@one`) into
/// `plural_form` + stripped base key. Applies to units with no plural form
/// yet, and to units whose declared form already matches the suffix (CSV
/// `key,plural_form` exports). PO arm units keep their index form.
pub(crate) fn apply_plural_suffixes(units: &mut [TranslationUnit]) {
    for unit in units.iter_mut() {
        if let Some(pos) = unit.key.rfind(['_', '@']) {
            let (base, suffix) = (&unit.key[..pos], &unit.key[pos + 1..]);
            if PLURAL_CATEGORIES.contains(&suffix) && !base.is_empty() {
                let matches = match &unit.plural_form {
                    None => true,
                    Some(existing) => existing == suffix,
                };
                if matches {
                    unit.plural_form = Some(suffix.to_string());
                    unit.key = base.to_string();
                }
            }
        }
    }
}

fn unit(key: &str, value: Option<String>, plural_form: Option<String>) -> TranslationUnit {
    TranslationUnit {
        key: key.to_string(),
        value,
        locale: None,
        plural_form,
        placeholders: Vec::new(),
    }
}

/// `Some` for non-empty values, `None` otherwise (empty and whitespace-only
/// values are treated as untranslated).
fn some_value(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

// ---------------------------------------------------------------------------
// JSON / ARB
// ---------------------------------------------------------------------------

fn parse_json(content: &str, arb: bool) -> Option<ParsedCatalogue> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    if !value.is_object() {
        return None;
    }
    let mut embedded_locale = None;
    if arb {
        if let Some(loc) = value.get("@@locale").and_then(|v| v.as_str()) {
            embedded_locale = ogma_locale::parse_locale(loc);
        }
    }
    let mut units = Vec::new();
    flatten_json(&value, "", arb, &mut units);
    Some(ParsedCatalogue { embedded_locale, units })
}

fn flatten_json(value: &serde_json::Value, prefix: &str, arb: bool, units: &mut Vec<TranslationUnit>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if arb && key.starts_with('@') {
                    continue;
                }
                let full = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_json(child, &full, arb, units);
            }
        }
        serde_json::Value::String(s) => {
            // Numbers, booleans, nulls and arrays are ignored by design.
            units.push(unit(prefix, some_value(s), None));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// TOML
// ---------------------------------------------------------------------------

fn parse_toml(content: &str) -> Option<ParsedCatalogue> {
    let value: toml::Value = content.parse().ok()?;
    if !value.is_table() {
        return None;
    }
    let mut units = Vec::new();
    flatten_toml(&value, "", &mut units);
    Some(ParsedCatalogue { embedded_locale: None, units })
}

fn flatten_toml(value: &toml::Value, prefix: &str, units: &mut Vec<TranslationUnit>) {
    match value {
        toml::Value::Table(table) => {
            for (key, child) in table {
                let full = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_toml(child, &full, units);
            }
        }
        toml::Value::String(s) => units.push(unit(prefix, some_value(s), None)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// YAML (minimal subset)
// ---------------------------------------------------------------------------

fn parse_yaml(content: &str) -> Option<ParsedCatalogue> {
    let mut units = Vec::new();
    // (indent, key) stack of open parent keys.
    let mut stack: Vec<(usize, String)> = Vec::new();

    for raw in content.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some((key, raw_value)) = trimmed.split_once(':') else {
            continue; // not a key:value line; subset parser tolerates
        };
        let key = key.trim().trim_matches(['\'', '"']);
        if key.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(d, _)| *d >= indent) {
            stack.pop();
        }
        let value = raw_value.trim();
        let full = {
            let mut parts: Vec<&str> = stack.iter().map(|(_, k)| k.as_str()).collect();
            parts.push(key);
            parts.join(".")
        };
        if value.is_empty() {
            stack.push((indent, key.to_string()));
        } else {
            units.push(unit(&full, some_value(&unquote_yaml(value)), None));
        }
    }

    Some(ParsedCatalogue { embedded_locale: None, units })
}

fn unquote_yaml(value: &str) -> String {
    // Strip a trailing unquoted comment first ("value  # note" or "'v'  # note").
    let value = match value.find(" #") {
        Some(idx) => value[..idx].trim_end(),
        None => value,
    };
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// PO / POT
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PoEntry {
    msgctxt: Option<String>,
    msgid: Option<String>,
    msgid_plural: Option<String>,
    msgstr: Option<String>,
    arms: std::collections::BTreeMap<usize, String>,
}

#[derive(Clone, Copy, PartialEq)]
enum PoField {
    Ctxt,
    Id,
    IdPlural,
    Str,
    Arm(usize),
}

fn po_unquote(raw: &str) -> Option<(PoField, String)> {
    let raw = raw.trim();
    let (field, rest) = if let Some(rest) = raw.strip_prefix("msgctxt") {
        (PoField::Ctxt, rest.trim_start())
    } else if let Some(rest) = raw.strip_prefix("msgid_plural") {
        (PoField::IdPlural, rest.trim_start())
    } else if let Some(rest) = raw.strip_prefix("msgid") {
        (PoField::Id, rest.trim_start())
    } else if let Some(rest) = raw.strip_prefix("msgstr") {
        let rest = rest.trim_start();
        if let Some(open) = rest.strip_prefix('[') {
            let close = open.find(']')?;
            let index: usize = open[..close].trim().parse().ok()?;
            (PoField::Arm(index), open[close + 1..].trim_start())
        } else {
            (PoField::Str, rest)
        }
    } else {
        return None;
    };
    let text = po_unescape(field_text(rest)?);
    Some((field, text))
}

/// Extract the quoted string body from the remainder of a directive line.
fn field_text(rest: &str) -> Option<&str> {
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner)
}

fn po_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_po(content: &str) -> Option<ParsedCatalogue> {
    let mut entries: Vec<PoEntry> = Vec::new();
    let mut current = PoEntry::default();
    let mut last: Option<PoField> = None;
    let mut saw_directive = false;

    let flush = |entries: &mut Vec<PoEntry>, current: &mut PoEntry, last: &mut Option<PoField>| {
        if current.msgid.is_some() || !current.arms.is_empty() {
            entries.push(std::mem::take(current));
        }
        *last = None;
    };

    for raw in content.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut entries, &mut current, &mut last);
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some((field, text)) = po_unquote(trimmed) {
            saw_directive = true;
            match field {
                PoField::Ctxt => current.msgctxt = Some(text),
                PoField::Id => current.msgid = Some(text),
                PoField::IdPlural => current.msgid_plural = Some(text),
                PoField::Str => current.msgstr = Some(text),
                PoField::Arm(i) => {
                    current.arms.insert(i, text);
                }
            }
            last = Some(field);
        } else if trimmed.starts_with('"') {
            // Continuation of the previous string.
            if let (Some(field), Some(inner)) = (last, field_text(trimmed)) {
                let text = po_unescape(inner);
                match field {
                    PoField::Ctxt => current.msgctxt.get_or_insert_with(String::new).push_str(&text),
                    PoField::Id => current.msgid.get_or_insert_with(String::new).push_str(&text),
                    PoField::IdPlural => {
                        current.msgid_plural.get_or_insert_with(String::new).push_str(&text)
                    }
                    PoField::Str => current.msgstr.get_or_insert_with(String::new).push_str(&text),
                    PoField::Arm(i) => current.arms.entry(i).or_default().push_str(&text),
                }
            }
        }
    }
    flush(&mut entries, &mut current, &mut last);

    if !saw_directive && entries.is_empty() {
        return None;
    }

    // Header: first entry with empty msgid. Read nplurals from its Plural-Forms.
    let header = entries.iter().find(|e| e.msgid.as_deref() == Some(""));
    let nplurals = header.and_then(|h| {
        let text = h.msgstr.as_deref().unwrap_or("");
        let pf = text.split("Plural-Forms:").nth(1)?;
        let pf = pf.split(';').next()?;
        let num = pf.split("nplurals=").nth(1)?;
        let num = num.split(|c: char| !c.is_ascii_digit()).next()?;
        num.parse::<usize>().ok()
    });

    let mut units = Vec::new();
    for entry in &entries {
        let Some(msgid) = entry.msgid.as_deref() else { continue };
        if msgid.is_empty() {
            continue; // header entry
        }
        let key = match entry.msgctxt.as_deref() {
            Some(ctx) if !ctx.is_empty() => format!("{ctx}|{msgid}"),
            _ => msgid.to_string(),
        };
        if entry.msgid_plural.is_some() {
            let prefix = if nplurals.is_some() { "arm" } else { "rawarm" };
            match nplurals {
                Some(k) => {
                    // Pad to the declared nplurals so the declared plurality
                    // is recoverable from the units alone.
                    for i in 0..k {
                        let value = entry.arms.get(&i).and_then(|v| some_value(v));
                        units.push(unit(&key, value, Some(format!("{prefix}{i}"))));
                    }
                }
                None => {
                    if entry.arms.is_empty() {
                        units.push(unit(&key, None, Some(format!("{prefix}0"))));
                    } else {
                        for (i, arm) in &entry.arms {
                            units.push(unit(&key, some_value(arm), Some(format!("{prefix}{i}"))));
                        }
                    }
                }
            }
        } else {
            units.push(unit(&key, entry.msgstr.as_deref().and_then(some_value), None));
        }
    }

    Some(ParsedCatalogue { embedded_locale: None, units })
}

// ---------------------------------------------------------------------------
// properties
// ---------------------------------------------------------------------------

fn parse_properties(content: &str) -> Option<ParsedCatalogue> {
    let mut embedded_locale = None;
    let mut units = Vec::new();

    for (idx, raw) in content.lines().enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if idx == 0 {
            let first = line.trim_start_matches('\u{feff}').trim();
            if let Some(value) = first.strip_prefix("locale=") {
                embedded_locale = ogma_locale::parse_locale(value.trim());
                continue;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        let split_at = trimmed
            .find(|c| c == '=' || c == ':')
            .unwrap_or(trimmed.len());
        let key = trimmed[..split_at].trim();
        let value = trimmed[split_at..].trim_start_matches(['=', ':']).trim();
        if key.is_empty() {
            continue;
        }
        units.push(unit(key, some_value(value), None));
    }

    Some(ParsedCatalogue { embedded_locale, units })
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

fn parse_csv(content: &str) -> Option<ParsedCatalogue> {
    let mut units = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        if idx == 0 && cells.first().is_some_and(|c| c.eq_ignore_ascii_case("key")) {
            continue; // header row
        }
        let Some(&key) = cells.first() else { continue };
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        let value = cells.get(1).copied().unwrap_or("");
        let plural = cells
            .get(2)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        units.push(unit(key, some_value(value), plural));
    }
    Some(ParsedCatalogue { embedded_locale: None, units })
}

// ---------------------------------------------------------------------------
// XLIFF
// ---------------------------------------------------------------------------

fn parse_xliff(content: &str) -> Option<ParsedCatalogue> {
    let mut embedded_locale = None;
    let mut units = Vec::new();
    let mut block = String::new();
    let mut in_block = false;

    for raw in content.lines() {
        if embedded_locale.is_none() {
            if let Some(loc) = xml_attr(raw, "target-language") {
                embedded_locale = ogma_locale::parse_locale(&loc);
            } else if let Some(loc) = xml_attr(raw, "source-language") {
                embedded_locale = ogma_locale::parse_locale(&loc);
            }
        }

        if !in_block {
            if raw.contains("<trans-unit") {
                in_block = true;
                block.clear();
                block.push_str(raw);
                block.push('\n');
                if raw.contains("</trans-unit>") {
                    if let Some(u) = finish_trans_unit(&block) {
                        units.push(u);
                    }
                    in_block = false;
                }
            }
        } else {
            block.push_str(raw);
            block.push('\n');
            if raw.contains("</trans-unit>") {
                if let Some(u) = finish_trans_unit(&block) {
                    units.push(u);
                }
                in_block = false;
            }
        }
    }

    Some(ParsedCatalogue { embedded_locale, units })
}

fn xml_attr(line: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn xml_tag_body(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(xml_unescape(&block[start..end]))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn finish_trans_unit(block: &str) -> Option<TranslationUnit> {
    let id = xml_attr(block, "id")?;
    let source = xml_tag_body(block, "source");
    let target = xml_tag_body(block, "target");
    let value = target
        .filter(|v| !v.trim().is_empty())
        .or(source.filter(|v| !v.trim().is_empty()));
    Some(unit(&id, value, None))
}

// ---------------------------------------------------------------------------
// RESX
// ---------------------------------------------------------------------------

fn parse_resx(content: &str) -> Option<ParsedCatalogue> {
    let mut units = Vec::new();
    let mut name: Option<String> = None;
    let mut block = String::new();

    for raw in content.lines() {
        if name.is_none() {
            if raw.contains("<data name=\"") {
                name = xml_attr(raw, "name");
                block.clear();
                block.push_str(raw);
                block.push('\n');
                if raw.contains("</data>") {
                    push_resx(&mut units, name.take().unwrap_or_default(), &block);
                }
            }
        } else {
            block.push_str(raw);
            block.push('\n');
            if raw.contains("</data>") {
                push_resx(&mut units, name.take().unwrap_or_default(), &block);
            }
        }
    }

    Some(ParsedCatalogue { embedded_locale: None, units })
}

fn push_resx(units: &mut Vec<TranslationUnit>, name: String, block: &str) {
    if name.is_empty() {
        return;
    }
    let value = xml_tag_body(block, "value");
    units.push(unit(&name, value, None));
}

// ---------------------------------------------------------------------------
// Apple .strings
// ---------------------------------------------------------------------------

fn parse_strings(content: &str) -> Option<ParsedCatalogue> {
    // Strip /* */ comments (they may span lines).
    let mut cleaned = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for d in chars.by_ref() {
                if prev == '*' && d == '/' {
                    break;
                }
                prev = d;
            }
            cleaned.push(' ');
        } else {
            cleaned.push(c);
        }
    }

    let mut units = Vec::new();
    let bytes = cleaned.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let (key, next) = scan_quoted(&cleaned, i)?;
            let mut j = next;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'=') {
                j += 1;
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'"') {
                    let (value, after) = scan_quoted(&cleaned, j)?;
                    units.push(unit(&key, some_value(&value), None));
                    i = after;
                    continue;
                }
            }
        }
        i += 1;
    }

    Some(ParsedCatalogue { embedded_locale: None, units })
}

/// Read a quoted string starting at `start` (which points at the opening
/// quote). Returns (unescaped content, index just past the closing quote).
fn scan_quoted(s: &str, start: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                match bytes[i + 1] {
                    b'"' => out.push('"'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'\\' => out.push('\\'),
                    other => {
                        out.push('\\');
                        out.push(other as char);
                    }
                }
                i += 2;
            }
            b'"' => return Some((out, i + 1)),
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Apple .stringsdict (lite)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum SdEventKind {
    Key,
    String,
    DictOpen,
    DictClose,
}

fn parse_stringsdict(content: &str) -> Option<ParsedCatalogue> {
    // Tokenise into (kind, text, depth) events.
    let mut events: Vec<(SdEventKind, String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut rest = content;
    loop {
        let candidates = ["<key>", "<string>", "<dict>", "</dict>"];
        let next = candidates
            .iter()
            .filter_map(|t| rest.find(t).map(|i| (i, *t)))
            .min_by_key(|(i, _)| *i);
        let Some((idx, tag)) = next else { break };
        let text_start = idx + tag.len();
        if tag == "<dict>" {
            depth += 1;
            events.push((SdEventKind::DictOpen, String::new(), depth));
            rest = &rest[text_start..];
        } else if tag == "</dict>" {
            events.push((SdEventKind::DictClose, String::new(), depth));
            depth = depth.saturating_sub(1);
            rest = &rest[text_start..];
        } else {
            let close = if tag == "<key>" { "</key>" } else { "</string>" };
            let end = rest[text_start..].find(close)? + text_start;
            let text = xml_unescape(&rest[text_start..end]);
            let kind = if tag == "<key>" { SdEventKind::Key } else { SdEventKind::String };
            events.push((kind, text, depth));
            rest = &rest[end + close.len()..];
        }
    }

    // Top-level keys live at dict depth 1; their entry body is depth 2;
    // plural category keys inside variable dicts are at depth >= 2.
    let mut units = Vec::new();
    let mut i = 0usize;
    while i < events.len() {
        let (kind, text, d) = &events[i];
        if *kind == SdEventKind::Key && *d == 1 {
            let message_key = text.clone();
            // Collect (category, value) pairs within the entry body.
            let mut arms: Vec<(String, String)> = Vec::new();
            let mut fallback: Option<String> = None;
            let mut j = i + 1;
            while j < events.len() {
                let (k2, t2, d2) = &events[j];
                if *k2 == SdEventKind::Key && *d2 == 1 {
                    break; // next top-level key
                }
                if *k2 == SdEventKind::Key
                    && *d2 >= 2
                    && PLURAL_CATEGORIES.contains(&t2.as_str())
                {
                    if let Some((SdEventKind::String, value, _)) = events.get(j + 1) {
                        arms.push((t2.clone(), value.clone()));
                    }
                    j += 2;
                    continue;
                }
                if *k2 == SdEventKind::String && fallback.is_none() {
                    fallback = Some(t2.clone());
                }
                j += 1;
            }
            if arms.is_empty() {
                units.push(unit(&message_key, fallback.and_then(|v| some_value(&v)), None));
            } else {
                for (cat, value) in arms {
                    units.push(unit(&message_key, some_value(&value), Some(cat)));
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    Some(ParsedCatalogue { embedded_locale: None, units })
}
