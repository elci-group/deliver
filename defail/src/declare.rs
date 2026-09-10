//! Embedded self-declaration: a component describes its capabilities, its
//! failure modes, the remediations it permits, and how recovery is proven.

use std::fmt;

use crate::address::OpAddress;
use crate::signal::{SignalPattern, SignalValue};

/// Class of failure, e.g. `capacity/rate-limit` or `resource/ingredient-missing`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FailureClass(String);

impl FailureClass {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier of a declared remediation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemedyId(String);

impl RemedyId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RemedyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a remediation does. Descriptive, not executable: the mechanics live in
/// the host application's [`AppWorld`](crate::engine::AppWorld) implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemedyKind {
    Retry,
    Substitute {
        parameter: String,
        replacement: String,
    },
    Fallback {
        to: String,
    },
    RefreshCredential,
    ConstrainedProceed {
        directive: String,
    },
    Escalate,
}

impl RemedyKind {
    pub fn describe(&self) -> String {
        match self {
            RemedyKind::Retry => "retry the operation".into(),
            RemedyKind::Substitute {
                parameter,
                replacement,
            } => format!("substitute {replacement} for {parameter}"),
            RemedyKind::Fallback { to } => format!("switch to {to}"),
            RemedyKind::RefreshCredential => "refresh the credential and retry".into(),
            RemedyKind::ConstrainedProceed { directive } => {
                format!("proceed under constraint: {directive}")
            }
            RemedyKind::Escalate => "escalate to the supervising operator".into(),
        }
    }
}

/// A deterministic constraint a remediation must satisfy against the execution
/// state. Policy validates these before and after the remedy runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    /// The parameter may only be substituted before this address has run:
    /// a substitution window. Once the batter is committed, the window is
    /// expired and the remedy is invalid no matter what inference proposes.
    WindowOpen {
        parameter: String,
        closes_after: OpAddress,
    },
    /// The state must provide `capability` at tier >= `tier` once the remedy
    /// has run. Post-condition, checked after execution.
    CapabilityAtLeast { capability: String, tier: u32 },
    /// The given address must not have run yet.
    StateNotReached { address: OpAddress },
}

impl Constraint {
    pub fn describe(&self) -> String {
        match self {
            Constraint::WindowOpen {
                parameter,
                closes_after,
            } => format!(
                "substitution window for {parameter} is open until {closes_after} completes"
            ),
            Constraint::CapabilityAtLeast { capability, tier } => {
                format!("preserve capability {capability} at tier >= {tier}")
            }
            Constraint::StateNotReached { address } => format!("{address} must not have run yet"),
        }
    }
}

/// A declared verification predicate: how the component proves it recovered.
#[derive(Debug, Clone, PartialEq)]
pub enum Verification {
    SignalEquals { key: String, value: SignalValue },
    CapabilityAtLeast { capability: String, tier: u32 },
    StateReached { address: OpAddress },
    All(Vec<Verification>),
    Any(Vec<Verification>),
}

impl Verification {
    pub fn describe(&self) -> String {
        match self {
            Verification::SignalEquals { key, value } => format!("signal {key} equals {value}"),
            Verification::CapabilityAtLeast { capability, tier } => {
                format!("capability {capability} >= tier {tier} holds")
            }
            Verification::StateReached { address } => format!("{address} reached a valid state"),
            Verification::All(vs) => join_descriptions(vs, " and "),
            Verification::Any(vs) => join_descriptions(vs, " or "),
        }
    }
}

fn join_descriptions(vs: &[Verification], sep: &str) -> String {
    vs.iter()
        .map(Verification::describe)
        .collect::<Vec<_>>()
        .join(sep)
}

/// Why a declaration is invalid. Bad declarations must fail loudly at
/// construction time — DEFAIL never classifies against a declaration that
/// could produce a meaningless confidence.
#[derive(Debug, Clone, PartialEq)]
pub enum DeclarationError {
    /// A signal pattern weight is NaN, infinite, or negative.
    BadWeight { mode: String, key: String, weight: f32 },
    /// A failure mode's minimum confidence is outside `[0, 1]`.
    BadMinConfidence { mode: String, min_confidence: f32 },
    /// A failure mode permits no remedies at all: nothing could ever resolve it.
    EmptyPermitted { mode: String },
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclarationError::BadWeight { mode, key, weight } => write!(
                f,
                "failure mode `{mode}` gives signal `{key}` an invalid weight {weight} \
                 (must be a positive, finite number)"
            ),
            DeclarationError::BadMinConfidence {
                mode,
                min_confidence,
            } => write!(
                f,
                "failure mode `{mode}` declares min_confidence {min_confidence} \
                 outside the [0, 1] interval"
            ),
            DeclarationError::EmptyPermitted { mode } => {
                write!(f, "failure mode `{mode}` permits no remedies")
            }
        }
    }
}

impl std::error::Error for DeclarationError {}

