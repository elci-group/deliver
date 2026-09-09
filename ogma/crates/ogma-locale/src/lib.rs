//! Locale parsing, normalisation, fallback hierarchies and language
//! profiles for Ogma.
//!
//! Everything here is deterministic and table-driven. Locale identifiers are
//! normalised to a canonical BCP-47-like representation while the original
//! declaration is preserved on [`LocaleId::original`].

use ogma_model::{
    FallbackKind, FallbackRule, Language, LocaleId,
};

/// Text direction for a language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// Static linguistic requirements for a language.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LanguageProfile {
    pub language: Language,
    pub direction: TextDirection,
    /// CLDR plural categories the language requires, sorted.
    pub required_plural_categories: Vec<String>,
    pub requires_bidi: bool,
}

/// Two-letter ISO 639-1 codes Ogma recognises as valid language subtags.
const KNOWN_LANGUAGES: &[&str] = &[
    "aa", "ab", "ae", "af", "ak", "am", "an", "ar", "as", "av", "ay", "az", "ba", "be", "bg",
    "bh", "bi", "bm", "bn", "bo", "br", "bs", "ca", "ce", "ch", "co", "cr", "cs", "cu", "cv",
    "cy", "da", "de", "dv", "dz", "ee", "el", "en", "eo", "es", "et", "eu", "fa", "ff", "fi",
    "fj", "fo", "fr", "fy", "ga", "gd", "gl", "gn", "gu", "gv", "ha", "he", "hi", "ho", "hr",
    "ht", "hu", "hy", "hz", "ia", "id", "ie", "ig", "ii", "ik", "io", "is", "it", "iu", "ja",
    "jv", "ka", "kg", "ki", "kj", "kk", "kl", "km", "kn", "ko", "kr", "ks", "ku", "kv", "kw",
    "ky", "la", "lb", "lg", "li", "ln", "lo", "lt", "lu", "lv", "mg", "mh", "mi", "mk", "ml",
    "mn", "mr", "ms", "mt", "my", "na", "nb", "nd", "ne", "ng", "nl", "nn", "no", "nr", "nv",
    "ny", "oc", "oj", "om", "or", "os", "pa", "pi", "pl", "ps", "pt", "qu", "rm", "rn", "ro",
    "ru", "rw", "sa", "sc", "sd", "se", "sg", "si", "sk", "sl", "sm", "sn", "so", "sq", "sr",
    "ss", "st", "su", "sv", "sw", "ta", "te", "tg", "th", "ti", "tk", "tl", "tn", "to", "tr",
    "ts", "tt", "tw", "ty", "ug", "uk", "ur", "uz", "ve", "vi", "vo", "wa", "wo", "xh", "yi",
    "yo", "za", "zh", "zu",
];

/// Languages written right-to-left.
const RTL_LANGUAGES: &[&str] = &["ar", "fa", "he", "ps", "sd", "ug", "ur", "yi"];

/// Languages with a single `other` plural category.
const PLURAL_OTHER_ONLY: &[&str] = &[
    "id", "ja", "ko", "ms", "th", "vi", "zh", "fa",
];

/// Languages with `zero/one/two/few/many/other` plural categories.
const PLURAL_FULL: &[&str] = &["ar"];

/// Languages with `one/few/many/other` plural categories.
const PLURAL_FEW_MANY: &[&str] = &["be", "bs", "hr", "lt", "pl", "ru", "sr", "uk"];

/// Languages with `one/few/other` plural categories.
const PLURAL_FEW: &[&str] = &["cs", "ro", "sk"];

/// Languages with `zero/one/other` plural categories.
const PLURAL_ZERO_ONE: &[&str] = &["lv"];

/// Languages with `one/two/few/many/other` plural categories.
const PLURAL_MANY: &[&str] = &["ga", "mt"];

/// Returns true when `code` is a recognised ISO 639-1 language subtag.
pub fn is_known_language(code: &str) -> bool {
    KNOWN_LANGUAGES.contains(&code)
}

