//! Worked example: a search client embeds DEFAIL to ride through a
//! rate-limited backend.
//!
//! The example defines its own `AppWorld` (it does not call into
//! `defail::demo`), declares its own validated `ComponentSpec`, runs a
//! scripted failure through `Enforcer::run_step`, prints the resulting
//! `FailureReport`, and inspects the decision trace through a `VecSink`.
//!
//! Run: `cargo run --example provider` (the replica accepts the fallback and
//! the query recovers) or `cargo run --example provider -- --degraded` (the
//! replica is down too: escalation, and the gate blocks the render step).
//! Deterministic; the only I/O is stdout.

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

const QUERY: &str = "SearchApp.query.run";
const RENDER: &str = "SearchApp.query.render";

fn addr(raw: &str) -> OpAddress {
    OpAddress::new(raw).expect("static address")
}

/// The client's self-declaration: one failure mode (rate limit), one
/// permitted remediation (fail over to the replica), and the capability
/// invariant recovery must preserve.
fn search_spec() -> ComponentSpec {
    ComponentSpec {
        name: "SearchApp".into(),
        capabilities: vec![("relevance".into(), 2)],
        failure_modes: vec![FailureMode {
            id: "rate-limited".into(),
            class: FailureClass::new("capacity/rate-limit"),
            summary: "search backend rejected the query".into(),
            patterns: vec![
                SignalPattern::exact("http_status", SignalValue::Int(429), 0.6),
                SignalPattern::any("retry_after", 0.38),
                SignalPattern::any("backend_outage", 0.02),
            ],
            permitted: vec![RemedyId::new("failover-replica")],
            verification: Verification::CapabilityAtLeast {
                capability: "relevance".into(),
                tier: 2,
            },
            verification_desc: "replica answered an equivalent query".into(),
            min_confidence: 0.9,
        }],
        remedies: vec![RemedySpec {
            id: RemedyId::new("failover-replica"),
            description: "fail over to the replica search backend".into(),
            kind: RemedyKind::Fallback {
                to: "replica".into(),
            },
            precedence: 1,
            constraints: vec![Constraint::CapabilityAtLeast {
                capability: "relevance".into(),
                tier: 2,
            }],
        }],
    }
    .validated()
    .expect("example declaration must pass validation")
}

/// The host application: a scripted search client. The primary backend
/// always rejects; with `replica_ok` the replica answers, otherwise it
/// rejects too and the failure must escalate.
struct SearchWorld {
    replica_ok: bool,
    active: u8,
    last_signals: Vec<Signal>,
}

impl AppWorld for SearchWorld {
    fn execute(
        &mut self,
        address: &OpAddress,
        state: &mut ExecutionState,
    ) -> Result<(), Observation> {
        self.last_signals.clear();
        match address.as_str() {
            QUERY => {
                let tier = match self.active {
                    0 => {
                        // Primary backend: quota exhausted.
                        self.last_signals
                            .push(Signal::new("http_status", SignalValue::Int(429)));
                        self.last_signals
                            .push(Signal::new("retry_after", SignalValue::text("present")));
                        state.set_capability("relevance", 0);
                        return Err(Observation::new(
                            address.clone(),
                            "search backend rejected the query",
                            self.last_signals.clone(),
                        ));
                    }
                    1 if self.replica_ok => 3,
                    _ => {
                        self.last_signals
                            .push(Signal::new("http_status", SignalValue::Int(429)));
                        self.last_signals
                            .push(Signal::new("backend_outage", SignalValue::Bool(true)));
                        state.set_capability("relevance", 0);
                        return Err(Observation::new(
                            address.clone(),
                            "search backend rejected the query",
                            self.last_signals.clone(),
                        ));
                    }
                };
                state.set_capability("relevance", tier);
                state.set_param("results", "10 hits");
                Ok(())
            }
            RENDER => {
                state.set_param("page", "rendered");
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
            "failover-replica" => {
                self.active = 1;
                state.note("active backend switched to the replica");
                Ok(())
            }
            other => Err(format!("the client does not implement remedy `{other}`").into()),
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
    let degraded = std::env::args().skip(1).any(|arg| arg == "--degraded");

    let sink = SharedSink::default();
    let engine = DeFail::new(search_spec()).with_sink(sink.clone());
    let mut enforcer = Enforcer::new(engine);
    let mut world = SearchWorld {
        replica_ok: !degraded,
        active: 0,
        last_signals: Vec::new(),
    };
    let mut state = ExecutionState::new(vec![
        PlanStep::new(addr(QUERY), "run the search query"),
        PlanStep::new(addr(RENDER), "render the results page"),
    ]);
    enforcer.engine_ref().seed_capabilities(&mut state);

    println!(
        "== search client ({}) ==",
        if degraded { "replica degraded" } else { "rate-limited, replica healthy" }
    );
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
