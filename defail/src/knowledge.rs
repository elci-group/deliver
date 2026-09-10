//! The knowledge base: every failure produces structured knowledge about how
//! that failure can be resolved next time.
//!
//! The map is `failure class + context signature → remediation → outcome`.
//! If the same failure happens 100 times, the system does not invent 100
//! responses: a previously successful remediation is recommended directly.

use std::collections::BTreeMap;
use std::fmt;

use crate::declare::{FailureClass, RemedyId};
use crate::json;

/// Version marker written as the first line of every saved record file.
/// Files without this header are read as format v1 (see [`KnowledgeBase::load_records`]).
pub const KB_FORMAT_V2_HEADER: &str = "defail-kb v2";

/// Where in the execution graph a failure was seen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContextSig(pub String);

impl fmt::Display for ContextSig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KbKey {
    pub class: FailureClass,
    pub context: ContextSig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbEntry {
    pub remedy: RemedyId,
    pub verification_desc: String,
    pub successes: u32,
    pub failures: u32,
    /// Outcomes learned under a *different* remedy id than `remedy` at the
    /// same key (remedy-id divergence): remedy id → (successes, failures).
    /// Never silently attributed to the first-learned remedy.
    pub alt_remedies: BTreeMap<RemedyId, (u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbError {
    BadRecord(String),
}

impl fmt::Display for KbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KbError::BadRecord(detail) => write!(f, "bad knowledge record: {detail}"),
        }
    }
}

impl std::error::Error for KbError {}

/// What happened while loading records: how many records were parsed and
/// where duplicate keys overwrote an earlier record (last wins is retained,
/// but the overwrite is reported, with 1-based line numbers).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Record lines parsed, excluding the format header.
    pub records: usize,
    /// 1-based line numbers of records that overwrote an earlier record
    /// with the same key.
    pub duplicates: Vec<usize>,
    /// True when the source file was zero-length: the load succeeds as an
    /// empty bank (there is nothing to parse), but no valid bank is ever
    /// zero bytes — this crate writes at least the format header — so the
    /// flag tells the caller the save likely never happened. Only set by
    /// the file-based loader ([`crate::store::KnowledgeStore::load_reported`]);
    /// [`KnowledgeBase::load_records`] cannot see file length.
    pub empty_file: bool,
}

/// Deterministic record of what has worked where. Iteration order is stable
/// (BTreeMap) so reports and persistence round-trip exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeBase {
    entries: BTreeMap<KbKey, KbEntry>,
}

