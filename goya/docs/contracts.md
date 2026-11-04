# Goya — Contract Declarations (Wave 1)

Status: Candidate (shared between active workstreams; architecture approval
required for changes). See GOYA directive §8.3.

## GY-IR/1 — Typed IR + capability schema (`crates/goya-ir`)

- Provides: `Module { nodes: Vec<(NodeId, Node)>, root: Option<NodeId> }`,
  `Node` (Column/Row/Text/Button/Spacer), `NodeId`, `ActionId`,
  `Capability`, `parse(&str) -> Result<Module, ParseError>`.
- Invariants: node ids are unique per module; `root` must reference a
  declared node; parsing is total (never panics on user input).
- Stability: Candidate. Parser grammar may grow (nested syntax, attributes)
  but the `Module`/`Node` shape is frozen for Wave 1.

## GY-STATE/1 — Revisioned state transactions (`crates/goya-state`)

- Provides: `Store` (Clone, thread-safe), `Transaction`, `Value`
  (Int/Text/Bool), `CommitRejected`.
- Protocol: `begin()` → `set()`* → `commit(invariant)`; drop without commit =
  rollback. Single-writer enforced (`begin` returns `None` if a transaction
  is open). Committed snapshot is never mutated in place; commit swaps the
  staged copy atomically and bumps `revision()` only on success.
- Invariants: reads outside a transaction always observe the last committed
  revision; a rejected commit leaves state and revision untouched.

## GY-LAYOUT/1 — Deterministic layout (`crates/goya-layout`)

- Consumes: GY-IR/1.
- Provides: `layout(&Module, Constraints) -> Frame`, `Rect`, millipoint
  integer geometry (1 pt = 1000 mp).
- Invariants: layout is a pure function of (module, constraints) — no time,
  locale, RNG or environment dependence; identical inputs yield identical
  frames (unit-tested).

## GY-RUNTIME/1 — Event dispatch pipeline (`crates/goya-runtime`)

- Consumes: GY-IR/1, GY-STATE/1, GY-LAYOUT/1.
- Provides: `Runtime::new(module, store, constraints)`, `handle(Event) -> bool`
  (false = quit), `hit_test(x_mp, y_mp) -> Option<ActionId>`,
  `render_list() -> Vec<Primitive>`.
- Invariants: events are processed synchronously; layout is recomputed only
  when the state revision advanced; unknown actions are inert.

## Capability schema (Wave 1 form)

`Capability` enumerates the grantable rights (window, text-input, network,
filesystem, tts, motion, sensors, graphics3d). Wave 2 adds a manifest-level
grant/deny enforcement layer; Wave 1 only defines the vocabulary.

## Work-package DAG (Wave 1, completed nodes marked)

```
GY-IR-001 ✓ typed IR + parser            GY-STATE-001 ✓ transaction engine
GY-LAYOUT-001 ✓ deterministic layout     GY-RUNTIME-001 ✓ event pipeline
GY-HELLO-001 ✓ first integrated proof
GY-WAYLAND-001 (open): Wayland surface adapter, consumes GY-RUNTIME/1
GY-TEST-001 (open): conformance suite + fuzzed parser (assurance swarm)
```

## Ownership registry (Wave 1)

