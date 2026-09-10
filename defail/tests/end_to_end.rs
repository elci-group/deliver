//! End-to-end scenarios: the whole DEFAIL pipeline through the reference
//! Baker and Provider worlds.

use defail::address::OpAddress;
use defail::demo::{
    baker_plan, baker_spec, provider_spec, run_baker, run_provider, BakerWorld, ProviderWorld,
    SodaAvailability, BAKE, COMPLETE, DRY_MIX, SUMMARIZE,
};
use defail::enforce::{Enforcer, GateReason};
use defail::engine::{DeFail, StepOutcome};
use defail::knowledge::{ContextSig, KnowledgeBase};
use defail::report::Disposition;
use defail::signal::{Observation, Signal, SignalValue};
use defail::state::{ExecutionState, PlanStep};
use defail::trace::{RejectStage, RemedySource, TraceEvent, TraceSink};
use defail::{FailureClass, RemedyId};

#[test]
fn baker_early_failure_substitutes_while_window_open() {
    let out = run_baker(false);

    let dry_mix = out.step(DRY_MIX).unwrap();
    assert_eq!(dry_mix.outcome, "recovered");
    let report = dry_mix.report.as_ref().unwrap();
    assert_eq!(report.disposition, Disposition::Recovered);
    assert_eq!(report.class.as_str(), "resource/ingredient-missing");
    assert_eq!(report.confidence, 1.0);
    assert!(report.resolution.contains("yeast"));
    assert!(report.constraint.contains("substitution window"));
    assert!(out.params.get("leavening_agent").unwrap().contains("yeast"));
    assert!(out.notes.iter().any(|n| n.contains("yeast substituted")));

    // The whole recipe then completes, including the bake.
    assert_eq!(out.step(BAKE).unwrap().outcome, "completed");
    assert_eq!(out.params.get("cake"), Some(&"risen".to_string()));
}

#[test]
fn baker_late_failure_frozen_recipe_constrained_proceed() {
    let out = run_baker(true);

    // Early steps sail through; the absence is only caught at the oven.
    assert_eq!(out.step(DRY_MIX).unwrap().outcome, "completed");

    let bake = out.step(BAKE).unwrap();
    assert_eq!(bake.outcome, "recovered");
    let report = bake.report.as_ref().unwrap();
    assert_eq!(report.disposition, Disposition::Recovered);
    // Substitution was proposed first and rejected: the window had expired.
    assert!(report
        .rejected
        .iter()
        .any(|r| r.contains("substitution window expired")));
    assert!(report.resolution.contains("constrained recovery"));
    assert_eq!(out.params.get("recovery"), Some(&"constrained".to_string()));
    // One lesson learned, under the late context signature: the same failure
    // class in the early context maps to a different remedy (substitute-yeast).
    assert_eq!(out.kb.len(), 1);
    let (key, entry) = out.kb.entries().next().unwrap();
    assert!(key.context.0.contains("at:Baker.recipe.bake"));
    assert_eq!(entry.remedy.as_str(), "constrained-proceed");
    assert_eq!((entry.successes, entry.failures), (1, 0));
}

#[test]
fn provider_report_matches_the_structured_format() {
    let out = run_provider(true);
    let step = out.step(COMPLETE).unwrap();
    assert_eq!(step.outcome, "recovered");
    let report = step.report.as_ref().unwrap();
    let text = report.to_string();

    assert!(text.starts_with("Failure: provider request rejected\n"));
    assert!(text.contains("Class: capacity/rate-limit\n"));
    assert!(text.contains("Evidence: http_status=429 + quota_header=present\n"));
    assert!(text.contains("Confidence: 0.98\n"));
    assert!(text.contains("Resolution: switch to provider B\n"));
    assert!(text.contains("Constraint: preserve capability generation at tier >= 3\n"));
    assert!(text.contains("Verification: provider B accepted an equivalent request"));
    assert!(text.contains("Disposition: recovered"));
    assert_eq!(report.attempts, 1);
    assert!(out.step(SUMMARIZE).unwrap().outcome == "completed");
}

