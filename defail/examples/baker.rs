//! Worked example: a small bakery line embeds DEFAIL instead of panicking
//! when an ingredient goes missing.
//!
//! The example defines its own `AppWorld` (it does not call into
//! `defail::demo`), declares its own validated `ComponentSpec`, runs a
//! scripted failure through `Enforcer::run_step`, prints the resulting
//! `FailureReport`, and inspects the decision trace through a `VecSink`.
//!
//! Run: `cargo run --example baker` (missing powder caught early, while the
//! substitution window is open) or `cargo run --example baker -- --late`
//! (discovered at the oven, after the batter is committed). Deterministic;
//! the only I/O is stdout.

use std::cell::RefCell;
use std::rc::Rc;

use defail::declare::{
    ComponentSpec, Constraint, FailureClass, FailureMode, RemedyId, RemedyKind, RemedySpec,
    Verification,
};
use defail::engine::{AppWorld, RemedyError, StepOutcome};
use defail::signal::{Observation, Signal, SignalPattern, SignalValue};
use defail::state::{ExecutionState, PlanStep};
use defail::trace::{TraceEvent, TraceSink, VecSink};
use defail::{DeFail, Enforcer, OpAddress};

const PREP_DRY: &str = "Bakery.line.prep_dry";
const MIX_WET: &str = "Bakery.line.mix_wet";
const COMBINE: &str = "Bakery.line.combine";
const BAKE: &str = "Bakery.line.bake";

fn addr(raw: &str) -> OpAddress {
    OpAddress::new(raw).expect("static address")
}

/// The bakery's self-declaration: one failure mode (baking powder missing),
/// two permitted remediations (substitute, or proceed under constraint).
/// `validated()` rejects the declaration loudly if it is ever malformed.
fn bakery_spec() -> ComponentSpec {
    let combine = addr(COMBINE);
    ComponentSpec {
        name: "Bakery".into(),
        capabilities: vec![("leavening".into(), 1)],
        failure_modes: vec![FailureMode {
            id: "missing-powder".into(),
            class: FailureClass::new("resource/ingredient-missing"),
            summary: "baking powder unavailable on the line".into(),
            patterns: vec![
                SignalPattern::exact("ingredient_missing", SignalValue::text("baking_powder"), 0.6),
                SignalPattern::any("pantry_audited", 0.4),
            ],
            permitted: vec![
                RemedyId::new("substitute-egg-white"),
                RemedyId::new("constrained-dense-bake"),
            ],
            verification: Verification::Any(vec![
                Verification::CapabilityAtLeast { capability: "leavening".into(), tier: 1 },
                Verification::StateReached { address: addr(BAKE) },
            ]),
            verification_desc: "leavening preserved, or dense bake completed".into(),
            min_confidence: 0.5,
        }],
        remedies: vec![
            RemedySpec {
                id: RemedyId::new("substitute-egg-white"),
                description: "substitute whipped egg whites for baking powder".into(),
                kind: RemedyKind::Substitute {
                    parameter: "leavening_agent".into(),
                    replacement: "egg_white".into(),
                },
                precedence: 1,
                constraints: vec![Constraint::WindowOpen {
                    parameter: "leavening_agent".into(),
                    closes_after: combine,
                }],
            },
            RemedySpec {
                id: RemedyId::new("constrained-dense-bake"),
                description: "proceed with the constrained dense-bake recovery procedure".into(),
                kind: RemedyKind::ConstrainedProceed {
                    directive: "substitution window expired; do not modify the recipe".into(),
                },
                precedence: 9,
                constraints: vec![],
            },
        ],
    }
    .validated()
    .expect("example declaration must pass validation")
}

/// The host application: a scripted bakery line. `late = false` models the
/// pantry audit catching the missing powder at the dry-prep step;
/// `late = true` models it slipping through until the rise check at the oven.
struct BakeryWorld {
    late: bool,
    last_signals: Vec<Signal>,
}