/// A failure mode a component declares: how to recognise it, which remediations
/// are permitted, and how recovery is proven.
#[derive(Debug, Clone)]
pub struct FailureMode {
    pub id: String,
    pub class: FailureClass,
    /// Human-readable statement of the failure, e.g. "provider request rejected".
    pub summary: String,
    /// Diagnostic signal patterns; matched weights produce the diagnosis confidence.
    pub patterns: Vec<SignalPattern>,
    /// Remediations permitted for this mode, by id.
    pub permitted: Vec<RemedyId>,
    /// Verification predicate that must hold after remediation.
    pub verification: Verification,
    /// Human-readable statement of what verification means.
    pub verification_desc: String,
    /// Minimum diagnosis confidence required before remediating at all.
    pub min_confidence: f32,
}

impl FailureMode {
    /// Validate this declaration. Returns [`DeclarationError`] on the first
    /// problem found: a NaN/negative pattern weight, a `min_confidence`
    /// outside `[0, 1]`, or an empty `permitted` remedy list.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        if self.permitted.is_empty() {
            return Err(DeclarationError::EmptyPermitted {
                mode: self.id.clone(),
            });
        }
        if !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(DeclarationError::BadMinConfidence {
                mode: self.id.clone(),
                min_confidence: self.min_confidence,
            });
        }
        for pattern in &self.patterns {
            if !pattern.weight.is_finite() || pattern.weight < 0.0 {
                return Err(DeclarationError::BadWeight {
                    mode: self.id.clone(),
                    key: pattern.key.clone(),
                    weight: pattern.weight,
                });
            }
        }
        Ok(())
    }
}

/// A remediation the component permits, with the constraints under which it
/// is valid. Lower `precedence` values are considered first.
#[derive(Debug, Clone)]
pub struct RemedySpec {
    pub id: RemedyId,
    pub description: String,
    pub kind: RemedyKind,
    pub precedence: u32,
    pub constraints: Vec<Constraint>,
}

/// The embedded declaration of a component: its capabilities, failure modes,
/// and permitted remediations. This is what lets DEFAIL construct a
/// deterministic failure-resolution graph.
#[derive(Debug, Clone)]
pub struct ComponentSpec {
    pub name: String,
    /// Capability name → nominal tier.
    pub capabilities: Vec<(String, u32)>,
    pub failure_modes: Vec<FailureMode>,
    pub remedies: Vec<RemedySpec>,
}

impl ComponentSpec {
    pub fn remedy(&self, id: &RemedyId) -> Option<&RemedySpec> {
        self.remedies.iter().find(|r| &r.id == id)
    }

    /// Validate every declared failure mode. Bad declarations fail loudly
    /// here — at construction time — instead of classifying silently later.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        for mode in &self.failure_modes {
            mode.validate()?;
        }
        Ok(())
    }

    /// Consumes the spec: validate it, then return it for embedding.
    pub fn validated(self) -> Result<Self, DeclarationError> {
        self.validate()?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode() -> FailureMode {
        FailureMode {
            id: "rate-limited".into(),
            class: FailureClass::new("capacity/rate-limit"),
            summary: "provider request rejected".into(),
            patterns: vec![
                SignalPattern::exact("http_status", SignalValue::Int(429), 0.6),
                SignalPattern::any("quota_header", 0.4),
            ],
            permitted: vec![RemedyId::new("fallback")],
            verification: Verification::CapabilityAtLeast {
                capability: "generation".into(),
                tier: 3,
            },
            verification_desc: "equivalent request accepted".into(),
            min_confidence: 0.9,
        }
    }

    #[test]
    fn validation_rejects_nan_and_negative_weights() {
        let mut bad = mode();
        bad.patterns[0].weight = f32::NAN;
        assert!(matches!(
            bad.validate(),
            Err(DeclarationError::BadWeight { .. })
        ));
        let mut bad = mode();
        bad.patterns[1].weight = -0.5;
        assert!(matches!(
            bad.validate(),
            Err(DeclarationError::BadWeight { .. })
        ));
    }

    #[test]
    fn validation_rejects_min_confidence_outside_unit_interval() {
        for out_of_range in [-0.1, 1.5, f32::NAN, f32::INFINITY] {
            let mut bad = mode();
            bad.min_confidence = out_of_range;
            assert!(matches!(
                bad.validate(),
                Err(DeclarationError::BadMinConfidence { .. })
            ));
        }
        let mut edge = mode();
        edge.min_confidence = 0.0;
        assert!(edge.validate().is_ok());
        edge.min_confidence = 1.0;
        assert!(edge.validate().is_ok());
    }

    #[test]
    fn validation_rejects_empty_permitted_remedies() {
        let mut bad = mode();
        bad.permitted = vec![];
        assert!(matches!(
            bad.validate(),
            Err(DeclarationError::EmptyPermitted { .. })
        ));
    }

    #[test]
    fn valid_declaration_and_demo_specs_pass_validation() {
        assert!(mode().validate().is_ok());
        let spec = ComponentSpec {
            name: "T".into(),
            capabilities: vec![],
            failure_modes: vec![mode()],
            remedies: vec![],
        };
        assert!(spec.validated().is_ok());
        assert!(crate::demo::baker_spec().validate().is_ok());
        assert!(crate::demo::provider_spec().validate().is_ok());
    }
}