#[test]
fn provider_degraded_escalates_and_gate_blocks_downstream() {
    let out = run_provider(false);

    let complete = out.step(COMPLETE).unwrap();
    assert_eq!(complete.outcome, "escalated");
    let report = complete.report.as_ref().unwrap();
    assert_eq!(report.disposition, Disposition::Escalated);
    assert_eq!(report.attempts, 1);
    // The fallback ran but could not satisfy recovery; no fix was invented.
    assert!(report.resolution.contains("escalated after 1 attempt"));

    // ENFORCE: the downstream operation is refused until a valid recovery
    // state exists.
    let summarize = out.step(SUMMARIZE).unwrap();
    assert_eq!(summarize.outcome, "gated");
    assert!(summarize
        .gate_directive
        .as_ref()
        .unwrap()
        .contains("recovery state"));
}

#[test]
fn same_failure_gets_the_same_response() {
    let mut responses = std::collections::BTreeSet::new();
    for _ in 0..100 {
        let out = run_provider(true);
        let step = out.step(COMPLETE).unwrap();
        let report = step.report.as_ref().unwrap();
        responses.insert((
            step.outcome.clone(),
            report.resolution.clone(),
            report.disposition,
            report.attempts,
        ));
    }
    assert_eq!(responses.len(), 1, "determinism: one response, not 100");
}

#[test]
fn knowledge_base_routes_repeat_failures() {
    let spec = provider_spec();
    let mut kb = KnowledgeBase::new();
    let op = OpAddress::new(COMPLETE).unwrap();
    let mut attempts_seen = Vec::new();

    for _ in 0..2 {
        let mut engine = DeFail::new(spec.clone()).with_knowledge(kb.clone());
        let mut world = defail::demo::ProviderWorld::new(true);
        let mut state = ExecutionState::new(vec![PlanStep::new(op.clone(), "call provider")]);
        engine.seed_capabilities(&mut state);
        let report = match engine.run_step(&mut world, &mut state, &op) {
            StepOutcome::Recovered(report) => *report,
            other => panic!("expected recovery, got {other:?}"),
        };
        attempts_seen.push(report.attempts);
        kb = engine.knowledge().clone();
    }

    assert_eq!(attempts_seen, vec![1, 1]);
    let entries: Vec<_> = kb.entries().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.successes, 2);
    assert_eq!(entries[0].1.failures, 0);
}

#[test]
fn unclassified_failure_escalates_without_guessing() {
    let mut engine = DeFail::new(baker_spec());
    let mut world = BakerWorld::new(SodaAvailability::Present);
    let at = OpAddress::new(DRY_MIX).unwrap();
    let mut state = ExecutionState::new(vec![PlanStep::new(at.clone(), "dry mix")]);
    engine.seed_capabilities(&mut state);

    let obs = Observation::new(
        at,
        "mysterious condition",
        vec![Signal::new("unexplainable", SignalValue::int(1))],
    );
    let report = engine.resolve(&mut world, &mut state, obs);

    assert_eq!(report.disposition, Disposition::Escalated);
    assert_eq!(report.class.as_str(), "unclassified");
    assert_eq!(report.attempts, 0);
    assert!(report.constraint.contains("refused to diagnose"));
    assert!(engine.knowledge().is_empty());
}

#[test]
fn enforcer_requires_plan_order() {
    let state = ExecutionState::new(baker_plan());
    let enforcer = Enforcer::new(DeFail::new(baker_spec()));
    let violation = enforcer
        .request(&state, &OpAddress::new(BAKE).unwrap())
        .unwrap_err();
    assert!(matches!(
        violation.reason,
        GateReason::PrerequisitesIncomplete(_)
    ));
}

#[test]
fn save_kb_rejects_paths_escaping_the_working_directory() {
    let dir = cli_workdir("escape");
    // Lexical `..` above the cwd must be refused with exit code 2.
    for bad in ["../outside", "sub/../../outside", "..", "/definitely/not/here/defail-kb"] {
        let out = defail_bin().current_dir(&dir).args(["demo", "provider", "--save-kb", bad]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "path {bad:?} must exit 2");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("escapes") || stderr.contains("USAGE"), "got {stderr}");
    }
}

