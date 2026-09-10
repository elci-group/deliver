//! Deterministic policy: validate remediation constraints against the
//! execution state. Inference proposes; policy decides.

use std::fmt;

use crate::declare::Constraint;
use crate::state::ExecutionState;

/// A policy violation: which constraint failed and why, in addressable terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub constraint: String,
    pub reason: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "violated {}: {}", self.constraint, self.reason)
    }
}

impl std::error::Error for Violation {}

fn pre_violation(constraint: &Constraint, state: &ExecutionState) -> Option<String> {
    match constraint {
        Constraint::WindowOpen {
            parameter: _,
            closes_after,
        } if state.reached(closes_after) => Some(format!(
            "substitution window expired: {closes_after} has already run"
        )),
        Constraint::StateNotReached { address } if state.reached(address) => {
            Some(format!("{address} has already run"))
        }
        _ => None,
    }
}

fn post_violation(constraint: &Constraint, state: &ExecutionState) -> Option<String> {
    match constraint {
        Constraint::CapabilityAtLeast { capability, tier }
            if state.capability_tier(capability) < *tier =>
        {
            Some(format!(
                "capability {capability} is at tier {} but tier {tier} is required",
                state.capability_tier(capability)
            ))
        }
        _ => None,
    }
}

/// Validates constraints that must hold before a remedy runs: substitution
/// windows and state shape.
pub struct Policy;

impl Policy {
    pub fn validate_pre(
        constraints: &[Constraint],
        state: &ExecutionState,
    ) -> Result<(), Violation> {
        for constraint in constraints {
            if let Some(reason) = pre_violation(constraint, state) {
                return Err(Violation {
                    constraint: constraint.describe(),
                    reason,
                });
            }
        }
        Ok(())
    }

    /// Validates constraints that must hold after the remedy has run and the
    /// operation has resumed: capability invariants.
    pub fn validate_post(
        constraints: &[Constraint],
        state: &ExecutionState,
    ) -> Result<(), Violation> {
        for constraint in constraints {
            if let Some(reason) = post_violation(constraint, state) {
                return Err(Violation {
                    constraint: constraint.describe(),
                    reason,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::OpAddress;
    use crate::state::{ExecutionState, OpStatus, PlanStep};

    fn addr(s: &str) -> OpAddress {
        OpAddress::new(s).unwrap()
    }

    #[test]
    fn substitution_window_expires_once_committed() {
        let mut state = ExecutionState::new(vec![
            PlanStep::new(addr("app.combine"), "combine"),
            PlanStep::new(addr("app.bake"), "bake"),
        ]);
        let window = Constraint::WindowOpen {
            parameter: "leavening_agent".into(),
            closes_after: addr("app.combine"),
        };
        assert!(Policy::validate_pre(std::slice::from_ref(&window), &state).is_ok());
        state.mark(&addr("app.combine"), OpStatus::Done);
        let err = Policy::validate_pre(&[window], &state).unwrap_err();
        assert!(err.reason.contains("substitution window expired"));
    }

    #[test]
    fn capability_invariant_checked_after_remedy() {
        let state = ExecutionState::new(vec![]);
        let constraint = Constraint::CapabilityAtLeast {
            capability: "generation".into(),
            tier: 3,
        };
        assert!(Policy::validate_post(std::slice::from_ref(&constraint), &state).is_err());
        let mut recovered = state;
        recovered.set_capability("generation", 4);
        assert!(Policy::validate_post(&[constraint], &recovered).is_ok());
    }

    #[test]
    fn violation_is_a_std_error() {
        let err = Box::new(Violation {
            constraint: "preserve capability generation at tier >= 3".into(),
            reason: "capability generation is at tier 0 but tier 3 is required".into(),
        }) as Box<dyn std::error::Error>;
        assert!(err.to_string().contains("tier"));
    }
}
