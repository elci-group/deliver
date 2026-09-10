//! Property tests over generated inputs, driven by a std-only harness.
//!
//! Every generator draws from a deterministic xorshift PRNG with a fixed
//! seed, documented next to each test: a failure reproduces exactly by
//! re-running the test, and no wall clock, randomness source, or thread is
//! involved. The existing 100× end-to-end determinism property lives in
//! `tests/end_to_end.rs` and is deliberately not duplicated here.

use std::collections::BTreeSet;

use defail::address::OpAddress;
use defail::declare::{ComponentSpec, FailureClass, FailureMode, RemedyId, Verification};
use defail::inference::classify;
use defail::knowledge::{ContextSig, KbKey, KnowledgeBase};
use defail::signal::{Observation, Signal, SignalPattern, SignalValue};

/// Deterministic xorshift64* PRNG. Seeds are fixed per test so any failing
/// case reproduces byte-for-byte.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // An all-zero state would make xorshift degenerate forever.
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Fragments that stress the record escaping: every separator and escape
/// introducer, control characters, quotes, unicode, the empty string, and
/// text that mimics the format header itself.
const FRAGMENTS: &[&str] = &[
    "|", "\\", "\n", "\r", "\t", "\"", "'", "=", ",", "/", "defail-kb v2", "defail-report/1",
    "class|context", "a=b,c", "", "plain", "üñïçødé", "中文", "remedy-7", "successes", "0", " ",
    "..", "record ", ":", "{", "}",
];

fn gen_string(rng: &mut Rng, max_fragments: u64) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(max_fragments + 1) {
        out.push_str(rng.pick(FRAGMENTS));
    }
    out
}

/// A bank of 1–6 keys, each learned under 1–3 remedies with 1–4 outcomes
/// each, so entries carry both primary and alternative-remedy counts.
fn gen_bank(rng: &mut Rng) -> KnowledgeBase {
    let mut kb = KnowledgeBase::new();
    for _ in 0..rng.below(6) + 1 {
        let key = KbKey {
            class: FailureClass::new(gen_string(&mut *rng, 4)),
            context: ContextSig(gen_string(&mut *rng, 4)),
        };
        for _ in 0..rng.below(3) + 1 {
            let remedy = RemedyId::new(gen_string(&mut *rng, 3));
            let desc = gen_string(&mut *rng, 3);
            for _ in 0..rng.below(4) + 1 {
                kb.learn(key.clone(), remedy.clone(), desc.clone(), rng.below(2) == 0);
            }
        }
    }
    kb
}

#[test]
fn absorb_is_commutative_associative_and_idempotent() {
    // Seed 0xA850_2211: fixed, so a violating case reproduces exactly.
    let mut rng = Rng::new(0xA850_2211);
    for case in 0..200 {
        let a = gen_bank(&mut rng);
        let b = gen_bank(&mut rng);

        // Commutativity: A ⊕ B == B ⊕ A.
        let mut ab = a.clone();
        ab.absorb(&b);
        let mut ba = b.clone();
        ba.absorb(&a);
        assert_eq!(ab, ba, "absorb is not commutative (case {case})");

        // Idempotence: A ⊕ A == A.
        let mut aa = a.clone();
        aa.absorb(&a);
        assert_eq!(aa, a, "absorb is not idempotent (case {case})");

        // Associativity: (A ⊕ B) ⊕ C == A ⊕ (B ⊕ C).
        let c = gen_bank(&mut rng);
        let mut ab_c = ab.clone();
        ab_c.absorb(&c);
        let mut bc = b.clone();
        bc.absorb(&c);
        let mut a_bc = a.clone();
        a_bc.absorb(&bc);
        assert_eq!(ab_c, a_bc, "absorb is not associative (case {case})");
    }
}

