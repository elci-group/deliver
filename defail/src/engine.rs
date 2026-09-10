//! The DEFAIL engine: execution under a deterministic failure-resolution
//! pipeline.
//!
//! ```text
//! observed failure → inference → candidate remediation
//!                  → policy / capability constraints → deterministic action
//!                  → verification → known state (resume or escalate)
//! ```

use std::fmt;

use crate::address::OpAddress;
use crate::declare::{ComponentSpec, FailureClass, RemedyId, RemedySpec, Verification};
use crate::inference;
use crate::knowledge::{ContextSig, KbKey, KnowledgeBase};
use crate::policy::Policy;
use crate::report::{Disposition, FailureReport};
use crate::signal::{Observation, Signal};
use crate::state::{ExecutionState, OpStatus};
use crate::trace::{NoopSink, RemedySource, RejectStage, TraceEvent, TraceSink};

/// Why the host refused or failed to apply a remedy.
///
/// This is the typed error boundary between DEFAIL and the host application:
/// a remedy that cannot be applied is reported with a reason, never a bare
/// string. Convert from `String`/`&str` with `.into()` when implementing
/// [`AppWorld`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemedyError {
    /// The host refused the remedy, or applying it failed.
    Rejected(String),
}

impl fmt::Display for RemedyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemedyError::Rejected(reason) => {
                write!(f, "remedy refused or failed to apply: {reason}")
            }
        }
    }
}

impl std::error::Error for RemedyError {}

impl From<String> for RemedyError {
    fn from(reason: String) -> Self {
        RemedyError::Rejected(reason)
    }
}

impl From<&str> for RemedyError {
    fn from(reason: &str) -> Self {
        RemedyError::Rejected(reason.to_string())
    }
}

/// The host application. Execution and remediation are application-specific;
/// DEFAIL drives them deterministically through this boundary — that is the
/// "embedded" in DEFAIL.
pub trait AppWorld {
    /// Attempt one plan step. `Err(observation)` reports a detected failure
    /// with the diagnostic signals the application can see.
    fn execute(
        &mut self,
        address: &OpAddress,
        state: &mut ExecutionState,
    ) -> Result<(), Observation>;

    /// Apply a permitted remediation, updating state. `Err` means the
    /// remedy was refused or failed to apply.
    fn apply_remedy(
        &mut self,
        remedy: &RemedySpec,
        state: &mut ExecutionState,
    ) -> Result<(), RemedyError>;

    /// Diagnostic signals from the most recent `execute` call, used to
    /// evaluate signal predicates.
    fn recent_signals(&self) -> &[Signal];

    /// Evaluate a declared verification predicate. The provided
    /// implementation is deterministic: state predicates consult the
    /// execution state, signal predicates consult [`AppWorld::recent_signals`].
    fn verify(&mut self, verification: &Verification, state: &ExecutionState) -> bool {
        eval_verification(verification, state, self.recent_signals())
    }
}

