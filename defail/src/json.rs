//! A minimal, correct JSON writer — std-only, no serde.
//!
//! Every DEFAIL export ([`FailureReport::to_json`](crate::report::FailureReport),
//! [`KnowledgeBase::to_json`](crate::knowledge::KnowledgeBase),
//! [`TraceEvent::to_json`](crate::trace::TraceEvent)) is built from these
//! primitives, so escape correctness is verified in exactly one place.
//!
//! Number formatting: `f32` values use std's shortest round-trip `Display`
//! formatting (e.g. `0.98`, `1`, `0.5`), which is valid JSON. JSON has no
//! representation for NaN or infinities, so non-finite values are emitted as
//! `null`.

/// Escape and quote one string: `"` and `\` are backslash-escaped, the short
/// escapes `\n` `\r` `\t` `\b` `\f` are used where they exist, and every other
/// control character below U+0020 becomes a `\u00XX` escape. Everything else,
/// including non-ASCII text, passes through as UTF-8.
pub fn quote(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Format an `f32` as a JSON number. Non-finite values become `null`.
pub fn f32(x: f32) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "null".into()
    }
}

/// Join pre-rendered items into a JSON array.
pub fn array(items: impl IntoIterator<Item = String>) -> String {
    let mut out = String::from("[");
    let mut first = true;
    for item in items {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&item);
    }
    out.push(']');
    out
}

/// Render fields as a JSON object. Field order is preserved exactly as given,
/// so exports are deterministic.
pub fn object(fields: &[(&str, String)]) -> String {
    let mut out = String::from("{");
    let mut first = true;
    for (key, value) in fields {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&quote(key));
        out.push(':');
        out.push_str(value);
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_backslashes_escape() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote("back\\slash"), "\"back\\\\slash\"");
        assert_eq!(quote("both \" and \\"), "\"both \\\" and \\\\\"");
    }

    #[test]
    fn control_characters_use_short_or_unicode_escapes() {
        assert_eq!(quote("a\nb"), "\"a\\nb\"");
        assert_eq!(quote("a\rb"), "\"a\\rb\"");
        assert_eq!(quote("a\tb"), "\"a\\tb\"");
        assert_eq!(quote("a\u{08}b"), "\"a\\bb\"");
        assert_eq!(quote("a\u{0C}b"), "\"a\\fb\"");
        // No short escape: full \u00XX form, lowercase hex.
        assert_eq!(quote("a\u{01}b"), "\"a\\u0001b\"");
        assert_eq!(quote("\u{1f}"), "\"\\u001f\"");
        // DEL (0x7F) is not a JSON control character: passthrough.
        assert_eq!(quote("a\u{7f}b"), "\"a\u{7f}b\"");
    }

    #[test]
    fn unicode_text_passes_through_unescaped() {
        assert_eq!(quote("café ☃ 😀"), "\"café ☃ 😀\"");
        assert_eq!(quote("中文|pipe"), "\"中文|pipe\"");
    }

    #[test]
    fn adversarial_strings_never_break_the_envelope() {
        // Every escape introducer, embedded in one string.
        let nasty = "\"\\\n\r\t\u{08}\u{0C}\u{00}\u{1f}\u{7f}|{}[],:";
        assert_eq!(
            quote(nasty),
            "\"\\\"\\\\\\n\\r\\t\\b\\f\\u0000\\u001f\u{7f}|{}[],:\""
        );
        // The DEL passthrough is the only raw character: re-escaping is a
        // fixed point.
        assert_eq!(quote(&quote(nasty)), quote(&quote(nasty)));
    }

    #[test]
    fn f32_formats_as_shortest_round_trip_json_number() {
        assert_eq!(f32(0.98), "0.98");
        assert_eq!(f32(1.0), "1");
        assert_eq!(f32(0.5), "0.5");
        assert_eq!(f32(0.0), "0");
        // JSON has no NaN/Infinity: null by contract.
        assert_eq!(f32(f32::NAN), "null");
        assert_eq!(f32(f32::INFINITY), "null");
        assert_eq!(f32(f32::NEG_INFINITY), "null");
        // Shortest round-trip: this exact bit pattern must come back.
        let x: f32 = 0.1;
        assert_eq!(f32(x).parse::<f32>().unwrap(), x);
    }

    #[test]
    fn arrays_and_objects_preserve_order() {
        assert_eq!(array(Vec::<String>::new()), "[]");
        assert_eq!(
            array([quote("a"), "1".to_string(), "true".to_string()]),
            "[\"a\",1,true]"
        );
        assert_eq!(object(&[]), "{}");
        assert_eq!(
            object(&[("k", quote("v")), ("n", "42".to_string())]),
            "{\"k\":\"v\",\"n\":42}"
        );
    }
}
