# DEFAIL architecture

Date: 2026-09-10 · Applies to: v0.2.0 (pre-1.0)

This document is the contract layer. It documents the pipeline, the module
map, the determinism and threading contracts, the persistence formats, the
observability model, and the gate semantics. Behavior documented here —
together with the names the crate root re-exports — is the stable surface
(see the API stability policy in README.md); module internals are free to
evolve as long as these contracts hold.

## The pipeline

Every failure travels the same deterministic resolution pipeline:

```text
execution → detection → classification → causal inference
         → remediation selection → policy validation
         → remediation execution → verification → resume / escalate
```

Concretely, in `DeFail::run_step` / `DeFail::resolve` (src/engine.rs):

1. **Execution** — the host's `AppWorld::execute` runs the step. `Err(obs)`
   is the detection boundary: an `Observation` with diagnostic signals.
2. **Classification** — `inference::classify` matches the observation
   against the declared failure modes. No match at the declared minimum
   confidence escalates as `unclassified` immediately; nothing is guessed.
3. **Causal inference** — the matched mode's pattern weights yield a
   confidence and an evidence list (signal renderings only).
4. **Remediation selection** — deterministic order: a proven knowledge-base
   recommendation first (only while it succeeds more than it fails), then
   declared precedence, then remedy id. Never anything else.
5. **Policy validation (pre)** — `Policy::validate_pre` checks substitution
   windows and state shape before any attempt is consumed.
6. **Remediation execution** — the host's `AppWorld::apply_remedy` performs
   the permitted remedy; a refusal is a typed `RemedyError` and counts as a
   rejected candidate.
7. **Resume** — the operation itself must now succeed; a failed resume
   rejects the candidate.
8. **Policy validation (post)** — capability invariants must hold in the
   resumed state.
9. **Verification** — the declared `Verification` predicate must hold.
10. **Learn & report** — the outcome is learned into the `KnowledgeBase`
    (remedy-id divergence recorded as `alt_remedies`, never silently
    attributed) and a `FailureReport` is produced. If candidates are
    exhausted, the report escalates.

The gate (`Enforcer::run_step`) wraps step 1: an operation may run only when
the gate permits it (see *Gate semantics* below).

## Module map

| Module | Role |
|---|---|
| `address` | `OpAddress`: addressable points of the execution graph; parsing and navigation (`component`, `parent`, `join`) |
| `signal` | `Signal`/`SignalValue` observations, `SignalPattern` declarations, `Observation` |
| `declare` | embedded declarations: `ComponentSpec`, `FailureMode`, `RemedySpec`, `Constraint`, `Verification`; construction-time validation (`DeclarationError`) |
| `state` | `ExecutionState`: the addressable model of application state (statuses, capabilities, params, notes) plus `PlanStep` |
| `inference` | deterministic classification and causal confidence ranking |
| `policy` | pre/post constraint validation (`Violation`) |
| `knowledge` | the learned `failure class + context → remedy → outcome` map; record formats v1/v2; the `absorb` join |
| `store` | knowledge persistence: `bank` primary, `mkdir` fallback, atomic staging rename |
| `report` | `FailureReport` (structured, attributable output) and its `defail-report/1` JSON export |
| `trace` | `TraceEvent` decision trace, `TraceSink`/`NoopSink`/`VecSink` |
| `json` | the hand-rolled, escaping-correct JSON writer behind every export |
| `engine` | the resolution pipeline; `AppWorld` host boundary; `DeFail` |
| `enforce` | the gate: prerequisite closure, cycle detection, `GateViolation` |
| `demo` | Baker and Provider reference scenarios |

## The determinism contract

Every decision is reproducible. The mechanisms that guarantee it:

- **No time, no randomness, no threads** anywhere in `src/`. Nothing in
  inference, policy, learning, reporting, or the gate reads the clock, a
  random source, or shared mutable state. (The two sanctioned exceptions,
  both outside the decision path: staging file names embed the process id —
  see *Persistence* — and the demo CLI reads its arguments.)
- **Total orders at every tie point.** Wherever two candidates could tie,
  a documented total order decides:
  - classification: confidence desc, then evidence count desc, then summary
    asc (src/inference.rs);
  - remediation selection: knowledge first, then declared precedence asc,
    then remedy id asc (src/engine.rs);
  - knowledge conflict resolution inside `absorb`: more successes, then
    fewer failures, then remedy id (src/knowledge.rs);
  - gate cycle reporting: `requires` edges visited in declaration order
    (src/enforce.rs).
- **BTreeMap ordering.** `KnowledgeBase` entries, execution capabilities
  and params, and alternative-remedy counts all iterate in sorted key
  order, so reports, persistence, and JSON exports are byte-for-byte
  deterministic.
- **`absorb` is a join-semilattice**: commutative, associative, and
  idempotent. Banks merge in any order, grouping never matters, and
  re-merging an already-merged bank changes nothing. Property-tested over
  generated banks in tests/properties.rs.
- **Confidence arithmetic** sums declared pattern weights in declaration
  order (never observation order — signal shuffling cannot change a
  diagnosis; property-tested), and JSON renders `f32` via std's shortest
  round-trip `Display` (non-finite values become `null`).
- **Validated declarations.** NaN/negative weights, out-of-range
  `min_confidence`, and empty `permitted` lists fail loudly at construction
  time (`ComponentSpec::validated`), so inference can never be poisoned by
  a bad declaration.

## The threading contract

DEFAIL is **single-threaded by design**.

