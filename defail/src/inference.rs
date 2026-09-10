//! Causal inference: classify an observation against declared failure modes.
//!
//! Deterministic by construction: candidates are ranked by confidence, then
//! evidence count, then summary, so the same observation always yields the
//! same diagnosis. No inference here has authority to invent a fix — it can
//! only select among declared failure modes.

use std::cmp::Ordering;

use crate::declare::{ComponentSpec, FailureClass};
use crate::signal::Observation;

/// A classified cause: a deterministic match of the observation against one
/// declared failure mode, with confidence and the evidence that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    pub mode_index: usize,
    pub class: FailureClass,
    pub summary: String,
    pub confidence: f32,
    pub evidence: Vec<String>,
}

/// Classify an observation against a component's declared failure modes.
/// Returns `None` when no mode matches at its declared minimum confidence —
/// the caller must then escalate rather than guess.
pub fn classify(spec: &ComponentSpec, obs: &Observation) -> Option<Diagnosis> {
    let mut candidates: Vec<Diagnosis> = Vec::new();

    for (i, mode) in spec.failure_modes.iter().enumerate() {
        // Defensive: declarations are validated at construction time, but a
        // hand-built spec must never turn an invalid minimum confidence into
        // "match everything" (a negative or NaN threshold makes
        // `confidence < min_confidence` never true). Such a mode is treated
        // as unmatchable: the observation escalates as unclassified.
        if !mode.min_confidence.is_finite() || !(0.0..=1.0).contains(&mode.min_confidence) {
            continue;
        }
        // Defensive: declarations are validated at construction time, but a
        // hand-built spec must never turn an invalid weight into a NaN
        // confidence. Invalid weights are ignored entirely.
        let valid_weight = |w: f32| w.is_finite() && w > 0.0;
        let total: f32 = mode
            .patterns
            .iter()
            .filter(|p| valid_weight(p.weight))
            .map(|p| p.weight)
            .sum();
        match total.partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => {}
            _ => continue,
        }
        let mut matched_weight = 0.0f32;
        let mut evidence = Vec::new();
        for pattern in &mode.patterns {
            if !valid_weight(pattern.weight) {
                continue;
            }
            if pattern.matches(obs) {
                matched_weight += pattern.weight;
                match obs.signal(&pattern.key) {
                    Some(signal) => evidence.push(signal.to_string()),
                    None => evidence.push(format!("{}=absent", pattern.key)),
                }
            }
        }
        if matched_weight <= 0.0 {
            continue;
        }
        let confidence = (matched_weight / total).clamp(0.0, 1.0);
        if confidence < mode.min_confidence {
            continue;
        }
        candidates.push(Diagnosis {
            mode_index: i,
            class: mode.class.clone(),
            summary: mode.summary.clone(),
            confidence,
            evidence,
        });
    }

    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.evidence.len().cmp(&a.evidence.len()))
            .then_with(|| a.summary.cmp(&b.summary))
    });
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declare::{FailureMode, RemedyId, Verification};
    use crate::signal::{Signal, SignalPattern, SignalValue};

    fn addr(s: &str) -> crate::address::OpAddress {
        crate::address::OpAddress::new(s).unwrap()
    }

    fn spec() -> ComponentSpec {
        ComponentSpec {
            name: "T".into(),
            capabilities: vec![],
            failure_modes: vec![FailureMode {
                id: "rate-limited".into(),
                class: FailureClass::new("capacity/rate-limit"),
                summary: "provider request rejected".into(),
                patterns: vec![
                    SignalPattern::exact("http_status", SignalValue::Int(429), 0.6),
                    SignalPattern::any("quota_header", 0.38),
                    SignalPattern::any("provider_outage", 0.02),
                ],
                permitted: vec![RemedyId::new("fallback")],
                verification: Verification::CapabilityAtLeast {
                    capability: "generation".into(),
                    tier: 3,
                },
                verification_desc: "equivalent request accepted".into(),
                min_confidence: 0.9,
            }],
            remedies: vec![],
        }
    }

    #[test]
    fn full_evidence_yields_high_confidence() {
        let obs = Observation::new(
            addr("T.call"),
            "provider request rejected",
            vec![
                Signal::new("http_status", SignalValue::Int(429)),
                Signal::new("quota_header", SignalValue::text("present")),
            ],
        );
        let dx = classify(&spec(), &obs).unwrap();
        assert_eq!(dx.class.as_str(), "capacity/rate-limit");
        assert_eq!(dx.confidence, 0.98);
        assert_eq!(dx.evidence, vec!["http_status=429", "quota_header=present"]);
    }

    #[test]
    fn weak_evidence_below_threshold_is_unclassified() {
        let obs = Observation::new(
            addr("T.call"),
            "odd",
            vec![Signal::new("http_status", SignalValue::Int(429))],
        );
        // 0.6 < 0.9 declared minimum: refuse to diagnose.
        assert!(classify(&spec(), &obs).is_none());
    }

    #[test]
    fn invalid_weights_are_skipped_defensively() {
        let mut bad = spec();
        // NaN weights must not poison the confidence of a valid mode, and a
        // mode whose weights are all invalid must never match. Keep the
        // quota_header pattern (0.38) valid: it is present in the
        // observation, so confidence is 0.38/0.38 = 1.0.
        bad.failure_modes[0].patterns[0].weight = f32::NAN;
        bad.failure_modes[0].patterns[2].weight = f32::NAN;
        let obs = Observation::new(
            addr("T.call"),
            "provider request rejected",
            vec![
                Signal::new("http_status", SignalValue::Int(429)),
                Signal::new("quota_header", SignalValue::text("present")),
            ],
        );
        // Only the 0.38 quota_header pattern remains valid: total > 0, and
        // it matches, so confidence is 1.0.
        let dx = classify(&bad, &obs).unwrap();
        assert_eq!(dx.confidence, 1.0);

        let mut all_bad = spec();
        for pattern in &mut all_bad.failure_modes[0].patterns {
            pattern.weight = f32::NAN;
        }
        assert!(classify(&all_bad, &obs).is_none());

        let mut negative = spec();
        for pattern in &mut negative.failure_modes[0].patterns {
            pattern.weight = -1.0;
        }
        assert!(classify(&negative, &obs).is_none());
    }

    #[test]
    fn invalid_min_confidence_modes_never_match() {
        // A hand-built spec can declare a min_confidence outside [0, 1]
        // (negative or NaN); `confidence < min_confidence` would then never
        // be true and every observation would "match" at confidence 1.0.
        // Such modes must be treated as unmatchable instead.
        let obs = Observation::new(
            addr("T.call"),
            "provider request rejected",
            vec![
                Signal::new("http_status", SignalValue::Int(429)),
                Signal::new("quota_header", SignalValue::text("present")),
            ],
        );
        for bad_threshold in [-5.0, -f32::EPSILON, f32::NAN, f32::NEG_INFINITY, 1.5] {
            let mut bad = spec();
            bad.failure_modes[0].min_confidence = bad_threshold;
            assert!(
                classify(&bad, &obs).is_none(),
                "min_confidence {bad_threshold} must not match"
            );
        }
        // In-range thresholds still work.
        let mut ok = spec();
        ok.failure_modes[0].min_confidence = 0.98;
        assert!(classify(&ok, &obs).is_some());
    }
}