fn is_alpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// Parses and normalises a locale identifier such as `en`, `en-US`,
/// `en_GB`, or `zh-Hant-TW`. Returns `None` when the identifier has no
/// usable language subtag.
pub fn parse_locale(input: &str) -> Option<LocaleId> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split(['-', '_']).collect();
    let raw_lang = parts[0];
    if !(2..=3).contains(&raw_lang.len()) || !is_alpha(raw_lang) {
        return None;
    }
    let language = Language::new(raw_lang);
    if !is_known_language(language.code()) {
        return None;
    }

    let mut script: Option<String> = None;
    let mut region: Option<String> = None;
    let rest = &parts[1..];
    let mut i = 0;
    if i < rest.len() && rest[i].len() == 4 && is_alpha(rest[i]) {
        let s: String = rest[i].chars().flat_map(|c| c.to_lowercase()).collect();
        script = Some(
            s.chars()
                .enumerate()
                .map(|(n, c)| if n == 0 { c.to_ascii_uppercase() } else { c })
                .collect(),
        );
        i += 1;
    }
    if i < rest.len() && ((rest[i].len() == 2 && is_alpha(rest[i])) || (rest[i].len() == 3 && rest[i].chars().all(|c| c.is_ascii_digit()))) {
        region = Some(rest[i].to_ascii_uppercase());
        i += 1;
    }
    // Anything further (variants, extensions) is accepted but dropped from
    // the canonical form; the original declaration is preserved.
    let _ = i;

    let mut canonical = language.code().to_string();
    if let Some(s) = &script {
        canonical.push('-');
        canonical.push_str(s);
    }
    if let Some(r) = &region {
        canonical.push('-');
        canonical.push_str(r);
    }

    Some(LocaleId {
        original: trimmed.to_string(),
        canonical,
        language,
        script,
        region,
    })
}

/// Canonicalises a locale string, returning it unchanged when it cannot be
/// parsed (callers that need validation should use [`parse_locale`]).
pub fn canonicalize(input: &str) -> String {
    parse_locale(input)
        .map(|l| l.canonical)
        .unwrap_or_else(|| input.to_string())
}