/// Deterministic evaluation of a verification predicate.
pub fn eval_verification(
    verification: &Verification,
    state: &ExecutionState,
    signals: &[Signal],
) -> bool {
    match verification {
        Verification::SignalEquals { key, value } => signals
            .iter()
            .any(|s| &s.key == key && s.value.matches(value)),
        Verification::CapabilityAtLeast { capability, tier } => {
            state.capability_tier(capability) >= *tier
        }
        Verification::StateReached { address } => state.reached(address),
        Verification::All(vs) => vs.iter().all(|v| eval_verification(v, state, signals)),
        Verification::Any(vs) => vs.iter().any(|v| eval_verification(v, state, signals)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepOutcome {
    /// The step succeeded without needing remediation.
    Completed,
    /// The step failed, a remediation satisfied policy and verification,
    /// and the step then resumed successfully.
    Recovered(Box<FailureReport>),
    /// The step failed and no permitted remediation resolved it.
    Escalated(Box<FailureReport>),
}

pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

/// The DEFAIL orchestrator.
pub struct DeFail {
    spec: ComponentSpec,
    kb: KnowledgeBase,
    max_attempts: u32,
    /// Every decision recorded since construction, in order. Each
    /// [`FailureReport`] carries the slice emitted during its resolution.
    events: Vec<TraceEvent>,
    /// Optional host-attached sink; events are forwarded as they happen.
    sink: Box<dyn TraceSink>,
}

impl DeFail {
    pub fn new(spec: ComponentSpec) -> Self {
        Self {
            spec,
            kb: KnowledgeBase::new(),
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            events: Vec::new(),
            sink: Box::new(NoopSink),
        }
    }

    /// Attach a [`TraceSink`] to receive every decision as it happens. The
    /// engine records events internally regardless, so existing constructors
    /// keep working without a sink.
    pub fn with_sink(mut self, sink: impl TraceSink + 'static) -> Self {
        self.sink = Box::new(sink);
        self
    }

    /// Every event recorded since construction, in emission order.
    pub fn trace(&self) -> &[TraceEvent] {
        &self.events
    }

    /// Record an event: appended to the engine trace and forwarded to the
    /// attached sink. The gate uses this to log denials; hosts may log their
    /// own decisions alongside DEFAIL's.
    pub fn emit_event(&mut self, event: TraceEvent) {
        self.events.push(event.clone());
        self.sink.on_event(&event);
    }

    /// Carry knowledge from a previous run.
    pub fn with_knowledge(mut self, kb: KnowledgeBase) -> Self {
        self.kb = kb;
        self
    }

    pub fn knowledge(&self) -> &KnowledgeBase {
        &self.kb
    }

    pub fn knowledge_mut(&mut self) -> &mut KnowledgeBase {
        &mut self.kb
    }

    pub fn spec(&self) -> &ComponentSpec {
        &self.spec
    }

    /// Cap remediation attempts per failure (at least 1).
    pub fn set_max_attempts(&mut self, n: u32) {
        self.max_attempts = n.max(1);
    }

    /// Seed the component's declared nominal capabilities into fresh state.
    pub fn seed_capabilities(&self, state: &mut ExecutionState) {
        for (capability, tier) in &self.spec.capabilities {
            state.set_capability(capability, *tier);
        }
    }

    /// Run one step under DEFAIL: execute, and on failure run the resolution
    /// pipeline. A recovered step is marked Done; an escalated step stays
    /// Failed so downstream gates can see it.
    pub fn run_step(
        &mut self,
        world: &mut dyn AppWorld,
        state: &mut ExecutionState,
        address: &OpAddress,
    ) -> StepOutcome {
        state.mark(address, OpStatus::InProgress);
        match world.execute(address, state) {
            Ok(()) => {
                state.mark(address, OpStatus::Done);
                StepOutcome::Completed
            }
            Err(obs) => {
                state.mark(&obs.at, OpStatus::Failed);
                let report = self.resolve(world, state, obs);
                match report.disposition {
                    Disposition::Recovered => {
                        state.mark(address, OpStatus::Done);
                        StepOutcome::Recovered(Box::new(report))
                    }
                    Disposition::Escalated => StepOutcome::Escalated(Box::new(report)),
                }
            }
        }
    }

    /// The resolution pipeline for an observed failure:
    /// classify → select (knowledge first, then declared precedence) →
    /// policy-validate → apply → resume → verify → learn → report.
    pub fn resolve(
        &mut self,
        world: &mut dyn AppWorld,
        state: &mut ExecutionState,
        obs: Observation,
    ) -> FailureReport {
        let at = obs.at.clone();
        let trace_start = self.events.len();
        let Some(dx) = inference::classify(&self.spec, &obs) else {
            let evidence_count = obs.signals.len();
            self.emit_event(TraceEvent::EscalatedUnclassified {
                address: at.clone(),
                evidence_count,
            });
            return FailureReport {
                failure: obs.summary.clone(),
                class: FailureClass::new("unclassified"),
                evidence: obs.signals.iter().map(|s| s.to_string()).collect(),
                confidence: 0.0,
                resolution: "escalated: no declared failure mode matched the observation".into(),
                constraint: "inference refused to diagnose below declared confidence".into(),
                verification: "not attempted".into(),
                disposition: Disposition::Escalated,
                attempts: 0,
                rejected: Vec::new(),
                trace: self.events[trace_start..].to_vec(),
            };
        };
        self.emit_event(TraceEvent::Classified {
            address: at.clone(),
            class: dx.class.clone(),
            confidence: dx.confidence,
            evidence_count: dx.evidence.len(),
        });

        let mode = self.spec.failure_modes[dx.mode_index].clone();
        let context = ContextSig(state.context_signature(&at));
        let mut attempts = 0u32;
        let mut rejected: Vec<String> = Vec::new();

        // Deterministic selection order: proven knowledge first, then declared
        // precedence, then id. Never anything else.
        let mut order: Vec<(RemedyId, RemedySource)> = Vec::new();
        if let Some(entry) = self.kb.recommend(&dx.class, &context) {
            if mode.permitted.contains(&entry.remedy) {
                order.push((entry.remedy.clone(), RemedySource::Knowledge));
            }
        }
        let mut declared: Vec<&RemedySpec> = self
            .spec
            .remedies
            .iter()
            .filter(|r| mode.permitted.contains(&r.id))
            .collect();
        declared.sort_by(|a, b| {
            a.precedence
                .cmp(&b.precedence)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        for remedy in declared {
            if !order.iter().any(|(id, _)| id == &remedy.id) {
                order.push((remedy.id.clone(), RemedySource::Declared));
            }
        }

        for (remedy_id, source) in &order {
            if attempts >= self.max_attempts {
                break;
            }
            let Some(spec) = self.spec.remedy(remedy_id) else {
                continue;
            };
            let remedy = spec.clone();
            let kb_key = KbKey {
                class: dx.class.clone(),
                context: context.clone(),
            };

            // 1. Deterministic policy validation, pre-execution.
            if let Err(violation) = Policy::validate_pre(&remedy.constraints, state) {
                self.emit_event(TraceEvent::RemedyRejected {
                    address: at.clone(),
                    remedy: remedy.id.clone(),
                    stage: RejectStage::PolicyPre,
                });
                rejected.push(format!("{} ({})", remedy.description, violation.reason));
                continue;
            }
            self.emit_event(TraceEvent::RemedySelected {
                address: at.clone(),
                remedy: remedy.id.clone(),
                source: *source,
            });

            // 2. Deterministic action.
            attempts += 1;
            if let Err(reason) = world.apply_remedy(&remedy, state) {
                self.emit_event(TraceEvent::Learned {
                    address: at.clone(),
                    class: dx.class.clone(),
                    remedy: remedy.id.clone(),
                    positive: false,
                });
                self.emit_event(TraceEvent::RemedyRejected {
                    address: at.clone(),
                    remedy: remedy.id.clone(),
                    stage: RejectStage::ApplyFailed,
                });
                self.kb.learn(
                    kb_key,
                    remedy.id.clone(),
                    mode.verification_desc.clone(),
                    false,
                );
                rejected.push(format!("{} ({reason})", remedy.description));
                continue;
            }
            self.emit_event(TraceEvent::RemedyApplied {
                address: at.clone(),
                remedy: remedy.id.clone(),
            });

            // 3. Resume: the operation itself must now succeed.
            let resumed = world.execute(&at, state);
            self.emit_event(TraceEvent::Resumed {
                address: at.clone(),
                ok: resumed.is_ok(),
            });
            if let Err(resumed_obs) = resumed {
                state.mark(&at, OpStatus::Failed);
                self.emit_event(TraceEvent::Learned {
                    address: at.clone(),
                    class: dx.class.clone(),
                    remedy: remedy.id.clone(),
                    positive: false,
                });
                self.emit_event(TraceEvent::RemedyRejected {
                    address: at.clone(),
                    remedy: remedy.id.clone(),
                    stage: RejectStage::ResumeFailed,
                });
                self.kb.learn(
                    kb_key,
                    remedy.id.clone(),
                    mode.verification_desc.clone(),
                    false,
                );
                let reason = if resumed_obs.signals.is_empty() {
                    "remediation did not unblock the operation".to_string()
                } else {
                    resumed_obs.summary
                };
                rejected.push(format!("{} ({reason})", remedy.description));
                continue;
            }
            state.mark(&at, OpStatus::Done);

            // 4. Capability invariants must hold in the resumed state.
            if let Err(violation) = Policy::validate_post(&remedy.constraints, state) {
                state.mark(&at, OpStatus::Failed);
                self.emit_event(TraceEvent::Learned {
                    address: at.clone(),
                    class: dx.class.clone(),
                    remedy: remedy.id.clone(),
                    positive: false,
                });
                self.emit_event(TraceEvent::RemedyRejected {
                    address: at.clone(),
                    remedy: remedy.id.clone(),
                    stage: RejectStage::PolicyPost,
                });
                self.kb.learn(
                    kb_key,
                    remedy.id.clone(),
                    mode.verification_desc.clone(),
                    false,
                );
                rejected.push(format!("{} ({})", remedy.description, violation.reason));
                continue;
            }

            // 5. Declared verification predicate must hold.
            let verified = world.verify(&mode.verification, state);
            self.emit_event(TraceEvent::Verified {
                address: at.clone(),
                remedy: remedy.id.clone(),
                ok: verified,
            });
            if !verified {
                state.mark(&at, OpStatus::Failed);
                self.emit_event(TraceEvent::Learned {
                    address: at.clone(),
                    class: dx.class.clone(),
                    remedy: remedy.id.clone(),
                    positive: false,
                });
                self.emit_event(TraceEvent::RemedyRejected {
                    address: at.clone(),
                    remedy: remedy.id.clone(),
                    stage: RejectStage::Verification,
                });
                self.kb.learn(
                    kb_key,
                    remedy.id.clone(),
                    mode.verification_desc.clone(),
                    false,
                );
                rejected.push(format!(
                    "{} (verification failed: {})",
                    remedy.description,
                    mode.verification.describe()
                ));
                continue;
            }

            // Known state. Learn it and report.
            self.emit_event(TraceEvent::Learned {
                address: at.clone(),
                class: dx.class.clone(),
                remedy: remedy.id.clone(),
                positive: true,
            });
            self.kb.learn(
                kb_key,
                remedy.id.clone(),
                mode.verification_desc.clone(),
                true,
            );
            return FailureReport {
                failure: obs.summary.clone(),
                class: dx.class.clone(),
                evidence: dx.evidence.clone(),
                confidence: dx.confidence,
                resolution: remedy.description.clone(),
                constraint: remedy
                    .constraints
                    .first()
                    .map(|c| c.describe())
                    .unwrap_or_else(|| "declared remediation without constraints".into()),
                verification: format!("{} — held; {at} resumed", mode.verification_desc),
                disposition: Disposition::Recovered,
                attempts,
                rejected,
                trace: self.events[trace_start..].to_vec(),
            };
        }

        self.emit_event(TraceEvent::EscalatedExhausted {
            address: at.clone(),
            attempts,
            rejected: rejected.len(),
        });
        FailureReport {
            failure: obs.summary.clone(),
            class: dx.class.clone(),
            evidence: dx.evidence.clone(),
            confidence: dx.confidence,
            resolution: if attempts == 0 && rejected.is_empty() {
                "escalated: no remediation is permitted for this failure mode".into()
            } else {
                format!("escalated after {attempts} attempt(s)")
            },
            constraint: if attempts == 0 && rejected.is_empty() {
                "no permitted remediation satisfied the declared constraints".into()
            } else {
                "all permitted remediations were rejected by policy or verification".into()
            },
            verification: "not satisfied".into(),
            disposition: Disposition::Escalated,
            attempts,
            rejected,
            trace: self.events[trace_start..].to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::SignalValue;

    #[test]
    fn verification_predicates_evaluate_deterministically() {
        let mut state = ExecutionState::new(vec![]);
        state.set_capability("generation", 4);
        let signals = vec![Signal::new("http_status", SignalValue::int(200))];
        let predicate = Verification::All(vec![
            Verification::CapabilityAtLeast {
                capability: "generation".into(),
                tier: 3,
            },
            Verification::Any(vec![
                Verification::SignalEquals {
                    key: "http_status".into(),
                    value: SignalValue::int(200),
                },
                Verification::SignalEquals {
                    key: "http_status".into(),
                    value: SignalValue::int(201),
                },
            ]),
        ]);
        assert!(eval_verification(&predicate, &state, &signals));
        assert!(!eval_verification(
            &predicate,
            &ExecutionState::new(vec![]),
            &signals
        ));
    }

    #[test]
    fn remedy_error_converts_from_strings_and_is_a_std_error() {
        let from_str = RemedyError::from("window expired");
        let from_string = RemedyError::from("window expired".to_string());
        assert_eq!(from_str, from_string);
        let err = Box::new(from_str) as Box<dyn std::error::Error>;
        assert!(err.to_string().contains("window expired"));
    }
}
