//! # DEFAIL — Deterministic Embedded Failure Addressing Inference Logic
//!
//! DEFAIL is an inference/control layer that makes failures deterministic,
//! attributable, and actionable, instead of letting them propagate as vague
//! errors.
//!
//! The pipeline every failure travels:
//!
//! ```text
//! execution → detection → classification → causal inference
//!          → remediation selection → policy validation
//!          → remediation execution → verification → resume / escalate
//! ```
//!
//! ## The determinism contract
//!
//! Inference proposes, deterministic policy validates, remediation executes,
//! an invariant verifies. If inference cannot produce a remediation that
//! satisfies the constraints the component declared, DEFAIL escalates — it
//! never invents a fix.
//!
//! ## Embedded declarations
//!
//! Components describe themselves with [`ComponentSpec`]: their capabilities,
//! their [`FailureMode`]s (diagnostic signals, permitted remediations,
//! verification predicates), and the [`RemedySpec`]s they permit with the
//! [`Constraint`]s under which each is valid (for example a substitution
//! window that closes once the batter is committed).
//!
//! ## Addressable execution state
//!
//! Every operation has an address ([`OpAddress`], e.g. `Baker.recipe.combine`).
//! At a point of failure DEFAIL can retrieve local state, upstream state, and
//! applicable constraints from [`ExecutionState`] — an addressable model of
//! the application's execution graph.
//!
//! ## Enforce
//!
//! [`Enforcer`] is the gate: an operation may only run when its prerequisites
//! are done and no earlier failure stands unresolved. That is what prevents
//! `continue_to_bake()` until a valid recovery state has been established.
//!
//! ## Learning
//!
//! Every resolution attempt updates [`KnowledgeBase`] (`failure type →
//! remediation → verification`), so the same failure produces the same
//! response next time, deterministically.

#![forbid(unsafe_code)]

pub mod address;
pub mod declare;
pub mod demo;
pub mod enforce;
pub mod engine;
pub mod inference;
pub mod json;
pub mod knowledge;
pub mod policy;
pub mod report;
pub mod signal;
pub mod state;
pub mod store;
pub mod trace;

pub use address::{BadAddress, OpAddress};
pub use declare::{
    ComponentSpec, Constraint, DeclarationError, FailureClass, FailureMode, RemedyId, RemedyKind,
    RemedySpec, Verification,
};
pub use enforce::{Enforcer, GateReason, GateViolation};
pub use engine::{AppWorld, DeFail, RemedyError, StepOutcome};
pub use knowledge::{ContextSig, KbEntry, KbKey, KnowledgeBase, LoadReport};
pub use report::{Disposition, FailureReport};
pub use signal::{Observation, Signal, SignalPattern, SignalValue};
pub use state::{ExecutionState, OpStatus, PlanStep};
pub use store::{Backend, KnowledgeStore, StoreError};
pub use trace::{NoopSink, RejectStage, RemedySource, TraceEvent, TraceSink, VecSink};
