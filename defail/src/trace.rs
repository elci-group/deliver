//! Structured event trace: every decision DEFAIL makes, emitted in order as a
//! plain enum.
//!
//! The engine records these events as it runs and attaches the events of each
//! resolution to the resulting [`FailureReport`](crate::report::FailureReport);
//! hosts can additionally attach a [`TraceSink`] to stream them anywhere.
//!
//! Security boundary (roadmap risk R2): events carry addresses, ids, and
//! decision data only — never signal payloads or host state. The evidence
//! text of a classification stays in the report; the trace only counts it.

use crate::address::OpAddress;
use crate::declare::{FailureClass, RemedyId};
use crate::json;

/// One decision in the resolution pipeline, in emission order.
///
/// The variants follow the pipeline exactly as [`DeFail::resolve`](crate::engine::DeFail::resolve)
/// runs it: `Classified` (or `EscalatedUnclassified`), then per candidate
/// remedy `RemedySelected` / `RemedyRejected`, `RemedyApplied`, `Resumed`,
/// `Verified`, `Learned`, and finally `EscalatedExhausted` when no candidate
/// survived. The gate contributes `GateDenied`; a construction-time invalid
/// declaration contributes `DeclarationSkipped`.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceEvent {
    /// Classification succeeded: the observation matched a declared failure
    /// mode at or above its minimum confidence.
    Classified {
        address: OpAddress,
        class: FailureClass,
        confidence: f32,
        /// How many evidence items the diagnosis carried; the items
        /// themselves are report data, not trace data (risk R2).
        evidence_count: usize,
    },
    /// No declared failure mode matched at its minimum confidence; the
    /// failure escalates as `unclassified` instead of being guessed at.
    EscalatedUnclassified {
        address: OpAddress,
        evidence_count: usize,
    },
    /// A remedy passed pre-execution policy validation and is about to be
    /// attempted.
    RemedySelected {
        address: OpAddress,
        remedy: RemedyId,
        source: RemedySource,
    },
    /// A remedy was rejected: by pre-execution policy, by the host, by a
    /// failed resume, by post-execution policy, or by the declared
    /// verification predicate.
    RemedyRejected {
        address: OpAddress,
        remedy: RemedyId,
        stage: RejectStage,
    },
    /// The host accepted the remedy; one attempt has been consumed.
    RemedyApplied {
        address: OpAddress,
        remedy: RemedyId,
    },
    /// The operation was re-executed after the remedy; `ok` is whether it
    /// completed.
    Resumed {
        address: OpAddress,
        ok: bool,
    },
    /// The declared verification predicate was evaluated in the resumed
    /// state.
    Verified {
        address: OpAddress,
        remedy: RemedyId,
        ok: bool,
    },
    /// The outcome of an attempt was recorded in the knowledge base.
    Learned {
        address: OpAddress,
        class: FailureClass,
        remedy: RemedyId,
        positive: bool,
    },
    /// Every candidate was rejected or the attempt cap was reached; the
    /// failure escalates.
    EscalatedExhausted {
        address: OpAddress,
        attempts: u32,
        rejected: usize,
    },
    /// The gate refused an operation. `reason` is the rendered
    /// [`GateReason`](crate::enforce::GateReason) — decision data, not host
    /// payloads.
    GateDenied { address: OpAddress, reason: String },
    /// A failure mode was excluded from classification because its
    /// declaration is invalid (emitted by [`DeFail::new`](crate::engine::DeFail::new)
    /// when a hand-built spec fails validation; the mode can never match, so
    /// observations that would have matched it escalate as `unclassified`).
    DeclarationSkipped { mode: String, reason: String },
}

/// Where a selected remedy came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemedySource {
    /// The knowledge base recommended it (it demonstrably worked here before).
    Knowledge,
    /// Declared precedence order on the component spec.
    Declared,
}

/// At which stage a remedy was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectStage {
    /// Pre-execution constraint validation failed (e.g. the substitution
    /// window had expired).
    PolicyPre,
    /// The host refused the remedy or it failed to apply.
    ApplyFailed,
    /// The operation did not resume after the remedy.
    ResumeFailed,
    /// A post-execution capability invariant failed.
    PolicyPost,
    /// The declared verification predicate did not hold.
    Verification,
}