impl KnowledgeBase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&KbKey, &KbEntry)> {
        self.entries.iter()
    }

    /// Record the outcome of one executed remediation attempt. When the key
    /// is already known under a *different* remedy id, the outcome is
    /// recorded as divergence in `alt_remedies` — never attributed to the
    /// first-learned remedy. Counts saturate at `u32::MAX` rather than
    /// overflowing (loads reject `u32::MAX` counts up front, since they can
    /// never be incremented).
    pub fn learn(
        &mut self,
        key: KbKey,
        remedy: RemedyId,
        verification_desc: String,
        succeeded: bool,
    ) {
        let entry = self.entries.entry(key).or_insert(KbEntry {
            remedy: remedy.clone(),
            verification_desc,
            successes: 0,
            failures: 0,
            alt_remedies: BTreeMap::new(),
        });
        if entry.remedy == remedy {
            if succeeded {
                entry.successes = entry.successes.saturating_add(1);
            } else {
                entry.failures = entry.failures.saturating_add(1);
            }
        } else {
            let alt = entry.alt_remedies.entry(remedy).or_insert((0, 0));
            if succeeded {
                alt.0 = alt.0.saturating_add(1);
            } else {
                alt.1 = alt.1.saturating_add(1);
            }
        }
    }

    /// Deterministic recommendation: the recorded remedy for this exact
    /// failure class and context, but only while it succeeds more than it
    /// fails.
    pub fn recommend(&self, class: &FailureClass, context: &ContextSig) -> Option<&KbEntry> {
        self.entries
            .get(&KbKey {
                class: class.clone(),
                context: context.clone(),
            })
            .filter(|e| e.successes > e.failures)
    }

    /// Merge learned outcomes from another knowledge base. The merge is a
    /// deterministic join: for every remedy mentioned on either side (as a
    /// primary remedy or as an alternative-remedy count) the strongest
    /// recorded outcome wins component-wise; the stronger of the two primary
    /// remedies leads the merged entry and everything else remains visible
    /// as alternative-remedy counts. The join is commutative, associative,
    /// and idempotent — banks merge in any order, grouping never matters,
    /// and re-merging a bank that is already merged in changes nothing.
    pub fn absorb(&mut self, other: &KnowledgeBase) {
        for (key, entry) in other.entries() {
            match self.entries.get(key) {
                None => {
                    self.entries.insert(key.clone(), entry.clone());
                }
                Some(existing) => {
                    let merged = merge_entries(existing, entry);
                    self.entries.insert(key.clone(), merged);
                }
            }
        }
    }

    /// Versioned line-oriented records (format v2): a `defail-kb v2` header
    /// followed by `class|context|remedy|successes|failures|verification`
    /// lines, with an optional seventh field carrying alternative-remedy
    /// counts. Every field is backslash-escaped, so `|`, newlines, and
    /// backslashes in adversarial strings round-trip exactly.
    pub fn to_records(&self) -> Vec<String> {
        let mut out = vec![KB_FORMAT_V2_HEADER.to_string()];
        for (key, entry) in &self.entries {
            let mut line = format!(
                "{}|{}|{}|{}|{}|{}",
                escape_field(&key.class.to_string()),
                escape_field(&key.context.0),
                escape_field(&entry.remedy.to_string()),
                entry.successes,
                entry.failures,
                escape_field(&entry.verification_desc),
            );
            if !entry.alt_remedies.is_empty() {
                let alts = entry
                    .alt_remedies
                    .iter()
                    .map(|(remedy, (successes, failures))| {
                        format!("{}={successes}/{failures}", escape_field(&remedy.to_string()))
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                line.push('|');
                line.push_str(&alts);
            }
            out.push(line);
        }
        out
    }

    /// Render the bank as versioned JSON (`"schema": "defail-kb/2"`, matching
    /// the v2 record format), with entries in the same deterministic
    /// BTreeMap order as [`KnowledgeBase::to_records`].
    pub fn to_json(&self) -> String {
        let entries = self.entries.iter().map(|(key, entry)| {
            let alt_remedies = entry.alt_remedies.iter().map(|(remedy, (successes, failures))| {
                json::object(&[
                    ("remedy", json::quote(remedy.as_str())),
                    ("successes", successes.to_string()),
                    ("failures", failures.to_string()),
                ])
            });
            json::object(&[
                ("class", json::quote(key.class.as_str())),
                ("context", json::quote(&key.context.0)),
                ("remedy", json::quote(entry.remedy.as_str())),
                ("successes", entry.successes.to_string()),
                ("failures", entry.failures.to_string()),
                ("verification", json::quote(&entry.verification_desc)),
                ("alt_remedies", json::array(alt_remedies)),
            ])
        });
        json::object(&[
            ("schema", json::quote("defail-kb/2")),
            ("entries", json::array(entries)),
        ])
    }

    /// Load records produced by [`KnowledgeBase::to_records`], returning a
    /// [`LoadReport`] with duplicate-key diagnostics. Two formats are
    /// accepted:
    ///
    /// * **v2** (header `defail-kb v2`): all fields are backslash-escaped;
    ///   an optional seventh field carries alternative-remedy counts.
    /// * **v1** (no header, read compatibility): six raw pipe-separated
    ///   fields; only the verification description was historically
    ///   sanitized (`|` → `/`), and no unescaping is applied.
    ///
    /// Duplicate keys keep the last record (retained behavior) and are
    /// reported in [`LoadReport::duplicates`]. A bad line aborts the load
    /// with line-level diagnostics, without partial mutation.
    ///
    /// **Format ambiguity, accepted by design:** a v1 file has no header, so
    /// a v1 bank whose *first record* is literally `defail-kb v2` has that
    /// line consumed as the format header and the rest of the file parsed as
    /// escaped v2 records. This is deterministic and intentionally not
    /// disambiguated by heuristic parsing.
    pub fn load_records<I: IntoIterator<Item = String>>(
        &mut self,
        records: I,
    ) -> Result<LoadReport, KbError> {
        let mut loaded = BTreeMap::new();
        let mut report = LoadReport::default();
        let mut versioned = false;
        for (n, line) in records.into_iter().enumerate() {
            let line_no = n + 1;
            let bad = |detail: String| KbError::BadRecord(format!("record {line_no}: {detail}"));
            if line_no == 1 && line == KB_FORMAT_V2_HEADER {
                versioned = true;
                continue;
            }
            let raw_fields = if versioned {
                split_escaped_raw(&line, '|').map_err(bad)?
            } else {
                line.split('|').map(str::to_string).collect::<Vec<_>>()
            };
            if raw_fields.len() != 6 && !(versioned && raw_fields.len() == 7) {
                return Err(bad(format!(
                    "has {} fields, expected {}",
                    raw_fields.len(),
                    if versioned { "6 or 7" } else { "6" }
                )));
            }
            let successes: u32 = raw_fields[3]
                .parse()
                .map_err(|_| bad("successes is not a number".into()))?;
            let failures: u32 = raw_fields[4]
                .parse()
                .map_err(|_| bad("failures is not a number".into()))?;
            for (field, count) in [("successes", successes), ("failures", failures)] {
                if count == u32::MAX {
                    // Uncountable: the next `learn` could never increment it
                    // (it would saturate), so the record is meaningless.
                    return Err(bad(format!("{field} is uncountable (u32::MAX)")));
                }
            }
            // The alternative-remedy field keeps its own escape level: it is
            // parsed by split_escaped_raw above but unescaped inside
            // parse_alt_field, where its `,` and `=` separators live.
            let alt_remedies = if versioned && raw_fields.len() == 7 {
                parse_alt_field(&raw_fields[6], &bad)?
            } else {
                BTreeMap::new()
            };
            let field = |i: usize| unescape(&raw_fields[i]).map_err(bad);
            let key = KbKey {
                class: FailureClass::new(field(0)?),
                context: ContextSig(field(1)?),
            };
            let entry = KbEntry {
                remedy: RemedyId::new(field(2)?),
                verification_desc: field(5)?,
                successes,
                failures,
                alt_remedies,
            };
            report.records += 1;
            if loaded.insert(key, entry).is_some() {
                report.duplicates.push(line_no);
            }
        }
        self.entries.extend(loaded);
        Ok(report)
    }
}

/// The semilattice join of two entries recorded at the same key. Every
/// remedy mentioned by either side — each side's primary remedy and its
/// alternative-remedy counts — is unioned component-wise (the strongest
/// outcome per remedy wins). The merged entry is led by the stronger of the
/// two *primary* remedies; alternative remedies inform the counts but never
/// displace a primary, so merging a bank with itself is exactly the
/// identity. The rest stay as alternative-remedy counts.
fn merge_entries(a: &KbEntry, b: &KbEntry) -> KbEntry {
    let mut counts: BTreeMap<RemedyId, (u32, u32)> = BTreeMap::new();
    let mut record = |remedy: &RemedyId, outcome: (u32, u32)| {
        let slot = counts.entry(remedy.clone()).or_insert((0, 0));
        slot.0 = slot.0.max(outcome.0);
        slot.1 = slot.1.max(outcome.1);
    };
    record(&a.remedy, (a.successes, a.failures));
    for (remedy, outcome) in &a.alt_remedies {
        record(remedy, *outcome);
    }
    record(&b.remedy, (b.successes, b.failures));
    for (remedy, outcome) in &b.alt_remedies {
        record(remedy, *outcome);
    }
    // The leader is the stronger of the two primary remedies under a total
    // order (more successes, then fewer failures, then remedy id).
    // Alternative remedies inform the merged counts but never displace a
    // primary: `learn` pins the primary to the first remedy tried, and it
    // may be weaker than a later alternative — demoting it here would make
    // the join fail the identity law (A joined with A must be A).
    let (leader, (successes, failures)) = [a.remedy.clone(), b.remedy.clone()]
        .into_iter()
        .map(|remedy| {
            let outcome = counts[&remedy];
            (remedy, outcome)
        })
        .max_by(|x, y| {
            x.1 .0
                .cmp(&y.1 .0)
                .then_with(|| y.1 .1.cmp(&x.1 .1))
                .then_with(|| y.0.cmp(&x.0))
        })
        .expect("both entries contribute a primary remedy");
    let mut alt_remedies = counts;
    alt_remedies.remove(&leader);
    // The leader keeps the verification description recorded for its remedy;
    // when both sides recorded it, the lexicographically smaller description
    // wins, which is symmetric and associative like the join itself.
    let verification_desc = [a, b]
        .iter()
        .filter(|entry| entry.remedy == leader)
        .map(|entry| entry.verification_desc.as_str())
        .min()
        .unwrap_or_default()
        .to_string();
    KbEntry {
        remedy: leader,
        verification_desc,
        successes,
        failures,
        alt_remedies,
    }
}

/// Escape one record field: backslash, pipe, newline, carriage return, and
/// the separators used inside the alternative-remedy field (`,` and `=`)
/// gain a backslash prefix, so adversarial strings round-trip.
fn escape_field(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ',' => out.push_str("\\,"),
            '=' => out.push_str("\\="),
            c => out.push(c),
        }
    }
    out
}