- All mutation flows through `&mut` receivers:
  `DeFail::run_step(&mut self, world: &mut dyn AppWorld, state: &mut ExecutionState, ...)`.
  There is no internal locking, no background work, and no callbacks that
  outlive the call.
- The crate makes **no `Send`/`Sync` guarantees**. `DeFail` holds a
  `Box<dyn TraceSink>`, which is neither; do not share an engine, enforcer,
  world, or state across threads. (The lone `std::sync::atomic` in the
  crate is the staging-file name counter in src/store.rs — a uniqueness
  device for file names, not shared application state.)
- `TraceSink::on_event(&mut self)` is invoked synchronously on the engine's
  stack; a sink belongs to the thread that owns the engine. Hosts that are
  multi-threaded at the application level must serialize access to each
  engine/world/state triple (one owner at a time) or shard by component.

## Persistence formats

### Knowledge bank records

Versioned, line-oriented, human-inspectable:

- **v2 (write format)** — first line is the header `defail-kb v2`; each
  following line is one record with six pipe-separated fields:
  `class|context|remedy|successes|failures|verification`, plus an optional
  seventh field carrying alternative-remedy counts as
  `remedy=successes/failures,…`. Every field is backslash-escaped
  (`\\`, `\|`, `\n`, `\r`, `\,`, `\=`), so adversarial strings round-trip
  exactly and every record is exactly one physical line. Unknown escape
  sequences and trailing backslashes are load errors with 1-based line
  numbers; a bad line aborts the load without partial mutation.
- **Duplicate keys** keep the last record (retained behavior) and are
  reported through `LoadReport::duplicates` with 1-based line numbers.
- **v1 (read compatibility)** — a file without the v2 header is read as
  six raw pipe-separated fields, no unescaping applied; the historical
  `|`-to-`/` sanitization of the verification field is preserved.

### Atomic store semantics

`KnowledgeStore::save` writes the bank so that a crash mid-save never
truncates the previously saved bank:

1. Path creation prefers the `bank` utility (`bank -p -f`); any bank
   failure deterministically falls back to `mkdir -p`. Both backends
   produce identical records; `save` reports which backend wrote.
2. The record body is staged into a *new* file in the destination
   directory, created with `create_new(true)` — a collision is an error to
   retry, never an existing file to follow or truncate, so there is no
   predictable-name symlink surface. Staging names embed the process id
   and a counter for uniqueness; they never influence record content.
3. The staging file is written and `fsync`ed, then atomically renamed onto
   the target (a symlinked target is replaced, not written through), and
   the directory is `fsync`ed after the rename.
4. A failed rename removes the staging file; if cleanup fails too, both
   failures are reported as `StoreError::CleanupFailed`.

## Observability model

- **Event trace.** Every pipeline decision is emitted in order as a
  `TraceEvent`: `Classified` / `EscalatedUnclassified`, per candidate
  `RemedySelected` / `RemedyRejected` (with the stage: `policy_pre`,
  `apply_failed`, `resume_failed`, `policy_post`, `verification`),
  `RemedyApplied`, `Resumed`, `Verified`, `Learned`, `EscalatedExhausted`,
  plus `GateDenied` from the gate. The engine records every event
  internally (`DeFail::trace()`); each `FailureReport` carries the slice
  emitted during its own resolution; hosts can stream events anywhere with
  a `TraceSink` (`NoopSink` default, `VecSink` records).
- **Security boundary (risk R2).** Events carry addresses, ids, and
  decision data only — never signal payloads or host state. The
  attributable diagnostic channel is the report's `evidence` field.
- **JSON schemas.** `FailureReport::to_json()` exports
  `"schema": "defail-report/1"` (including the full trace);
  `KnowledgeBase::to_json()` exports `"schema": "defail-kb/2"` in the same
  BTreeMap order as the record format; each `TraceEvent` renders via a
  stable snake-case `event` discriminator. All exports share the
  `json` module: field order is stable, strings are escaped
  (short escapes plus `\u00XX` for other control characters; non-ASCII
  passes through as UTF-8), and `f32` uses shortest round-trip formatting —
  output is byte-for-byte deterministic.

## Gate semantics (DEFAIL ENFORCE)

`Enforcer::request` answers one question per operation: given what has
happened, may this run? Denials are `GateViolation`s carrying a
`GateReason` and a directive:

1. **Unknown operation** — the address is not a declared plan step:
   `GateReason::UnknownOperation`.
2. **Cyclic prerequisites** — the declared `requires` graph, followed over
   its transitive closure from the requested operation, contains a cycle.
   A cycle can never complete, so instead of gating forever the gate denies
   with `GateReason::CyclicPrerequisites`, naming the cycle in traversal
   order (head not repeated). Detection is an iterative white/gray/black
   depth-first search — deep plans cannot overflow the stack — visiting
   edges in declaration order so the reported cycle is deterministic.
   Edges pointing outside the declared plan cannot form a cycle and are
   reported separately as perpetually incomplete prerequisites.
3. **Blocked by failure** — an earlier plan step, or a declared
   prerequisite, is `Failed` and its failure stands unresolved:
   `GateReason::BlockedByFailure`. The application is constrained to a
   valid recovery path before proceeding. (The failed operation itself may
   be re-attempted through the gate.)
4. **Prerequisites incomplete** — earlier steps or `requires` edges are
   not `Done`: `GateReason::PrerequisitesIncomplete`, listing what remains.

`Enforcer::run_step` requests the gate first; a denial is recorded in the
engine trace as `TraceEvent::GateDenied` and returned as `Err`, so a
refusal is observable in the same stream as every other decision.
