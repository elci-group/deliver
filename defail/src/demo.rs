//! Reference scenarios.
//!
//! * **Baker** — the substitution-window story. Baking soda discovered missing
//!   before the batter is committed: substitute yeast. Discovered after the
//!   substitution window expired: the recipe is frozen and recovery proceeds
//!   under constraint instead.
//! * **Provider** — the rate-limit story. HTTP 429 plus a quota header
//!   classifies as `capacity/rate-limit` at 0.98 confidence; fallback must
//!   preserve `generation >= tier 3` or it does not count as recovery.

use std::collections::BTreeMap;
use std::fmt;

use crate::address::OpAddress;
use crate::declare::{
    ComponentSpec, Constraint, FailureClass, FailureMode, RemedyId, RemedyKind, RemedySpec,
    Verification,
};
use crate::enforce::Enforcer;
use crate::engine::{AppWorld, DeFail, RemedyError, StepOutcome};
use crate::knowledge::KnowledgeBase;
use crate::report::FailureReport;
use crate::signal::{Observation, Signal, SignalPattern, SignalValue};
use crate::state::{ExecutionState, PlanStep};

fn addr(s: &str) -> OpAddress {
    OpAddress::new(s).expect("static address")
}

/// Declarations are validated at construction: a demo with an invalid
/// declaration is a build-time bug, not a runtime condition.
fn validated(spec: ComponentSpec) -> ComponentSpec {
    spec.validated()
        .expect("demo declaration must pass validation")
}

// ---------------------------------------------------------------------------
// Baker
// ---------------------------------------------------------------------------

pub const DRY_MIX: &str = "Baker.recipe.prepare.dry_mix";
pub const WET_MIX: &str = "Baker.recipe.prepare.wet_mix";
pub const COMBINE: &str = "Baker.recipe.combine";
pub const LEAVEN: &str = "Baker.recipe.leaven";
pub const BAKE: &str = "Baker.recipe.bake";

pub fn baker_spec() -> ComponentSpec {
    let combine = addr(COMBINE);
    let bake = addr(BAKE);
    validated(ComponentSpec {
        name: "Baker".into(),
        capabilities: vec![("leavening".into(), 1)],
        failure_modes: vec![FailureMode {
            id: "missing-ingredient".into(),
            class: FailureClass::new("resource/ingredient-missing"),
            summary: "required ingredient unavailable: baking soda".into(),
            patterns: vec![
                SignalPattern::exact("ingredient_missing", SignalValue::text("baking_soda"), 0.6),
                SignalPattern::any("inventory_checked", 0.4),
            ],
            permitted: vec![
                RemedyId::new("substitute-yeast"),
                RemedyId::new("constrained-proceed"),
            ],
            verification: Verification::Any(vec![
                Verification::CapabilityAtLeast {
                    capability: "leavening".into(),
                    tier: 1,
                },
                Verification::StateReached { address: bake },
            ]),
            verification_desc: "leavening capability preserved, or constrained bake completed".into(),
            min_confidence: 0.5,
        }],
        remedies: vec![
            RemedySpec {
                id: RemedyId::new("substitute-yeast"),
                description: "substitute yeast for baking soda with extended proof time".into(),
                kind: RemedyKind::Substitute {
                    parameter: "leavening_agent".into(),
                    replacement: "yeast".into(),
                },
                precedence: 1,
                constraints: vec![Constraint::WindowOpen {
                    parameter: "leavening_agent".into(),
                    closes_after: combine,
                }],
            },
            RemedySpec {
                id: RemedyId::new("constrained-proceed"),
                description: "proceed with the constrained recovery procedure (dense bake, no recipe mutation)".into(),
                kind: RemedyKind::ConstrainedProceed {
                    directive: "substitution window expired; do not modify the recipe".into(),
                },
                precedence: 9,
                constraints: vec![],
            },
        ],
    })
}

pub fn baker_plan() -> Vec<PlanStep> {
    vec![
        PlanStep::new(addr(DRY_MIX), "combine flour, sugar, baking soda"),
        PlanStep::new(addr(WET_MIX), "whisk eggs and milk"),
        PlanStep::new(addr(COMBINE), "incorporate wet into dry"),
        PlanStep::new(addr(LEAVEN), "proof to activate leavening"),
        PlanStep::new(addr(BAKE), "bake the batter"),
    ]
}

/// When the missing baking soda is discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SodaAvailability {
    Present,
    /// Inventory check at the dry-mix step catches the absence early.
    MissingEarly,
    /// The absence slips through and is only discovered at the oven.
    MissingLate,
}

pub struct BakerWorld {
    soda: SodaAvailability,
    last_signals: Vec<Signal>,
}

impl BakerWorld {
    pub fn new(soda: SodaAvailability) -> Self {
        Self {
            soda,
            last_signals: Vec::new(),
        }
    }
}