/// Split at every *unescaped* occurrence of `sep`, leaving escape sequences
/// intact in the returned tokens.
fn split_escaped_raw(line: &str, sep: char) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut raw = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                raw.push('\\');
                match chars.next() {
                    Some(next) => raw.push(next),
                    None => return Err("trailing backslash in record field".into()),
                }
            }
            c if c == sep => {
                tokens.push(std::mem::take(&mut raw));
            }
            c => raw.push(c),
        }
    }
    tokens.push(raw);
    Ok(tokens)
}

/// Undo [`escape_field`].
fn unescape(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('|') => out.push('|'),
            Some(',') => out.push(','),
            Some('=') => out.push('='),
            Some(other) => return Err(format!("unknown escape sequence `\\{other}`")),
            None => return Err("trailing backslash in record field".into()),
        }
    }
    Ok(out)
}

/// Split off the first *unescaped* occurrence of `sep`, unescaping both
/// halves. `None` when no unescaped separator exists.
fn split_first_escaped(s: &str, sep: char) -> Option<(String, String)> {
    let mut raw = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                raw.push('\\');
                raw.push(chars.next()?);
            }
            c if c == sep => {
                return Some((unescape(&raw).ok()?, unescape(chars.as_str()).ok()?));
            }
            c => raw.push(c),
        }
    }
    None
}