/// Returns the static language profile for `lang`. Unknown languages get a
/// conservative LTR profile with `one/other` plural categories; profiles
/// never invent cultural assumptions beyond CLDR-derived plural/RTL facts.
pub fn profile_for(lang: &Language) -> LanguageProfile {
    let code = lang.code();
    let direction = if RTL_LANGUAGES.contains(&code) {
        TextDirection::Rtl
    } else {
        TextDirection::Ltr
    };
    let required_plural_categories: Vec<String> = if PLURAL_OTHER_ONLY.contains(&code) {
        vec!["other".to_string()]
    } else if PLURAL_FULL.contains(&code) {
        ["zero", "one", "two", "few", "many", "other"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else if PLURAL_FEW_MANY.contains(&code) {
        ["one", "few", "many", "other"].iter().map(|s| s.to_string()).collect()
    } else if PLURAL_FEW.contains(&code) {
        ["one", "few", "other"].iter().map(|s| s.to_string()).collect()
    } else if PLURAL_ZERO_ONE.contains(&code) {
        ["zero", "one", "other"].iter().map(|s| s.to_string()).collect()
    } else if PLURAL_MANY.contains(&code) {
        ["one", "two", "few", "many", "other"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        ["one", "other"].iter().map(|s| s.to_string()).collect()
    };
    LanguageProfile {
        language: lang.clone(),
        direction,
        required_plural_categories,
        requires_bidi: direction == TextDirection::Rtl,
    }
}

/// Computes the deterministic fallback resolution chain for `from` against
/// the set of `available` locales.
///
/// The chain is, in priority order:
/// 1. the same locale with region dropped (e.g. `fr-CA` → `fr`) — [`FallbackKind::Regional`],
/// 2. the same locale with script dropped — [`FallbackKind::LinguisticChain`],
/// 3. any available locale sharing the primary language — [`FallbackKind::LinguisticChain`],
/// 4. the application baseline, when provided and of another language —
///    [`FallbackKind::PrimaryLanguage`].
///
/// An exact match in `available` produces an empty chain: the locale is
/// fully supported and never falls back.
pub fn fallback_chain(
    from: &LocaleId,
    available: &[LocaleId],
    baseline: Option<&LocaleId>,
) -> Vec<FallbackRule> {
    let mut rules = Vec::new();
    let available: Vec<&LocaleId> = {
        let mut v: Vec<&LocaleId> = available.iter().collect();
        v.sort();
        v.dedup();
        v
    };
    if available.iter().any(|a| a.canonical == from.canonical) {
        return rules;
    }

    let mut seen: Vec<String> = Vec::new();
    fn push(
        seen: &mut Vec<String>,
        from: &LocaleId,
        target: &LocaleId,
        kind: FallbackKind,
        rules: &mut Vec<FallbackRule>,
    ) {
        if !seen.contains(&target.canonical) {
            seen.push(target.canonical.clone());
            rules.push(FallbackRule {
                requested: from.clone(),
                target: target.clone(),
                kind,
            });
        }
    }

    // 1. drop region
    if let (Some(script), Some(_region)) = (&from.script, &from.region) {
        let candidate = format!("{}-{}", from.language.code(), script);
        if let Some(t) = available.iter().find(|a| a.canonical == candidate) {
            push(&mut seen, from, t, FallbackKind::Regional, &mut rules);
        }
    } else if from.region.is_some() {
        let candidate = from.language.code().to_string();
        if let Some(t) = available.iter().find(|a| a.canonical == candidate) {
            push(&mut seen, from, t, FallbackKind::Regional, &mut rules);
        }
    }

    // 2. drop script
    if from.script.is_some() {
        let candidate = match &from.region {
            Some(r) => format!("{}-{}", from.language.code(), r),
            None => from.language.code().to_string(),
        };
        if let Some(t) = available.iter().find(|a| a.canonical == candidate) {
            push(&mut seen, from, t, FallbackKind::LinguisticChain, &mut rules);
        }
    }

    // 3. language-only
    let candidate = from.language.code().to_string();
    if let Some(t) = available
        .iter()
        .find(|a| a.canonical == candidate && a.canonical != from.canonical)
    {
        push(&mut seen, from, t, FallbackKind::LinguisticChain, &mut rules);
    }
    for t in &available {
        if t.language == from.language && t.canonical != from.canonical && !seen.contains(&t.canonical) {
            // same language, different subtags: acceptable linguistic fallback
            push(&mut seen, from, t, FallbackKind::LinguisticChain, &mut rules);
        }
    }

    // 4. baseline
    if let Some(b) = baseline {
        if b.language != from.language {
            push(&mut seen, from, b, FallbackKind::PrimaryLanguage, &mut rules);
        }
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> LocaleId {
        parse_locale(s).unwrap()
    }

    #[test]
    fn parses_simple_language() {
        let l = loc("en");
        assert_eq!(l.canonical, "en");
        assert_eq!(l.language.code(), "en");
        assert!(l.script.is_none() && l.region.is_none());
    }

    #[test]
    fn normalises_separators_and_case() {
        let l = loc("EN_us");
        assert_eq!(l.canonical, "en-US");
        assert_eq!(l.original, "EN_us");
        assert_eq!(l.region.as_deref(), Some("US"));
    }

    #[test]
    fn parses_script_and_region() {
        let l = loc("zh-hant-tw");
        assert_eq!(l.canonical, "zh-Hant-TW");
        assert_eq!(l.script.as_deref(), Some("Hant"));
        assert_eq!(l.region.as_deref(), Some("TW"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_locale("").is_none());
        assert!(parse_locale("1234").is_none());
        assert!(parse_locale("xx-not-a-lang").is_none());
    }

    #[test]
    fn arabic_is_rtl_with_full_plurals() {
        let p = profile_for(&Language::new("ar"));
        assert_eq!(p.direction, TextDirection::Rtl);
        assert!(p.requires_bidi);
        assert_eq!(p.required_plural_categories.len(), 6);
    }

    #[test]
    fn japanese_has_other_only_plural() {
        let p = profile_for(&Language::new("ja"));
        assert_eq!(p.direction, TextDirection::Ltr);
        assert_eq!(p.required_plural_categories, vec!["other".to_string()]);
    }

    #[test]
    fn russian_requires_few_and_many() {
        let p = profile_for(&Language::new("ru"));
        assert_eq!(p.required_plural_categories, vec!["one", "few", "many", "other"]);
    }

    #[test]
    fn fallback_exact_match_is_empty() {
        let available = vec![loc("fr"), loc("en")];
        assert!(fallback_chain(&loc("fr"), &available, Some(&loc("en"))).is_empty());
    }

    #[test]
    fn fallback_regional_then_language() {
        let available = vec![loc("fr"), loc("en")];
        let chain = fallback_chain(&loc("fr-CA"), &available, Some(&loc("en")));
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].target.canonical, "fr");
        assert_eq!(chain[0].kind, FallbackKind::Regional);
        assert_eq!(chain[1].target.canonical, "en");
        assert_eq!(chain[1].kind, FallbackKind::PrimaryLanguage);
        assert!(chain[1].crosses_language());
    }

    #[test]
    fn fallback_language_only_is_linguistic() {
        let available = vec![loc("de-AT"), loc("en")];
        let chain = fallback_chain(&loc("de"), &available, Some(&loc("en")));
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].kind, FallbackKind::LinguisticChain);
        assert!(!chain[0].crosses_language());
    }
}
