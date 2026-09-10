//! DEFAIL ENFORCE: prevents the application from continuing incorrectly
//! after a failure.
//!
//! The gate answers one question for every requested operation: given what
//! has happened so far, is the application still permitted to do this? An
//! operation may run only when every prerequisite step is Done and no earlier
//! failure stands unresolved in the plan. This is what stops
//! `continue_to_bake()` until a valid recovery state has been established.
//!
//! Prerequisite semantics: the gate checks the plan steps declared before
//! the operation, together with the operation's direct `requires` edges
//! (transitively-declared prerequisites are not individually gated on).
//! Cycle detection, however, follows the transitive closure of `requires`:
//! a cycle in that graph can never complete, so instead of gating forever
//! the gate reports
//! [`GateReason::CyclicPrerequisites`] and denies the operation.

use std::collections::HashMap;
use std::fmt;

use crate::address::OpAddress;
use crate::engine::{AppWorld, DeFail, StepOutcome};
use crate::state::{ExecutionState, OpStatus};
use crate::trace::TraceEvent;

/// Why the gate refused an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateReason {
    /// Earlier plan steps have not run.
    PrerequisitesIncomplete(Vec<OpAddress>),
    /// An earlier step failed and its failure was not resolved; the
    /// application is constrained to a valid recovery path before proceeding.
    BlockedByFailure { failed: OpAddress },
    /// The declared `requires` graph contains a cycle reachable from the
    /// requested operation; such prerequisites can never all complete.
    /// `cycle` holds the addresses in traversal order (head not repeated):
    /// `[A, B]` means `A` requires `B` requires `A`.
    CyclicPrerequisites { cycle: Vec<OpAddress> },
    /// The operation is not part of the declared plan.
    UnknownOperation(OpAddress),
}

impl fmt::Display for GateReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateReason::PrerequisitesIncomplete(missing) => write!(
                f,
                "prerequisites incomplete: {}",
                missing
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            GateReason::BlockedByFailure { failed } => {
                write!(f, "blocked by unresolved failure at {failed}")
            }
            GateReason::CyclicPrerequisites { cycle } => write!(
                f,
                "cyclic prerequisites: {} can never complete",
                cycle
                    .iter()
                    .chain(cycle.first())
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            GateReason::UnknownOperation(op) => {
                write!(f, "unknown operation: {op} is not in the declared plan")
            }
        }
    }
}

impl std::error::Error for GateReason {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateViolation {
    pub op: OpAddress,
    pub reason: GateReason,
    /// What the application must do instead.
    pub directive: String,
}

impl fmt::Display for GateViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gate refused {}: {} ({})", self.op, self.reason, self.directive)
    }
}

impl std::error::Error for GateViolation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.reason)
    }
}

impl GateViolation {
    fn unknown(op: OpAddress) -> Self {
        Self {
            directive: "declare the operation in the plan before invoking it".into(),
            op: op.clone(),
            reason: GateReason::UnknownOperation(op),
        }
    }
}

/// Gate-keeping wrapper around [`DeFail`].
pub struct Enforcer {
    engine: DeFail,
}

impl Enforcer {
    pub fn new(engine: DeFail) -> Self {
        Self { engine }
    }

    pub fn engine(&mut self) -> &mut DeFail {
        &mut self.engine
    }

    pub fn engine_ref(&self) -> &DeFail {
        &self.engine
    }