/// Parse the seventh v2 field: `remedy=successes/failures` entries joined
/// by commas, where remedy ids may themselves contain escaped characters.
fn parse_alt_field(
    field: &str,
    bad: &impl Fn(String) -> KbError,
) -> Result<BTreeMap<RemedyId, (u32, u32)>, KbError> {
    let mut alts = BTreeMap::new();
    if field.is_empty() {
        return Ok(alts);
    }
    for item in split_escaped_raw(field, ',').map_err(bad)? {
        let Some((remedy, counts)) = split_first_escaped(&item, '=') else {
            return Err(bad("alternative remedy entry is missing `=`".into()));
        };
        let Some((successes, failures)) = counts.split_once('/') else {
            return Err(bad(format!(
                "alternative remedy `{remedy}` is missing its `/` counts"
            )));
        };
        let successes: u32 = successes
            .parse()
            .map_err(|_| bad(format!("alternative remedy `{remedy}` has a bad success count")))?;
        let failures: u32 = failures
            .parse()
            .map_err(|_| bad(format!("alternative remedy `{remedy}` has a bad failure count")))?;
        if successes == u32::MAX || failures == u32::MAX {
            return Err(bad(format!(
                "alternative remedy `{remedy}` has an uncountable (u32::MAX) count"
            )));
        }
        alts.insert(RemedyId::new(remedy), (successes, failures));
    }
    Ok(alts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> KbKey {
        KbKey {
            class: FailureClass::new("capacity/rate-limit"),
            context: ContextSig("after:start;at:Client.call".into()),
        }
    }

    #[test]
    fn recommends_only_what_demonstrably_works() {
        let mut kb = KnowledgeBase::new();
        assert!(kb
            .recommend(
                &FailureClass::new("capacity/rate-limit"),
                &ContextSig("after:start;at:Client.call".into())
            )
            .is_none());
        kb.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        assert!(kb
            .recommend(
                &FailureClass::new("capacity/rate-limit"),
                &ContextSig("after:start;at:Client.call".into())
            )
            .is_some());
        // One failure now balances one success: no recommendation.
        kb.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), false);
        assert!(kb
            .recommend(
                &FailureClass::new("capacity/rate-limit"),
                &ContextSig("after:start;at:Client.call".into())
            )
            .is_none());
    }

    #[test]
    fn records_round_trip() {
        let mut kb = KnowledgeBase::new();
        kb.learn(
            key(),
            RemedyId::new("fallback-b"),
            "provider B accepted".into(),
            true,
        );
        kb.learn(
            key(),
            RemedyId::new("fallback-b"),
            "provider B accepted".into(),
            true,
        );
        let records = kb.to_records();
        assert_eq!(records[0], KB_FORMAT_V2_HEADER);
        let mut loaded = KnowledgeBase::new();
        loaded.load_records(records.clone()).unwrap();
        assert_eq!(loaded.to_records(), records);
        assert!(loaded.load_records(vec!["garbage".into()]).is_err());
    }

    #[test]
    fn v2_records_escape_adversarial_fields() {
        let mut kb = KnowledgeBase::new();
        let key = KbKey {
            class: FailureClass::new("we|rd/class"),
            context: ContextSig("line1\nline2 \\ tail".into()),
        };
        kb.learn(
            key,
            RemedyId::new("remedy,=tricky"),
            "pipe | here\nand \\ back".into(),
            true,
        );
        // Divergence under an equally adversarial remedy id must persist too.
        kb.learn(
            KbKey {
                class: FailureClass::new("we|rd/class"),
                context: ContextSig("line1\nline2 \\ tail".into()),
            },
            RemedyId::new("alt=remedy,|"),
            "alt desc".into(),
            false,
        );
        let records = kb.to_records();
        // Every record is exactly one line, and no raw separator leaks.
        for record in &records[1..] {
            assert!(!record.contains('\n'));
        }
        let mut loaded = KnowledgeBase::new();
        let report = loaded.load_records(records.clone()).unwrap();
        assert_eq!(report.duplicates, Vec::<usize>::new());
        assert_eq!(loaded.to_records(), records);
        let entry = loaded.entries().next().unwrap().1;
        assert_eq!(
            entry.alt_remedies.keys().next().unwrap().as_str(),
            "alt=remedy,|"
        );
        assert_eq!(entry.alt_remedies.values().next().unwrap(), &(0, 1));
    }

    #[test]
    fn v1_records_still_load() {
        // v0.2 banks: no header, six raw fields, `|` already sanitized to `/`.
        let v1 = vec![
            "capacity/rate-limit|after:start;at:Client.call|fallback-b|2|1|provider B / quota accepted".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        let report = kb.load_records(v1).unwrap();
        assert_eq!(report.records, 1);
        let entry = kb.entries().next().unwrap().1;
        assert_eq!(entry.remedy.as_str(), "fallback-b");
        assert_eq!((entry.successes, entry.failures), (2, 1));
        assert_eq!(entry.verification_desc, "provider B / quota accepted");
    }

    #[test]
    fn duplicate_keys_last_wins_and_are_reported() {
        let records = vec![
            KB_FORMAT_V2_HEADER.to_string(),
            "capacity/rate-limit|ctx|remedy-a|1|0|first".to_string(),
            "capacity/rate-limit|ctx|remedy-b|2|3|second".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        let report = kb.load_records(records).unwrap();
        assert_eq!(report.records, 2);
        assert_eq!(report.duplicates, vec![3]);
        let entry = kb.entries().next().unwrap().1;
        assert_eq!(entry.remedy.as_str(), "remedy-b");
        assert_eq!((entry.successes, entry.failures), (2, 3));
        assert_eq!(entry.verification_desc, "second");
    }

    #[test]
    fn malformed_v2_records_report_the_line() {
        let bad_escape = vec![
            KB_FORMAT_V2_HEADER.to_string(),
            "capacity/rate-limit|ctx|remedy-a|1|0|bad \\q escape".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        let err = kb.load_records(bad_escape).unwrap_err();
        let KbError::BadRecord(detail) = err;
        assert!(detail.starts_with("record 2:"), "got {detail}");
        assert!(detail.contains("unknown escape sequence"));

        let wrong_fields = vec![KB_FORMAT_V2_HEADER.to_string(), "a|b|c".to_string()];
        let err = kb.load_records(wrong_fields).unwrap_err();
        assert!(matches!(err, KbError::BadRecord(_)));
    }

    #[test]
    fn learn_records_remedy_divergence() {
        let mut kb = KnowledgeBase::new();
        kb.learn(key(), RemedyId::new("remedy-a"), "verified".into(), true);
        kb.learn(key(), RemedyId::new("remedy-b"), "verified".into(), true);
        kb.learn(key(), RemedyId::new("remedy-b"), "verified".into(), false);
        // Counts stay attributed to the remedy that earned them.
        let entry = kb.entries().next().unwrap().1;
        assert_eq!(entry.remedy.as_str(), "remedy-a");
        assert_eq!((entry.successes, entry.failures), (1, 0));
        assert_eq!(
            entry.alt_remedies.get(&RemedyId::new("remedy-b")),
            Some(&(1, 1))
        );
        // ...and survive a save/load round trip.
        let mut reloaded = KnowledgeBase::new();
        reloaded.load_records(kb.to_records()).unwrap();
        let entry = reloaded.entries().next().unwrap().1;
        assert_eq!(entry.alt_remedies.len(), 1);
        assert_eq!(
            entry.alt_remedies.get(&RemedyId::new("remedy-b")),
            Some(&(1, 1))
        );
    }

    #[test]
    fn load_rejects_uncountable_max_counts() {
        // successes = u32::MAX can never be incremented by learn(); loading
        // it would either panic (debug) or wrap to 0 (release), corrupting
        // recommendation thresholds. Both primary and alternative counts are
        // rejected as bad records.
        let records = vec![
            KB_FORMAT_V2_HEADER.to_string(),
            "capacity/rate-limit|ctx|remedy-a|4294967295|0|maxed".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        let err = kb.load_records(records).unwrap_err();
        let KbError::BadRecord(detail) = err;
        assert!(detail.contains("record 2:"), "got {detail}");
        assert!(detail.contains("uncountable"), "got {detail}");

        let records = vec![
            KB_FORMAT_V2_HEADER.to_string(),
            "capacity/rate-limit|ctx|remedy-a|1|0|ok|remedy-b=4294967295/0".to_string(),
        ];
        let err = kb.load_records(records).unwrap_err();
        assert!(matches!(err, KbError::BadRecord(_)));
    }

    #[test]
    fn learn_saturates_instead_of_wrapping() {
        // A count at u32::MAX - 1 loads fine; learning twice saturates at
        // u32::MAX instead of wrapping to 0 in release (or panicking in
        // debug), which would silently flip a proven remedy into an
        // unproven one.
        let records = vec![
            KB_FORMAT_V2_HEADER.to_string(),
            "capacity/rate-limit|ctx|remedy-a|4294967294|0|near max".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        kb.load_records(records).unwrap();
        let key = KbKey {
            class: FailureClass::new("capacity/rate-limit"),
            context: ContextSig("ctx".into()),
        };
        kb.learn(key.clone(), RemedyId::new("remedy-a"), "near max".into(), true);
        kb.learn(key, RemedyId::new("remedy-a"), "near max".into(), true);
        let entry = kb.entries().next().unwrap().1;
        assert_eq!(entry.successes, u32::MAX);
        // Saturated, not wrapped: the remedy is still recommended.
        assert!(kb
            .recommend(
                &FailureClass::new("capacity/rate-limit"),
                &ContextSig("ctx".into())
            )
            .is_some());
    }

    #[test]
    fn v1_first_record_matching_the_v2_header_is_consumed_as_header() {
        // Documented ambiguity: a v1 bank whose first record is literally
        // "defail-kb v2" loses that record to the format header, and the
        // remaining lines are parsed as escaped v2.
        let v1 = vec![
            "defail-kb v2".to_string(),
            "capacity/rate-limit|ctx|remedy-a|1|0|first".to_string(),
        ];
        let mut kb = KnowledgeBase::new();
        let report = kb.load_records(v1).unwrap();
        assert_eq!(report.records, 1);
        let entry = kb.entries().next().unwrap().1;
        assert_eq!(entry.remedy.as_str(), "remedy-a");
    }

    #[test]
    fn absorb_keeps_the_stronger_outcome_for_the_same_remedy() {
        let mut bank = KnowledgeBase::new();
        bank.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        let mut other = KnowledgeBase::new();
        other.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        other.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), false);
        bank.absorb(&other);
        let entry = bank.entries().next().unwrap().1;
        // The strongest recorded outcome wins per remedy: (1,1) over (1,0).
        assert_eq!((entry.successes, entry.failures), (1, 1));
        // The join is idempotent: absorbing an already-merged bank is a no-op.
        let records = bank.to_records();
        bank.absorb(&other);
        assert_eq!(bank.to_records(), records);
    }

    #[test]
    fn absorb_merges_alternative_remedies() {
        let mut bank = KnowledgeBase::new();
        bank.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        bank.learn(key(), RemedyId::new("retry"), "accepted".into(), false);
        let mut other = KnowledgeBase::new();
        other.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        other.learn(key(), RemedyId::new("retry"), "accepted".into(), true);
        bank.absorb(&other);
        let entry = bank.entries().next().unwrap().1;
        assert_eq!((entry.successes, entry.failures), (1, 0));
        // retry's outcomes union component-wise: (0,1) joined with (1,0).
        assert_eq!(entry.alt_remedies.get(&RemedyId::new("retry")), Some(&(1, 1)));
    }

    #[test]
    fn absorb_keeps_the_better_remedy_on_conflict() {
        let mut bank = KnowledgeBase::new();
        bank.learn(key(), RemedyId::new("retry"), "accepted".into(), true);
        let mut other = KnowledgeBase::new();
        other.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        other.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        bank.absorb(&other);
        let entry = bank.entries().next().unwrap().1;
        assert_eq!(entry.remedy.as_str(), "fallback-b");
        assert_eq!((entry.successes, entry.failures), (2, 0));
        // The losing remedy is not discarded: it stays visible as an
        // alternative-remedy count.
        assert_eq!(entry.alt_remedies.get(&RemedyId::new("retry")), Some(&(1, 0)));
        // Merging in the opposite direction gives the identical result.
        let mut reverse = KnowledgeBase::new();
        reverse.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        reverse.learn(key(), RemedyId::new("fallback-b"), "accepted".into(), true);
        let mut retry_bank = KnowledgeBase::new();
        retry_bank.learn(key(), RemedyId::new("retry"), "accepted".into(), true);
        reverse.absorb(&retry_bank);
        assert_eq!(reverse.to_records(), bank.to_records());
    }
}