#[test]
fn adversarial_records_round_trip_exactly() {
    // Seed 0xB360_3322: fixed, so a failing record reproduces exactly.
    let mut rng = Rng::new(0xB360_3322);
    for case in 0..200 {
        let mut kb = KnowledgeBase::new();
        let mut seen: BTreeSet<KbKey> = BTreeSet::new();
        for _ in 0..rng.below(8) + 1 {
            let key = KbKey {
                class: FailureClass::new(gen_string(&mut rng, 4)),
                context: ContextSig(gen_string(&mut rng, 4)),
            };
            let distinct = seen.insert(key.clone());
            for _ in 0..rng.below(3) + 1 {
                let remedy = RemedyId::new(gen_string(&mut rng, 3));
                let desc = gen_string(&mut rng, 3);
                for _ in 0..rng.below(4) + 1 {
                    kb.learn(key.clone(), remedy.clone(), desc.clone(), rng.below(2) == 0);
                }
            }
            if distinct {
                let records = kb.to_records();
                let mut loaded = KnowledgeBase::new();
                let report = loaded.load_records(records.clone()).unwrap();
                assert_eq!(loaded, kb, "round trip changed the bank (case {case})");
                assert_eq!(
                    loaded.to_records(),
                    records,
                    "records are not byte-stable (case {case})"
                );
                assert_eq!(
                    report.records,
                    records.len() - 1,
                    "record count mismatch (case {case})"
                );
                assert!(
                    report.duplicates.is_empty(),
                    "distinct generated keys reported as duplicates (case {case})"
                );
            }
        }
        // The fully-grown bank (with generated duplicates folded in by
        // last-wins) must round-trip exactly as well.
        let records = kb.to_records();
        let mut loaded = KnowledgeBase::new();
        loaded.load_records(records.clone()).unwrap();
        assert_eq!(loaded, kb, "final bank round trip changed it (case {case})");
        assert_eq!(loaded.to_records(), records, "final records unstable (case {case})");
    }
}

/// Three failure modes with overlapping signal patterns, so classification
/// has to rank candidates and exercise the tie-breaking rules.
fn multi_mode_spec() -> ComponentSpec {
    let mode = |id: &str,
                class: &str,
                summary: &str,
                patterns: Vec<SignalPattern>,
                min_confidence: f32| FailureMode {
        id: id.into(),
        class: FailureClass::new(class),
        summary: summary.into(),
        patterns,
        permitted: vec![RemedyId::new("recover")],
        verification: Verification::CapabilityAtLeast {
            capability: "generation".into(),
            tier: 1,
        },
        verification_desc: "recovered".into(),
        min_confidence,
    };
    ComponentSpec {
        name: "Client".into(),
        capabilities: vec![],
        failure_modes: vec![
            mode(
                "rate-limited",
                "capacity/rate-limit",
                "provider request rejected",
                vec![
                    SignalPattern::exact("http_status", SignalValue::Int(429), 0.6),
                    SignalPattern::any("quota_header", 0.38),
                    SignalPattern::any("provider_outage", 0.02),
                ],
                0.9,
            ),
            mode(
                "credential-expired",
                "auth/credential-expired",
                "credential rejected",
                vec![
                    SignalPattern::exact("http_status", SignalValue::Int(401), 0.7),
                    SignalPattern::any("token_header", 0.3),
                ],
                0.8,
            ),
            mode(
                "degraded",
                "capacity/degraded",
                "provider degraded",
                vec![
                    SignalPattern::exact("http_status", SignalValue::Int(429), 0.5),
                    SignalPattern::any("latency", 0.5),
                ],
                0.7,
            ),
        ],
        remedies: vec![],
    }
    .validated()
    .expect("test declaration must pass validation")
}

#[test]
fn classification_is_independent_of_signal_order() {
    // Seed 0xC470_4433: fixed, so a failing shuffle reproduces exactly.
    let spec = multi_mode_spec();
    let signals = vec![
        Signal::new("http_status", SignalValue::Int(429)),
        Signal::new("quota_header", SignalValue::text("present")),
        Signal::new("provider_outage", SignalValue::Bool(false)),
        Signal::new("latency", SignalValue::text("high")),
        Signal::new("token_header", SignalValue::text("present")),
    ];
    let at = OpAddress::new("Client.call").unwrap();
    let baseline = classify(
        &spec,
        &Observation::new(at.clone(), "provider request rejected", signals.clone()),
    )
    .unwrap();
    // Two modes reach full confidence (rate-limit at 1.0 with three evidence
    // items, degraded at 1.0 with two); the richer evidence must win.
    assert_eq!(baseline.class.as_str(), "capacity/rate-limit");
    assert_eq!(baseline.confidence, 1.0);

    let mut rng = Rng::new(0xC470_4433);
    for round in 0..100 {
        let mut shuffled = signals.clone();
        // Deterministic Fisher–Yates shuffle from the seeded PRNG.
        for i in (1..shuffled.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            shuffled.swap(i, j);
        }
        let dx = classify(
            &spec,
            &Observation::new(at.clone(), "provider request rejected", shuffled),
        )
        .unwrap();
        assert_eq!(
            dx, baseline,
            "signal insertion order changed the diagnosis (round {round})"
        );
    }
}