impl AppWorld for BakeryWorld {
    fn execute(
        &mut self,
        address: &OpAddress,
        state: &mut ExecutionState,
    ) -> Result<(), Observation> {
        self.last_signals.clear();
        match address.as_str() {
            PREP_DRY => {
                let missing_now = !self.late && state.param("leavening_agent").is_none();
                if missing_now {
                    self.last_signals.push(Signal::new(
                        "ingredient_missing",
                        SignalValue::text("baking_powder"),
                    ));
                    self.last_signals
                        .push(Signal::new("pantry_audited", SignalValue::Bool(true)));
                    state.set_capability("leavening", 0);
                    return Err(Observation::new(
                        address.clone(),
                        "baking powder unavailable on the line",
                        self.last_signals.clone(),
                    ));
                }
                state.set_param("dry_mix", "flour+sugar");
                Ok(())
            }
            MIX_WET => {
                state.set_param("wet_mix", "eggs+milk");
                Ok(())
            }
            COMBINE => {
                state.set_param("batter", "committed");
                Ok(())
            }
            BAKE => {
                let unrecoverable =
                    state.param("leavening_agent").is_none() && state.param("recovery").is_none();
                if unrecoverable {
                    self.last_signals.push(Signal::new(
                        "ingredient_missing",
                        SignalValue::text("baking_powder"),
                    ));
                    state.set_capability("leavening", 0);
                    return Err(Observation::new(
                        address.clone(),
                        "baking powder unavailable (discovered after the batter was committed)",
                        self.last_signals.clone(),
                    ));
                }
                let leavened = state.param("leavening_agent").is_some();
                state.set_param("cake", if leavened { "risen" } else { "dense" });
                Ok(())
            }
            other => Err(Observation::new(
                address.clone(),
                format!("unknown operation: {other}"),
                Vec::new(),
            )),
        }
    }

    fn apply_remedy(
        &mut self,
        remedy: &RemedySpec,
        state: &mut ExecutionState,
    ) -> Result<(), RemedyError> {
        match remedy.id.as_str() {
            "substitute-egg-white" => {
                if state.reached(&addr(COMBINE)) {
                    return Err("substitution window expired: batter already combined".into());
                }
                state.set_param("leavening_agent", "egg_white");
                state.set_capability("leavening", 1);
                Ok(())
            }
            "constrained-dense-bake" => {
                state.set_param("recovery", "dense-bake");
                Ok(())
            }
            other => Err(format!("the line does not implement remedy `{other}`").into()),
        }
    }

    fn recent_signals(&self) -> &[Signal] {
        &self.last_signals
    }
}

/// A shareable handle over a `VecSink`: `DeFail::with_sink` takes ownership
/// of its sink, so the host shares it through `Rc<RefCell<...>>` to inspect
/// the stream afterwards (single-threaded, std-only).
#[derive(Clone, Default)]
struct SharedSink(Rc<RefCell<VecSink>>);

impl TraceSink for SharedSink {
    fn on_event(&mut self, event: &TraceEvent) {
        self.0.borrow_mut().on_event(event);
    }
}

fn main() {
    let late = std::env::args().skip(1).any(|arg| arg == "--late");

    let sink = SharedSink::default();
    let engine = DeFail::new(bakery_spec()).with_sink(sink.clone());
    let mut enforcer = Enforcer::new(engine);
    let mut world = BakeryWorld {
        late,
        last_signals: Vec::new(),
    };
    let mut state = ExecutionState::new(vec![
        PlanStep::new(addr(PREP_DRY), "combine flour and sugar"),
        PlanStep::new(addr(MIX_WET), "whisk eggs and milk"),
        PlanStep::new(addr(COMBINE), "fold the wet into the dry"),
        PlanStep::new(addr(BAKE), "bake the batter"),
    ]);
    enforcer.engine_ref().seed_capabilities(&mut state);

    println!("== bakery line ({} discovery) ==", if late { "late" } else { "early" });
    for step in state.addresses() {
        match enforcer.run_step(&mut world, &mut state, &step) {
            Ok(StepOutcome::Completed) => println!("{step}: completed"),
            Ok(StepOutcome::Recovered(report)) => {
                println!("{step}: recovered");
                println!("{report}");
            }
            Ok(StepOutcome::Escalated(report)) => {
                println!("{step}: escalated");
                println!("{report}");
            }
            Err(violation) => println!("{step}: gated — {violation}"),
        }
    }

    let events = sink.0.borrow();
    println!();
    println!("trace observed by the host sink ({} events):", events.events().len());
    for event in events.events() {
        println!("  {}", event.to_json());
    }
}