impl TraceEvent {
    /// Stable snake-case discriminator used as the `"event"` field in JSON.
    pub fn kind(&self) -> &'static str {
        match self {
            TraceEvent::Classified { .. } => "classified",
            TraceEvent::EscalatedUnclassified { .. } => "escalated_unclassified",
            TraceEvent::RemedySelected { .. } => "remedy_selected",
            TraceEvent::RemedyRejected { .. } => "remedy_rejected",
            TraceEvent::RemedyApplied { .. } => "remedy_applied",
            TraceEvent::Resumed { .. } => "resumed",
            TraceEvent::Verified { .. } => "verified",
            TraceEvent::Learned { .. } => "learned",
            TraceEvent::EscalatedExhausted { .. } => "escalated_exhausted",
            TraceEvent::GateDenied { .. } => "gate_denied",
            TraceEvent::DeclarationSkipped { .. } => "declaration_skipped",
        }
    }

    /// Render this event as a JSON object (see [`crate::json`]).
    pub fn to_json(&self) -> String {
        let address = |a: &OpAddress| ("address", json::quote(&a.to_string()));
        let remedy = |r: &RemedyId| ("remedy", json::quote(r.as_str()));
        let mut fields = vec![("event", json::quote(self.kind()))];
        match self {
            TraceEvent::Classified {
                address: a,
                class,
                confidence,
                evidence_count,
            } => {
                fields.push(address(a));
                fields.push(("class", json::quote(class.as_str())));
                fields.push(("confidence", json::f32(*confidence)));
                fields.push(("evidence_count", evidence_count.to_string()));
            }
            TraceEvent::EscalatedUnclassified {
                address: a,
                evidence_count,
            } => {
                fields.push(address(a));
                fields.push(("evidence_count", evidence_count.to_string()));
            }
            TraceEvent::RemedySelected {
                address: a,
                remedy: r,
                source,
            } => {
                fields.push(address(a));
                fields.push(remedy(r));
                fields.push((
                    "source",
                    json::quote(match source {
                        RemedySource::Knowledge => "knowledge",
                        RemedySource::Declared => "declared",
                    }),
                ));
            }
            TraceEvent::RemedyRejected {
                address: a,
                remedy: r,
                stage,
            } => {
                fields.push(address(a));
                fields.push(remedy(r));
                fields.push((
                    "stage",
                    json::quote(match stage {
                        RejectStage::PolicyPre => "policy_pre",
                        RejectStage::ApplyFailed => "apply_failed",
                        RejectStage::ResumeFailed => "resume_failed",
                        RejectStage::PolicyPost => "policy_post",
                        RejectStage::Verification => "verification",
                    }),
                ));
            }
            TraceEvent::RemedyApplied { address: a, remedy: r } => {
                fields.push(address(a));
                fields.push(remedy(r));
            }
            TraceEvent::Resumed { address: a, ok } => {
                fields.push(address(a));
                fields.push(("ok", ok.to_string()));
            }
            TraceEvent::Verified {
                address: a,
                remedy: r,
                ok,
            } => {
                fields.push(address(a));
                fields.push(remedy(r));
                fields.push(("ok", ok.to_string()));
            }
            TraceEvent::Learned {
                address: a,
                class,
                remedy: r,
                positive,
            } => {
                fields.push(address(a));
                fields.push(("class", json::quote(class.as_str())));
                fields.push(remedy(r));
                fields.push(("positive", positive.to_string()));
            }
            TraceEvent::EscalatedExhausted {
                address: a,
                attempts,
                rejected,
            } => {
                fields.push(address(a));
                fields.push(("attempts", attempts.to_string()));
                fields.push(("rejected", rejected.to_string()));
            }
            TraceEvent::GateDenied { address: a, reason } => {
                fields.push(address(a));
                fields.push(("reason", json::quote(reason)));
            }
            TraceEvent::DeclarationSkipped { mode, reason } => {
                fields.push(("mode", json::quote(mode)));
                fields.push(("reason", json::quote(reason)));
            }
        }
        json::object(&fields)
    }
}

/// Receiver of trace events. Implement this to stream decisions into your own
/// logging or telemetry; the provided sinks cover the common cases.
pub trait TraceSink {
    fn on_event(&mut self, event: &TraceEvent);
}

/// The default sink: discards every event. [`DeFail`](crate::engine::DeFail)
/// uses this unless a host attaches one with
/// [`DeFail::with_sink`](crate::engine::DeFail::with_sink).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl TraceSink for NoopSink {
    fn on_event(&mut self, _event: &TraceEvent) {}
}

/// Records every event in order — the same record the engine keeps internally
/// and [`FailureReport`](crate::report::FailureReport) carries per resolution.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct VecSink {
    events: Vec<TraceEvent>,
}

impl VecSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
}

impl TraceSink for VecSink {
    fn on_event(&mut self, event: &TraceEvent) {
        self.events.push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> OpAddress {
        OpAddress::new(s).unwrap()
    }

    #[test]
    fn vec_sink_records_every_event_in_order() {
        let mut sink = VecSink::new();
        let events = vec![
            TraceEvent::Classified {
                address: addr("app.bake"),
                class: FailureClass::new("resource/ingredient-missing"),
                confidence: 1.0,
                evidence_count: 1,
            },
            TraceEvent::GateDenied {
                address: addr("app.serve"),
                reason: "blocked by unresolved failure at app.bake".into(),
            },
        ];
        for event in &events {
            sink.on_event(event);
        }
        assert_eq!(sink.events(), events.as_slice());
        NoopSink.on_event(&events[0]);
    }

    #[test]
    fn json_discriminator_and_fields_are_stable() {
        let event = TraceEvent::RemedyRejected {
            address: addr("app.bake"),
            remedy: RemedyId::new("substitute-yeast"),
            stage: RejectStage::PolicyPre,
        };
        assert_eq!(
            event.to_json(),
            "{\"event\":\"remedy_rejected\",\"address\":\"app.bake\",\
             \"remedy\":\"substitute-yeast\",\"stage\":\"policy_pre\"}"
        );
        let event = TraceEvent::Learned {
            address: addr("app.bake"),
            class: FailureClass::new("resource/ingredient-missing"),
            remedy: RemedyId::new("constrained-proceed"),
            positive: true,
        };
        assert_eq!(
            event.to_json(),
            "{\"event\":\"learned\",\"address\":\"app.bake\",\
             \"class\":\"resource/ingredient-missing\",\"remedy\":\"constrained-proceed\",\
             \"positive\":true}"
        );
    }

    #[test]
    fn json_never_leaks_unescaped_strings() {
        let event = TraceEvent::GateDenied {
            address: addr("app.bake"),
            reason: "prerequisites incomplete: \"a\", b\nc".into(),
        };
        let rendered = event.to_json();
        assert!(!rendered.contains('\n'));
        assert!(rendered.contains("\\\"a\\\""));
        assert_eq!(TraceEvent::kind(&event), "gate_denied");
    }
}