#[test]
fn save_kb_rejects_an_empty_path() {
    let dir = cli_workdir("empty-path");
    let out = defail_bin()
        .current_dir(&dir)
        .args(["demo", "provider", "--save-kb", ""])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("empty"));
}

#[test]
fn save_kb_accepts_paths_within_the_working_directory() {
    let dir = cli_workdir("within");
    let out = defail_bin()
        .current_dir(&dir)
        .args(["demo", "provider", "--save-kb", "sub/../kb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(dir.join("kb")).unwrap();
    assert!(text.starts_with("defail-kb v2\n"));
    assert!(text.contains("capacity/rate-limit"));
}

fn defail_bin() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_defail"))
}

// ---------------------------------------------------------------------------
// Observability (Phase 1): event trace + JSON export
// ---------------------------------------------------------------------------

#[test]
fn baker_late_emits_the_pipeline_in_order_and_the_report_carries_it() {
    let mut enforcer = Enforcer::new(DeFail::new(baker_spec()));
    let mut world = BakerWorld::new(SodaAvailability::MissingLate);
    let mut state = ExecutionState::new(baker_plan());
    enforcer.engine_ref().seed_capabilities(&mut state);

    let mut report = None;
    for step in state.addresses() {
        if let Ok(StepOutcome::Recovered(found)) = enforcer.run_step(&mut world, &mut state, &step)
        {
            report = Some(*found);
        }
    }
    let report = report.expect("the late bake must recover via constrained proceed");

    let bake = OpAddress::new(BAKE).unwrap();
    let class = FailureClass::new("resource/ingredient-missing");
    let yeast = RemedyId::new("substitute-yeast");
    let constrained = RemedyId::new("constrained-proceed");
    // The exact decision sequence of the late-baker pipeline: classify,
    // reject the expired substitution window, then select → apply → resume →
    // verify → learn the constrained proceed.
    let expected = vec![
        TraceEvent::Classified {
            address: bake.clone(),
            class: class.clone(),
            confidence: 0.6,
            evidence_count: 1,
        },
        TraceEvent::RemedyRejected {
            address: bake.clone(),
            remedy: yeast,
            stage: RejectStage::PolicyPre,
        },
        TraceEvent::RemedySelected {
            address: bake.clone(),
            remedy: constrained.clone(),
            source: RemedySource::Declared,
        },
        TraceEvent::RemedyApplied {
            address: bake.clone(),
            remedy: constrained.clone(),
        },
        TraceEvent::Resumed {
            address: bake.clone(),
            ok: true,
        },
        TraceEvent::Verified {
            address: bake.clone(),
            remedy: constrained.clone(),
            ok: true,
        },
        TraceEvent::Learned {
            address: bake.clone(),
            class,
            remedy: constrained,
            positive: true,
        },
    ];
    assert_eq!(enforcer.engine_ref().trace(), expected.as_slice());
    // The report carries exactly the events of its own resolution.
    assert_eq!(report.trace, expected);
}

#[test]
fn provider_degraded_trace_escalates_then_the_gate_denies_downstream() {
    let mut enforcer = Enforcer::new(DeFail::new(provider_spec()));
    let mut world = ProviderWorld::new(false);
    let mut state = ExecutionState::new(vec![
        PlanStep::new(OpAddress::new(COMPLETE).unwrap(), "call the LLM provider"),
        PlanStep::new(OpAddress::new(SUMMARIZE).unwrap(), "summarise the completion"),
    ]);
    enforcer.engine_ref().seed_capabilities(&mut state);
    for step in state.addresses() {
        let _ = enforcer.run_step(&mut world, &mut state, &step);
    }

    let complete = OpAddress::new(COMPLETE).unwrap();
    let summarize = OpAddress::new(SUMMARIZE).unwrap();
    let class = FailureClass::new("capacity/rate-limit");
    let fallback = RemedyId::new("fallback-provider-b");
    let expected = vec![
        TraceEvent::Classified {
            address: complete.clone(),
            class: class.clone(),
            confidence: 0.98,
            evidence_count: 2,
        },
        TraceEvent::RemedySelected {
            address: complete.clone(),
            remedy: fallback.clone(),
            source: RemedySource::Declared,
        },
        TraceEvent::RemedyApplied {
            address: complete.clone(),
            remedy: fallback.clone(),
        },
        TraceEvent::Resumed {
            address: complete.clone(),
            ok: false,
        },
        TraceEvent::Learned {
            address: complete.clone(),
            class: class.clone(),
            remedy: fallback.clone(),
            positive: false,
        },
        TraceEvent::RemedyRejected {
            address: complete.clone(),
            remedy: fallback.clone(),
            stage: RejectStage::ResumeFailed,
        },
        TraceEvent::EscalatedExhausted {
            address: complete.clone(),
            attempts: 1,
            rejected: 1,
        },
        TraceEvent::GateDenied {
            address: summarize,
            reason: format!("blocked by unresolved failure at {complete}"),
        },
    ];
    assert_eq!(enforcer.engine_ref().trace(), expected.as_slice());
}

#[test]
fn attached_sink_receives_the_same_events_as_the_engine_trace() {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Shared(Rc<RefCell<Vec<TraceEvent>>>);
    impl TraceSink for Shared {
        fn on_event(&mut self, event: &TraceEvent) {
            self.0.borrow_mut().push(event.clone());
        }
    }

    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut engine = DeFail::new(provider_spec()).with_sink(Shared(seen.clone()));
    let mut world = ProviderWorld::new(true);
    let op = OpAddress::new(COMPLETE).unwrap();
    let mut state = ExecutionState::new(vec![PlanStep::new(op.clone(), "call provider")]);
    engine.seed_capabilities(&mut state);
    match engine.run_step(&mut world, &mut state, &op) {
        StepOutcome::Recovered(_) => {}
        other => panic!("expected recovery, got {other:?}"),
    }
    assert!(!seen.borrow().is_empty());
    assert_eq!(*seen.borrow(), engine.trace());
}

#[test]
fn report_json_is_a_versioned_golden_export() {
    let out = run_provider(true);
    let report = out.step(COMPLETE).unwrap().report.clone().unwrap();
    assert_eq!(
        report.to_json(),
        "{\"schema\":\"defail-report/1\",\
         \"failure\":\"provider request rejected\",\
         \"class\":\"capacity/rate-limit\",\
         \"evidence\":[\"http_status=429\",\"quota_header=present\"],\
         \"confidence\":0.98,\
         \"resolution\":\"switch to provider B\",\
         \"constraint\":\"preserve capability generation at tier >= 3\",\
         \"verification\":\"provider B accepted an equivalent request — held; Client.llm.complete resumed\",\
         \"disposition\":\"recovered\",\
         \"attempts\":1,\
         \"rejected\":[],\
         \"trace\":[\
         {\"event\":\"classified\",\"address\":\"Client.llm.complete\",\"class\":\"capacity/rate-limit\",\"confidence\":0.98,\"evidence_count\":2},\
         {\"event\":\"remedy_selected\",\"address\":\"Client.llm.complete\",\"remedy\":\"fallback-provider-b\",\"source\":\"declared\"},\
         {\"event\":\"remedy_applied\",\"address\":\"Client.llm.complete\",\"remedy\":\"fallback-provider-b\"},\
         {\"event\":\"resumed\",\"address\":\"Client.llm.complete\",\"ok\":true},\
         {\"event\":\"verified\",\"address\":\"Client.llm.complete\",\"remedy\":\"fallback-provider-b\",\"ok\":true},\
         {\"event\":\"learned\",\"address\":\"Client.llm.complete\",\"class\":\"capacity/rate-limit\",\"remedy\":\"fallback-provider-b\",\"positive\":true}\
         ]}"
    );
}

#[test]
fn knowledge_base_json_is_versioned_ordered_and_escapes() {
    let mut kb = KnowledgeBase::new();
    let key = defail::knowledge::KbKey {
        class: FailureClass::new("capacity/rate-limit"),
        context: ContextSig("after:start;at:Client.call".into()),
    };
    kb.learn(
        key,
        RemedyId::new("fallback-b"),
        "provider B accepted".into(),
        true,
    );
    kb.learn(
        defail::knowledge::KbKey {
            class: FailureClass::new("we|rd/class\n\"quoted\""),
            context: ContextSig("after:start;at:Client.call".into()),
        },
        RemedyId::new("retry"),
        "retry the request".into(),
        false,
    );
    // Entry order is the deterministic BTreeMap order; adversarial text is
    // escaped inside the envelope.
    assert_eq!(
        kb.to_json(),
        "{\"schema\":\"defail-kb/2\",\"entries\":[\
         {\"class\":\"capacity/rate-limit\",\"context\":\"after:start;at:Client.call\",\
         \"remedy\":\"fallback-b\",\"successes\":1,\"failures\":0,\
         \"verification\":\"provider B accepted\",\"alt_remedies\":[]},\
         {\"class\":\"we|rd/class\\n\\\"quoted\\\"\",\"context\":\"after:start;at:Client.call\",\
         \"remedy\":\"retry\",\"successes\":0,\"failures\":1,\
         \"verification\":\"retry the request\",\"alt_remedies\":[]}\
         ]}"
    );
}

#[test]
fn unclassified_failure_trace_carries_no_signal_payloads() {
    let mut engine = DeFail::new(baker_spec());
    let mut world = BakerWorld::new(SodaAvailability::Present);
    let at = OpAddress::new(DRY_MIX).unwrap();
    let mut state = ExecutionState::new(vec![PlanStep::new(at.clone(), "dry mix")]);
    engine.seed_capabilities(&mut state);

    let obs = Observation::new(
        at.clone(),
        "mysterious condition",
        vec![Signal::new("unexplainable", SignalValue::int(1))],
    );
    let report = engine.resolve(&mut world, &mut state, obs);

    assert_eq!(
        engine.trace(),
        &[TraceEvent::EscalatedUnclassified {
            address: at,
            evidence_count: 1,
        }]
    );
    // Risk R2: the trace never carries the signal payload, only a count.
    // (The report's own `evidence` field is the attributable diagnostic
    // channel and is unchanged.)
    for event in &report.trace {
        let json = event.to_json();
        assert!(!json.contains("unexplainable"), "trace leaked payload: {json}");
        assert!(json.contains("\"evidence_count\":1"));
    }
}

#[test]
fn demo_json_flag_prints_the_human_report_then_json() {
    let out = defail_bin()
        .args(["demo", "baker", "--late", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Disposition: recovered"));
    assert!(stdout.contains("\"schema\":\"defail-report/1\""));
    assert!(stdout.contains("\"event\":\"classified\""));
    assert!(stdout.contains("\"stage\":\"policy_pre\""));
    assert!(stdout.contains("\"step\":\"Baker.recipe.bake\""));
    // Human report first, JSON after; exit code unchanged.
    let human_at = stdout.find("Disposition: recovered").unwrap();
    let json_at = stdout.find("\"schema\":\"defail-report/1\"").unwrap();
    assert!(human_at < json_at);
}

#[test]
fn kb_show_json_prints_the_versioned_export() {
    let dir = cli_workdir("kb-json");
    let out = defail_bin()
        .current_dir(&dir)
        .args(["demo", "provider", "--save-kb", "kb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));

    let out = defail_bin()
        .current_dir(&dir)
        .args(["kb", "show", "--json", "kb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"schema\":\"defail-kb/2\""));
    assert!(stdout.contains("\"class\":\"capacity/rate-limit\""));
    assert!(stdout.contains("\"remedy\":\"fallback-provider-b\""));
    // The v2 line-record header must not leak into the JSON export.
    assert!(!stdout.lines().any(|line| line == "defail-kb v2"));
}

fn cli_workdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("defail-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    dir
}