    /// The gate every operation must pass. `Err` means: do not run this.
    ///
    /// Prerequisites are the plan steps declared before the operation plus
    /// the operation's direct `requires` edges, deduplicated in declaration
    /// order. Cycle detection follows the transitive closure of the declared
    /// `requires` graph: if that closure contains a cycle, the operation is
    /// denied with [`GateReason::CyclicPrerequisites`] — a cycle can never
    /// complete, so gating on it would mean gating forever.
    pub fn request(&self, state: &ExecutionState, op: &OpAddress) -> Result<(), GateViolation> {
        let Some(index) = state.index(op) else {
            return Err(GateViolation::unknown(op.clone()));
        };

        if let Some(cycle) = cyclic_prerequisites(state, op) {
            return Err(GateViolation {
                op: op.clone(),
                reason: GateReason::CyclicPrerequisites {
                    cycle: cycle.clone(),
                },
                directive: format!(
                    "the declared prerequisites of {op} contain a cycle ({}); \
                     remove or re-declare one of the edges",
                    cycle
                        .iter()
                        .chain(cycle.first())
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(" -> ")
                ),
            });
        }

        let mut missing: Vec<OpAddress> = Vec::new();
        let mut blocked: Option<OpAddress> = None;

        for (i, step) in state.steps().iter().enumerate() {
            if i >= index {
                break;
            }
            match state.status(&step.address) {
                OpStatus::Done => {}
                OpStatus::Failed if blocked.is_none() => blocked = Some(step.address.clone()),
                _ => {
                    if !missing.contains(&step.address) {
                        missing.push(step.address.clone());
                    }
                }
            }
        }
        for pre in &state.steps()[index].requires {
            match state.status(pre) {
                OpStatus::Done => {}
                OpStatus::Failed if blocked.is_none() => blocked = Some(pre.clone()),
                _ => {
                    if !missing.contains(pre) {
                        missing.push(pre.clone());
                    }
                }
            }
        }

        if let Some(failed) = blocked {
            return Err(GateViolation {
                op: op.clone(),
                reason: GateReason::BlockedByFailure { failed },
                directive: format!(
                    "a valid recovery state for {op} has not been established; \
                     resolve or escalate the failure before proceeding"
                ),
            });
        }
        if !missing.is_empty() {
            return Err(GateViolation {
                op: op.clone(),
                reason: GateReason::PrerequisitesIncomplete(missing.clone()),
                directive: format!(
                    "prerequisites not complete for {op}: {}",
                    missing
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        Ok(())
    }

    /// Gated execution: request the gate, then run the step under DEFAIL.
    /// A denial is recorded in the engine trace as a
    /// [`TraceEvent::GateDenied`].
    pub fn run_step(
        &mut self,
        world: &mut dyn AppWorld,
        state: &mut ExecutionState,
        op: &OpAddress,
    ) -> Result<StepOutcome, GateViolation> {
        match self.request(state, op) {
            Ok(()) => Ok(self.engine.run_step(world, state, op)),
            Err(violation) => {
                self.engine.emit_event(TraceEvent::GateDenied {
                    address: op.clone(),
                    reason: violation.reason.to_string(),
                });
                Err(violation)
            }
        }
    }
}

/// The declared `requires` edges of `address`, empty when the address is not
/// a declared step.
fn requires_of<'a>(state: &'a ExecutionState, address: &OpAddress) -> &'a [OpAddress] {
    state
        .steps()
        .iter()
        .find(|step| &step.address == address)
        .map(|step| step.requires.as_slice())
        .unwrap_or(&[])
}

/// Deterministic cycle detection over the declared `requires` graph, starting
/// at `op` and following its transitive closure. Edges to addresses outside
/// the declared plan cannot be part of a cycle; they are skipped here and
/// surface as ordinary incomplete prerequisites that `mark()` can never
/// satisfy. Returns the first cycle found,
/// in traversal order with the head not repeated, or `None`.
fn cyclic_prerequisites(state: &ExecutionState, op: &OpAddress) -> Option<Vec<OpAddress>> {
    // Iterative depth-first search with white/gray/black coloring, so a
    // deep plan cannot overflow the stack. `requires` are visited in
    // declaration order, so the reported cycle is deterministic.
    let mut color: HashMap<OpAddress, u8> = HashMap::new();
    let mut stack: Vec<(OpAddress, usize)> = vec![(op.clone(), 0)];
    color.insert(op.clone(), 1); // gray: on the current path
    while let Some((node, next)) = stack.last().cloned() {
        let deps = requires_of(state, &node);
        if next < deps.len() {
            stack.last_mut().expect("stack is non-empty").1 += 1;
            let dep = deps[next].clone();
            if state.index(&dep).is_none() {
                continue;
            }
            match color.get(&dep).copied().unwrap_or(0) {
                1 => {
                    // A gray node is on the current path: back edge found.
                    let pos = stack
                        .iter()
                        .position(|(on_path, _)| on_path == &dep)
                        .expect("gray nodes are exactly the path");
                    return Some(stack[pos..].iter().map(|(n, _)| n.clone()).collect());
                }
                2 => {} // black: fully explored, no cycle through it
                _ => {
                    color.insert(dep.clone(), 1);
                    stack.push((dep, 0));
                }
            }
        } else {
            let (done, _) = stack.pop().expect("stack is non-empty");
            color.insert(done, 2);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::OpAddress;
    use crate::declare::ComponentSpec;
    use crate::state::{ExecutionState, OpStatus, PlanStep};

    fn addr(s: &str) -> OpAddress {
        OpAddress::new(s).unwrap()
    }

    fn state() -> ExecutionState {
        ExecutionState::new(vec![
            PlanStep::new(addr("app.mix"), "mix"),
            PlanStep::new(addr("app.bake"), "bake"),
        ])
    }

    #[test]
    fn gate_requires_prerequisites() {
        let enforcer = Enforcer::new(DeFail::new(ComponentSpec {
            name: "app".into(),
            capabilities: vec![],
            failure_modes: vec![],
            remedies: vec![],
        }));
        let err = enforcer.request(&state(), &addr("app.bake")).unwrap_err();
        assert!(matches!(
            err.reason,
            GateReason::PrerequisitesIncomplete(ref missing) if missing.len() == 1
        ));
    }

    #[test]
    fn gate_blocks_downstream_of_unresolved_failure() {
        let enforcer = Enforcer::new(DeFail::new(ComponentSpec {
            name: "app".into(),
            capabilities: vec![],
            failure_modes: vec![],
            remedies: vec![],
        }));
        let mut state = state();
        state.mark(&addr("app.mix"), OpStatus::Failed);
        let err = enforcer.request(&state, &addr("app.bake")).unwrap_err();
        assert!(matches!(err.reason, GateReason::BlockedByFailure { .. }));
        assert!(err.directive.contains("recovery state"));
        // The failed operation itself may be re-attempted through the gate.
        assert!(enforcer.request(&state, &addr("app.mix")).is_ok());
        // Unknown operations are refused outright.
        assert!(matches!(
            enforcer
                .request(&state, &addr("app.teleport"))
                .unwrap_err()
                .reason,
            GateReason::UnknownOperation(_)
        ));
    }

    #[test]
    fn gate_violation_chains_its_reason() {
        let enforcer = Enforcer::new(DeFail::new(ComponentSpec {
            name: "app".into(),
            capabilities: vec![],
            failure_modes: vec![],
            remedies: vec![],
        }));
        let violation = enforcer
            .request(&state(), &addr("app.teleport"))
            .unwrap_err();
        let err = Box::new(violation) as Box<dyn std::error::Error>;
        assert!(err.source().is_some());
        assert!(err.to_string().contains("unknown operation"));
    }

    fn enforcer() -> Enforcer {
        Enforcer::new(DeFail::new(ComponentSpec {
            name: "app".into(),
            capabilities: vec![],
            failure_modes: vec![],
            remedies: vec![],
        }))
    }

    #[test]
    fn gate_reports_a_two_node_prerequisite_cycle() {
        let a = addr("app.a");
        let b = addr("app.b");
        let step_a = PlanStep {
            address: a.clone(),
            description: "a".into(),
            requires: vec![b.clone()],
        };
        let step_b = PlanStep {
            address: b.clone(),
            description: "b".into(),
            requires: vec![a.clone()],
        };
        let state = ExecutionState::new(vec![step_a, step_b]);
        let err = enforcer().request(&state, &a).unwrap_err();
        let GateReason::CyclicPrerequisites { ref cycle } = err.reason else {
            panic!("expected a cycle violation, got {err}");
        };
        // The cycle names both nodes, in deterministic traversal order.
        assert_eq!(cycle, &vec![a, b]);
        assert!(err.directive.contains("cycle"));
        assert!(err.to_string().contains("app.a"));
        assert!(err.to_string().contains("app.b"));
        // The same cycle is reported from the other side.
        let err = enforcer().request(&state, &addr("app.b")).unwrap_err();
        assert!(matches!(err.reason, GateReason::CyclicPrerequisites { .. }));
    }

    #[test]
    fn gate_reports_a_self_loop_as_a_cycle() {
        let a = addr("app.a");
        let step = PlanStep {
            address: a.clone(),
            description: "a".into(),
            requires: vec![a.clone()],
        };
        let state = ExecutionState::new(vec![step]);
        let err = enforcer().request(&state, &a).unwrap_err();
        assert!(matches!(
            err.reason,
            GateReason::CyclicPrerequisites { ref cycle } if cycle == &vec![a]
        ));
    }

    #[test]
    fn gate_reports_a_cycle_reached_transitively() {
        let a = addr("app.a");
        let b = addr("app.b");
        let c = addr("app.c");
        let plan = vec![
            PlanStep {
                address: a.clone(),
                description: "a".into(),
                requires: vec![b.clone()],
            },
            PlanStep {
                address: b.clone(),
                description: "b".into(),
                requires: vec![c.clone()],
            },
            PlanStep {
                address: c.clone(),
                description: "c".into(),
                requires: vec![b.clone()],
            },
        ];
        let state = ExecutionState::new(plan);
        // `a` is not on the cycle itself but reaches it through its closure.
        let err = enforcer().request(&state, &a).unwrap_err();
        let GateReason::CyclicPrerequisites { ref cycle } = err.reason else {
            panic!("expected a cycle violation, got {err}");
        };
        assert_eq!(cycle, &vec![b, c]);
    }

    #[test]
    fn duplicate_plan_addresses_are_reported_once() {
        // A plan that declares the same address twice must not duplicate the
        // address in the denial: entries are deduplicated, preserving
        // declaration order.
        let mix = addr("app.mix");
        let bake = addr("app.bake");
        let state = ExecutionState::new(vec![
            PlanStep::new(mix.clone(), "mix"),
            PlanStep::new(mix.clone(), "mix again"),
            PlanStep::new(bake.clone(), "bake"),
        ]);
        let err = enforcer().request(&state, &bake).unwrap_err();
        assert!(matches!(
            err.reason,
            GateReason::PrerequisitesIncomplete(ref missing) if missing == &vec![mix]
        ));
    }

    #[test]
    fn acyclic_requires_gate_normally() {
        let mix = addr("app.mix");
        let bake = addr("app.bake");
        let plan = vec![
            PlanStep::new(mix.clone(), "mix"),
            PlanStep {
                address: bake.clone(),
                description: "bake".into(),
                requires: vec![mix.clone()],
            },
        ];
        let mut state = ExecutionState::new(plan);
        // Prerequisite not done yet: ordinary incomplete-prerequisite denial.
        let err = enforcer().request(&state, &bake).unwrap_err();
        assert!(matches!(
            err.reason,
            GateReason::PrerequisitesIncomplete(ref missing) if missing == &vec![mix.clone()]
        ));
        // Prerequisite done: the gate opens.
        state.mark(&mix, OpStatus::Done);
        assert!(enforcer().request(&state, &bake).is_ok());
    }
}
