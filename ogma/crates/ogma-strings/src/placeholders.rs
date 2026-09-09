use std::collections::BTreeSet;

/// Extract interpolation placeholders from a value, sorted and deduplicated.
///
/// Supported shapes, matched with priority so overlapping forms are never
/// double-counted:
/// - `%(name)s` named printf (letters/digits/underscore name, `s`/`d`/`x`/`f`)
/// - `%1$s` positional printf (digits, `$`, `s`/`d`/`x`/`f`)
/// - `%s %d %f %x %@ %%` simple printf
/// - `{{name}}` mustache (letters/digits/underscore, not starting with a
///   digit unless purely digits)
/// - `{name}` icu-style (same name rule; `{0}` is a positional placeholder)
pub fn extract_placeholders(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if let Some((token, next)) = match_named_printf(value, i) {
                    out.insert(token);
                    i = next;
                    continue;
                }
                if let Some((token, next)) = match_positional_printf(value, i) {
                    out.insert(token);
                    i = next;
                    continue;
                }
                if i + 1 < bytes.len() && matches!(bytes[i + 1], b's' | b'd' | b'f' | b'x' | b'@' | b'%') {
                    out.insert(value[i..=i + 1].to_string());
                    i += 2;
                    continue;
                }
                i += 1;
            }
            b'{' => {
                if let Some((token, next)) = match_mustache(value, i) {
                    out.insert(token);
                    i = next;
                    continue;
                }
                if let Some((token, next)) = match_brace(value, i) {
                    out.insert(token);
                    i = next;
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    out.into_iter().collect()
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn valid_brace_name(name: &str) -> bool {
    if name.is_empty() || !name.bytes().all(is_name_char) {
        return false;
    }
    // A name starting with a digit is only valid when purely digits ({0}).
    !name.as_bytes()[0].is_ascii_digit() || name.bytes().all(|c| c.is_ascii_digit())
}

fn scan_name(value: &str, start: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut j = start;
    while j < bytes.len() && is_name_char(bytes[j]) {
        j += 1;
    }
    if j > start {
        Some(j)
    } else {
        None
    }
}

fn match_named_printf(value: &str, i: usize) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    if bytes.get(i + 1) != Some(&b'(') {
        return None;
    }
    let name_start = i + 2;
    let name_end = scan_name(value, name_start)?;
    let name = &value[name_start..name_end];
    // Named printf arguments do not start with a digit.
    if name.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if bytes.get(name_end) != Some(&b')') {
        return None;
    }
    if let Some(&conv) = bytes.get(name_end + 1) {
        if matches!(conv, b's' | b'd' | b'x' | b'f') {
            return Some((value[i..=name_end + 1].to_string(), name_end + 2));
        }
    }
    None
}

fn match_positional_printf(value: &str, i: usize) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    let mut j = i + 1;
    let digits_start = j;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j == digits_start || bytes.get(j) != Some(&b'$') {
        return None;
    }
    if let Some(&conv) = bytes.get(j + 1) {
        if matches!(conv, b's' | b'd' | b'x' | b'f') {
            return Some((value[i..=j + 1].to_string(), j + 2));
        }
    }
    None
}

fn match_mustache(value: &str, i: usize) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    if bytes.get(i + 1) != Some(&b'{') {
        return None;
    }
    let name_start = i + 2;
    let name_end = scan_name(value, name_start)?;
    let name = &value[name_start..name_end];
    if !valid_brace_name(name) {
        return None;
    }
    if value[name_end..].starts_with("}}") {
        Some((value[i..name_end + 2].to_string(), name_end + 2))
    } else {
        None
    }
}

fn match_brace(value: &str, i: usize) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    let name_start = i + 1;
    let name_end = scan_name(value, name_start)?;
    let name = &value[name_start..name_end];
    if !valid_brace_name(name) {
        return None;
    }
    if bytes.get(name_end) == Some(&b'}') {
        Some((value[i..=name_end].to_string(), name_end + 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_printf_forms() {
        assert_eq!(
            extract_placeholders("Hello %s, you have %d of %f and %x and %@"),
            vec!["%@", "%d", "%f", "%s", "%x"]
        );
    }

    #[test]
    fn positional_printf() {
        assert_eq!(
            extract_placeholders("%1$s %2$d then %s"),
            vec!["%1$s", "%2$d", "%s"]
        );
    }

    #[test]
    fn named_printf() {
        assert_eq!(
            extract_placeholders("%(count)d items for %(name)s"),
            vec!["%(count)d", "%(name)s"]
        );
    }

    #[test]
    fn brace_and_mustache() {
        assert_eq!(
            extract_placeholders("Hi {name}, {{title}} {0}"),
            vec!["{0}", "{name}", "{{title}}"]
        );
    }

    #[test]
    fn mustache_not_double_counted_as_brace() {
        assert_eq!(extract_placeholders("{{user}}"), vec!["{{user}}"]);
    }

    #[test]
    fn named_printf_not_double_counted_as_percent_s() {
        assert_eq!(extract_placeholders("%(name)s"), vec!["%(name)s"]);
        assert_eq!(extract_placeholders("%1$s"), vec!["%1$s"]);
    }

    #[test]
    fn invalid_names_ignored() {
        assert_eq!(extract_placeholders("{9lives} {} %(9x)s"), Vec::<String>::new());
        // A printf token inside braces is still a printf token.
        assert_eq!(extract_placeholders("{%s}"), vec!["%s"]);
    }

    #[test]
    fn positional_digits_allowed() {
        assert_eq!(extract_placeholders("{0} {12}"), vec!["{0}", "{12}"]);
    }

    #[test]
    fn plural_suffix_keys_untouched() {
        assert_eq!(extract_placeholders("items_zero"), Vec::<String>::new());
    }

    #[test]
    fn dedup_and_sorted() {
        assert_eq!(
            extract_placeholders("%s {b} %s {a} {b}"),
            vec!["%s", "{a}", "{b}"]
        );
    }

    #[test]
    fn percent_escaped_percent() {
        assert_eq!(extract_placeholders("100%% of %s"), vec!["%%", "%s"]);
    }
}