impl AppWorld for BakerWorld {
    fn execute(
        &mut self,
        address: &OpAddress,
        state: &mut ExecutionState,
    ) -> Result<(), Observation> {
        self.last_signals.clear();
        match address.as_str() {
            DRY_MIX => {
                let missing_now = matches!(self.soda, SodaAvailability::MissingEarly)
                    && state.param("leavening_agent").is_none();
                if missing_now {
                    self.last_signals.push(Signal::new(
                        "ingredient_missing",
                        SignalValue::text("baking_soda"),
                    ));
                    self.last_signals
                        .push(Signal::new("inventory_checked", SignalValue::Bool(true)));
                    state.set_capability("leavening", 0);
                    state.note("dry mix halted: baking soda absent from inventory");
                    return Err(Observation::new(
                        address.clone(),
                        "required ingredient unavailable: baking soda",
                        self.last_signals.clone(),
                    ));
                }
                state.set_param("dry_mix", "flour+sugar");
                Ok(())
            }
            WET_MIX => {
                state.set_param("wet_mix", "eggs+milk");
                Ok(())
            }
            COMBINE => {
                state.set_param("batter", "combined");
                state.note("batter committed");
                Ok(())
            }
            LEAVEN => Ok(()),
            BAKE => {
                let unrecoverable = matches!(self.soda, SodaAvailability::MissingLate)
                    && state.param("leavening_agent").is_none()
                    && state.param("recovery").is_none();
                if unrecoverable {
                    self.last_signals.push(Signal::new(
                        "ingredient_missing",
                        SignalValue::text("baking_soda"),
                    ));
                    state.set_capability("leavening", 0);
                    state.note("rise check failed: no leavening agent was ever incorporated");
                    return Err(Observation::new(
                        address.clone(),
                        "required ingredient unavailable: baking soda (discovered after batter committed)",
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
            "substitute-yeast" => {
                if state.reached(&addr(COMBINE)) {
                    return Err("substitution window expired: batter already committed".into());
                }
                state.set_param("leavening_agent", "yeast");
                state.set_capability("leavening", 1);
                state.note("recipe altered: yeast substituted; proof time extended");
                Ok(())
            }
            "constrained-proceed" => {
                state.set_param("recovery", "constrained");
                state.note("constrained recovery: recipe frozen; dense bake accepted");
                Ok(())
            }
            other => Err(format!("baker does not implement remedy `{other}`").into()),
        }
    }

    fn recent_signals(&self) -> &[Signal] {
        &self.last_signals
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub const COMPLETE: &str = "Client.llm.complete";
pub const SUMMARIZE: &str = "Client.llm.summarize";

pub fn provider_spec() -> ComponentSpec {
    validated(ComponentSpec {
        name: "Client".into(),
        capabilities: vec![("generation".into(), 3)],
        failure_modes: vec![FailureMode {
            id: "rate-limited".into(),
            class: FailureClass::new("capacity/rate-limit"),
            summary: "provider request rejected".into(),
            patterns: vec![
                SignalPattern::exact("http_status", SignalValue::Int(429), 0.6),
                SignalPattern::any("quota_header", 0.38),
                SignalPattern::any("provider_outage", 0.02),
            ],
            permitted: vec![RemedyId::new("fallback-provider-b")],
            verification: Verification::CapabilityAtLeast {
                capability: "generation".into(),
                tier: 3,
            },
            verification_desc: "provider B accepted an equivalent request".into(),
            min_confidence: 0.9,
        }],
        remedies: vec![RemedySpec {
            id: RemedyId::new("fallback-provider-b"),
            description: "switch to provider B".into(),
            kind: RemedyKind::Fallback {
                to: "provider-b".into(),
            },
            precedence: 1,
            constraints: vec![Constraint::CapabilityAtLeast {
                capability: "generation".into(),
                tier: 3,
            }],
        }],
    })
}

pub struct ProviderWorld {
    /// Whether provider B is currently healthy.
    provider_b_ok: bool,
    active: u8,
    last_signals: Vec<Signal>,
}

impl ProviderWorld {
    pub fn new(provider_b_ok: bool) -> Self {
        Self {
            provider_b_ok,
            active: 0,
            last_signals: Vec::new(),
        }
    }
}

impl AppWorld for ProviderWorld {
    fn execute(
        &mut self,
        address: &OpAddress,
        state: &mut ExecutionState,
    ) -> Result<(), Observation> {
        self.last_signals.clear();
        match address.as_str() {
            COMPLETE => {
                let (status, tier) = match self.active {
                    0 => (429u16, 0u32),
                    1 if self.provider_b_ok => (200, 4),
                    _ => (429, 0),
                };
                if status == 429 {
                    self.last_signals
                        .push(Signal::new("http_status", SignalValue::Int(429)));
                    self.last_signals
                        .push(Signal::new("quota_header", SignalValue::text("present")));
                    state.set_capability("generation", 0);
                    return Err(Observation::new(
                        address.clone(),
                        "provider request rejected",
                        self.last_signals.clone(),
                    ));
                }
                state.set_capability("generation", tier);
                state.set_param("completion", "ok");
                Ok(())
            }
            SUMMARIZE => {
                state.set_param("summary", "done");
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
            "fallback-provider-b" => {
                self.active = 1;
                state.note("switched active provider to B");
                Ok(())
            }
            other => Err(format!("client does not implement remedy `{other}`").into()),
        }
    }

    fn recent_signals(&self) -> &[Signal] {
        &self.last_signals
    }
}

// ---------------------------------------------------------------------------
// Scenario driver
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct StepRecord {
    pub address: String,
    /// completed | recovered | escalated | gated
    pub outcome: String,
    pub report: Option<FailureReport>,
    pub gate_directive: Option<String>,
}

/// Everything a scenario run produced: per-step outcomes, final parameters,
/// state notes, and the knowledge learned.
#[derive(Debug, Default)]
pub struct ScenarioOutput {
    pub steps: Vec<StepRecord>,
    pub params: BTreeMap<String, String>,
    pub notes: Vec<String>,
    pub kb: KnowledgeBase,
}

impl ScenarioOutput {
    pub fn step(&self, address: &str) -> Option<&StepRecord> {
        self.steps.iter().find(|s| s.address == address)
    }
}

impl fmt::Display for ScenarioOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for step in &self.steps {
            writeln!(f, "{:<46} {}", step.address, step.outcome)?;
            if let Some(directive) = &step.gate_directive {
                writeln!(f, "    gate: {directive}")?;
            }
            if let Some(report) = &step.report {
                for line in report.to_string().lines() {
                    writeln!(f, "    {line}")?;
                }
            }
        }
        if !self.kb.is_empty() {
            writeln!(f, "Knowledge:")?;
            for record in self.kb.to_records() {
                writeln!(f, "  {record}")?;
            }
        }
        Ok(())
    }
}

fn drive(
    enforcer: &mut Enforcer,
    world: &mut dyn AppWorld,
    state: &mut ExecutionState,
    out: &mut ScenarioOutput,
) {
    for step in state.addresses() {
        let record = match enforcer.run_step(world, state, &step) {
            Ok(StepOutcome::Completed) => StepRecord {
                address: step.to_string(),
                outcome: "completed".into(),
                report: None,
                gate_directive: None,
            },
            Ok(StepOutcome::Recovered(report)) => StepRecord {
                address: step.to_string(),
                outcome: "recovered".into(),
                report: Some(*report),
                gate_directive: None,
            },
            Ok(StepOutcome::Escalated(report)) => StepRecord {
                address: step.to_string(),
                outcome: "escalated".into(),
                report: Some(*report),
                gate_directive: None,
            },
            Err(violation) => StepRecord {
                address: step.to_string(),
                outcome: "gated".into(),
                report: None,
                gate_directive: Some(violation.directive),
            },
        };
        out.steps.push(record);
    }
    out.params = state.params().clone();
    out.notes = state.notes().to_vec();
    out.kb = enforcer.engine_ref().knowledge().clone();
}

/// Baker scenario. `late = false`: the missing baking soda is caught at the
/// dry-mix step, while the substitution window is still open. `late = true`:
/// it is only discovered at the oven, after the batter is committed.
pub fn run_baker(late: bool) -> ScenarioOutput {
    let mut enforcer = Enforcer::new(DeFail::new(baker_spec()));
    let mut world = BakerWorld::new(if late {
        SodaAvailability::MissingLate
    } else {
        SodaAvailability::MissingEarly
    });
    let mut state = ExecutionState::new(baker_plan());
    enforcer.engine_ref().seed_capabilities(&mut state);
    let mut out = ScenarioOutput::default();
    drive(&mut enforcer, &mut world, &mut state, &mut out);
    out
}

/// Provider scenario. `provider_b_ok = true`: the fallback satisfies the
/// capability constraint and recovers. `false`: provider B is degraded too,
/// verification fails, and the failure escalates while the gate blocks the
/// downstream step.
pub fn run_provider(provider_b_ok: bool) -> ScenarioOutput {
    let mut enforcer = Enforcer::new(DeFail::new(provider_spec()));
    let mut world = ProviderWorld::new(provider_b_ok);
    let mut state = ExecutionState::new(vec![
        PlanStep::new(addr(COMPLETE), "call the LLM provider"),
        PlanStep::new(addr(SUMMARIZE), "summarise the completion"),
    ]);
    enforcer.engine_ref().seed_capabilities(&mut state);
    let mut out = ScenarioOutput::default();
    drive(&mut enforcer, &mut world, &mut state, &mut out);
    out
}
