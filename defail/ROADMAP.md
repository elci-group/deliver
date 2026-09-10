# DEFAIL SOTA Roadmap

**Target: enterprise-grade, state-of-the-art deterministic failure-inference substrate.**
Date: 2026-09-10 · Baseline: v0.2.0 (restored from `saved-state`, 28 tests green).

## SOTA definition for this class of system

DEFAIL competes conceptually with failure-oblivious computing, circuit-breaker/resilience
frameworks, and AIOps remediation engines. SOTA for a *deterministic embedded* substrate
means five pillars, each with an objective exit criterion:

| Pillar | SOTA bar | Verified by |
|---|---|---|
| 1. Correctness & determinism | Every decision reproducible; no panics on any input; validated declarations | Property + adversarial test suite, 100× determinism property |
| 2. Durability & crash safety | KB stores are atomic (write-temp + rename + fsync); corrupt state is detected and reported precisely | Crash-consistency and corruption tests |
| 3. Observability | Every pipeline decision emits a structured, machine-readable event; reports exportable as versioned JSON | Event-sequence assertions, golden JSON tests |
| 4. Security posture | No `/tmp` staging, no predictable-name symlink surface, no silent PATH trust (only the opt-in `Backend::Bank` execs an external tool), no injection into record format | Adversarial input tests, symlink/tamper tests |
| 5. Operability & docs | Documented threading contract, architecture doc, stable API policy, enforced quality gate | `deliver` gate expanded and green |

Constraint honored throughout: **zero runtime dependencies**. Determinism and auditability
are the product; every feature must be implementable with std only.

## Phase 0 — Hardening (Pillars 1, 2, 4)

- [x] P0.1 Error-type hygiene: `std::error::Error` for `BadAddress`, `KbError`,
      `GateReason`/`GateViolation`, `Policy::Violation`; typed remedy-application error
      replacing `Result<(), String>` host boundary.
- [x] P0.2 Declaration validation: reject NaN/negative pattern weights, `min_confidence`
      outside `[0,1]`, empty `permitted` remedy lists at construction time with a typed
      `DeclarationError` — bad declarations must fail loudly, never classify silently.
- [x] P0.3 KB record format v2: versioned header (`defail-kb v2`), full field escaping so
      `|`/newlines round-trip; malformed files report line-level diagnostics; duplicate
      keys detected (last-wins is retained but reported via load diagnostics).
- [x] P0.4 Atomic store: write-temp + `fsync` + atomic `rename`; staging files created
      with `create_new(true)` (no predictable-name symlink surface); cleanup failures
      reported; crash mid-save never truncates the previous bank.
- [x] P0.5 Knowledge merge correctness: `learn` records remedy-id divergence instead of
      silently attributing all counts to the first-learned remedy.
- [x] P0.6 Path discipline: `--save-kb` rejects empty/relative-escape targets; PATH probe
      for `bank` documented and confined to explicit opt-in.

Exit: new adversarial tests (NaN weights, `|`/newline injection, duplicate keys, mid-save
kill simulation via injected writer failure, symlink staging collision) all green; existing
28 tests unchanged in behavior.

## Phase 1 — Observability (Pillar 3)

- [x] P1.1 Structured event trace: `TraceSink` trait (std-only, no-op default); engine emits
      ordered events for classify / recommend / validate / apply / resume / verify / learn /
      escalate; `FailureReport` carries the trace.
- [x] P1.2 Versioned JSON export: `report.to_json()` and `KnowledgeBase::to_json()` via a
      hand-rolled, escaping-correct emitter (no serde); schema version field on every export.
- [x] P1.3 CLI: `--json` flag on demo/kb subcommands piping the same exports.

Exit: golden-file JSON tests; event-order assertions in e2e tests; README section.

## Phase 2 — Gate & docs (Pillars 1, 5)

- [x] P2.1 Gate precondition analysis: detect cyclic `requires` graphs and report them as
      gate violations instead of hanging/undefined behavior; document transitive-closure rule.
- [x] P2.2 Documented contracts: `ARCHITECTURE.md` (pipeline, determinism contract, threading
      contract — single-threaded by design, `&mut` discipline), API stability policy in README.
- [x] P2.3 Worked examples: expand `examples/` beyond 4-line wrappers — embed DEFAIL in a
      real mini-app with custom `AppWorld`.

Exit: cycle-detection tests; docs reviewed against code; examples compile and run.

## Phase 3 — Verification & gates (all pillars)

- [x] P3.1 Property tests (std-only harness): KB `absorb` commutativity/associativity/
      idempotence; record round-trip over generated adversarial strings; classification
      stability under signal reordering.
- [x] P3.2 `deliver.toml` gate expansion: add `cargo clippy -- -D warnings`, forbid-regex
      coverage for new modules, JSON export smoke command.
- [ ] P3.3 External analysis: `uni analyze` snapshot reviewed; `uni revise --apply`
      remediations evaluated individually and either merged or documented as rejected
      with rationale.
- [x] P3.4 Closing audit (2026-09-10, findings F1–F12): KB counts saturate and
      `u32::MAX` counts are rejected at load; `DeFail::try_new` validates while
      `DeFail::new` keeps its panic-free contract (invalid modes cannot classify,
      recorded as `declaration_skipped` trace events); classify guards
      `min_confidence`; zero-weight patterns rejected; gate denials deduplicated;
      `Backend::CpMkdir` became fully PATH-free (`std::fs::create_dir_all`, no
      external processes); staging-name determinism documented as in-scope for
      reproducibility with collision-DoS by a destination-dir writer out of scope;
      new `StoreError::PersistedButUnconfirmed` and `LoadReport::empty_file`.
      API additions only: new `TraceEvent`/`StoreError` variants and a
      `LoadReport` field.

Exit: full suite + clippy + deliver gate green; uni findings dispositioned in AUDIT.md.

## Risks

- R1: Record format v2 breaks banks written by v0.2 → mitigate: v1 loader retained for
  read, v2 on write; explicit test.
- R2: Event trace could leak host data into logs → mitigate: events carry addresses and
  decision data only, never signal payloads.
- R3: Validation rejects declarations that demo code currently constructs → mitigate:
  fix demos in the same commit; constructor APIs gain `try_new` while `new` documents
  its panic-free validation contract.

- [x] P3.3 External analysis: `uni analyze` snapshot reviewed; `uni revise --apply` executed — isopod merged, lwoodz unavailable (stale command), traci/fract AI patches rejected with rationale (see AUDIT.md)
- [x] P3.4 Closing audit: 12 findings fixed, dispositions recorded in AUDIT.md