| Path | Owner |
|---|---|
| crates/goya-ir/** | Swarm A (Language) |
| crates/goya-state/**, crates/goya-runtime/** | Swarm B (Runtime) |
| crates/goya-layout/** | Swarm C (Presentation) |
| crates/goya-hello/** | Swarm H (Integration) |

---

## GY-CHROMA/1 — Chromatic relativism engine (`crates/goya-chroma`)

Status: Candidate (Chromatic Swarm). Extension GY-CHR-001.

- Provides: `Oklch`/`Rgb8` with sRGB↔OKLCH conversion and chroma-shrinking
  gamut mapping; `PropertyRelation` (Absolute/Delta/Scale/Polar); `Polarity`;
  `Theme`/`ThemeRole`/`Harmony`/`WcagLevel`; `resolve(&Theme, Polarity)`.
- Resolution sequence (fixed, §5): anchor → inheritance → polarity →
  harmony → constraints → gamut-mapped composited output.
- Polarity model: `polar(d)` is a signed lightness relationship whose sign
  flips with theme polarity; the grey-line reflection `L' = Lg − p(L − Lg)`
  guards role sides around `GREY_LINE = 0.6`. Brand-locked roles preserve
  hue and chroma. Polarity is **not** an RGB complement.
- Constraints: contrast is enforced on the composited sRGB pair (WCAG AA /
  AAA) by deterministic lightness search in both directions; if unreachable,
  the resolver returns `ResolveError::ContrastUnreachable` — it never fakes
  compliance.
- Invariants: `resolve` is a pure function of (theme, polarity); unit-tested
  for determinism, hue preservation across polarity, and AA compliance in
  both modes.

## GY-PROVENANCE/1 — Text provenance (`crates/goya-provenance`)

Status: Candidate (Provenance Swarm).

- Provides: `SelectionRange` (clamping, char-boundary-safe), `SourceReference`,
  `TraceId`, `DiagnosticRecord`, `DeepCopyBundle`, `DeepCopyLimits`,
  `build_bundle`, `redact`/`RedactionRecord`.
- Policy: bundles are bounded (`max_bytes`, enforced on the rendered form,
  truncating diagnostics before ever touching the selected text); secrets
  (emails, bearer/API tokens, ≥32-char hex runs) are redacted with recorded
  reasons; traces/logs can be excluded per consent flags.
- Invariants: selection never panics on out-of-range or mid-codepoint
  offsets; an oversized selection fails loudly (`SelectionTooLarge`).

## GY-CLIPBOARD/1 — Dual-mode clipboard (`crates/goya-clipboard`)

Status: Candidate (Provenance Swarm).

- Consumes: GY-PROVENANCE/1.
- Provides: `CopyMode` (Verbatim / Deep), `CopyRequest`, `ClipboardProvider`
  trait, `MemoryClipboard`, `ClipboardCapability` + `CapabilityPolicy`.
- Contract: Verbatim copies exactly the selected visible text. Deep writes
  the provenance bundle as `text/plain` plus a structured
  `application/x-goya-provenance` payload. Every write is capability-gated
  (`Verbatim` vs `DeepContext` are distinct grants) and size-limited.
- Platform note: the Linux compositor data-device backend lands with
  GY-WAYLAND-001; shortcut registration must be remappable and must not
  steal Ctrl+Shift+C from embedded terminals.

## Ownership registry additions

| Path | Owner |
|---|---|
| crates/goya-chroma/** | Chromatic Swarm |
| crates/goya-provenance/**, crates/goya-clipboard/** | Provenance Swarm |

---

# Wave 2 — GY-SON-001, GY-NSB-001, GY-RPI-001, GY-EDC-001, GY-X402-001, GY-AIM-001

Implemented by a 10-agent concurrent swarm (J1–J10). All contracts Candidate.

## GY-SONIC/1 — Sonic events & policy (`crates/goya-sonic`, Swarm J)
`SonicEvent` with geometry-revision guard, `SonicResolver` trait, `SonicPolicy`/`PolicyEngine` (token-bucket rate limiting, mono/muted/spatial flags). Stale geometry revisions are rejected, never guessed.

## GY-SPATIAL-AUDIO/1 — Geometry → acoustics (`crates/goya-spatial-audio`, Swarm J)
Equal-power stereo `stereo_gains(p)` (power ≈ 1 across the sweep), viewport-relative `pan_from_rect`, bounded inverse distance law with floor, smoothed `StereoPan` trajectory, `fold_mono` accessibility fold-down.

## GY-AUDIO-DSP/1 + GY-AUDIO-LINUX/1 (`Swarm J`)
Deterministic decaying-sine `Tap`, priority-evicting `Mixer` (lower numeric = more urgent; graceful overload), hard + smoothed `Limiter`. Linux layer is the device lifecycle contract with `NullOutput` (missing device keeps the app operational); PipeWire C binding is a future adapter.

## GY-BRIDGE-PHYSICS/1 + GY-BRIDGE/1 — NSB (`Swarm J`)
Fixed-1/240s semi-implicit spring (deterministic, dt-split-invariant), `clamp_overshoot` bites only on genuine overshoot (approach travel passes unclamped), retarget preserves position+velocity (interruption contract), reduced-motion snap, `interactive()` false the instant authoritative state exits even mid-animation. `Bridgeable` implemented for f64 and Oklch (shortest hue path).

## GY-RPI/1 — Probability field (`crates/goya-rpi`, Swarm K)
Conditional context-relative probabilities (count-based Level-1), Brier calibration, `net_value = pB − C − λW` ranking, `Preparation` valid only against matching state+geometry revisions, Level-2 learning requires enabled AND consent (default off).

## GY-SPECULATION/1 — Budgeted speculation (`crates/goya-speculation`, Swarm K)
Mesut-style budget never overruns; cache lookups valid only at matching revisions; commit returns Commit/Recompute/Reject; uncommitted speculative work dropped at commit; greedy value-per-cost scheduler skips unaffordable positive-value items without blocking.

## GY-DETERMINISM/1 + GY-EXECUTABLE/1 — EDC (`Swarm`)
D0/D1/D2/N classification with rejection of unresolved claims (a D0 claim with effects is rejected, never silently pure), versioned `ExecutionProof` (compiler/schema/policy/target), idempotent artifact registry, capability-escalation detection (artifacts cannot acquire privileges), staleness on schema/policy drift, compilation cost model.

## GY-X402/1 + GY-ECONOMICS/1 — Native x402 (`Swarm`)
Exact-integer `Amount` (never floats for money), challenge validation with asset+recipient allowlists (recipient substitution refused), lifecycle where Timeout/ReceiptLost → `Ambiguous` (never Failed — reconciliation before duplicate charges), `Allowance` with per-payment approval policy, `inherited_allowance` = MIN of ancestors (permissions narrow, never expand), immutable `IntentRecord`.

## GY-SETTLEMENT/1 — Reconciliation (`crates/goya-settlement`, Swarm)
Append-only ledger, Verified→Settled / Lost→NeedsInvestigation / DuplicateSuspected→DuplicateBlocked, payload-ref idempotency blocks duplicate authorisation, open-obligation audit, secret-free diagnostic redaction.

## GY-ARTIFACT/1 + GY-WATERMARK/1 — AIM (`Swarm L`)
Three separate identities (byte hash FNV-1a, perceptual 8×8 block-mean hash, provenance ref); `finalise` is the only constructor path (hash computed last); keyed mid-frequency 8×8 DCT embedding with per-position nonlinear keystream (wrong keys rejected — the naive key⊕position seed was GF(2)-linear and collided), majority-vote detection with confidence threshold and false-positive fail-closed.

## Ownership registry (Wave 2)
| Path | Owner |
|---|---|
| crates/goya-sonic, goya-spatial-audio, goya-audio-dsp, goya-audio-linux | Swarm J (Spatial Sonic) |
| crates/goya-bridge, goya-bridge-physics | Swarm J4 (NSB) |
| crates/goya-rpi, goya-speculation | Swarm K (Anticipatory Expression) |
| crates/goya-determinism, goya-executable | Swarm (EDC) |
| crates/goya-x402, goya-economics, goya-settlement | Swarm (x402) |
| crates/goya-artifact, goya-watermark | Swarm L (Artifact Identity) |
