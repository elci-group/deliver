# DEFAIL Closing Audit — 2026-09-10

Scope: (A) gaps in defail itself after the SOTA roadmap execution; (B) gaps in the uni
tooling that analyzed/revised the project. Method: fresh-eyes adversarial audit with
compiled probes, then fixes; `uni analyze` + `uni revise` snapshots retained in
`/tmp/defail-uni-analysis.json` and `/tmp/defail-uni-revise*.json` (also
`.uni/snapshot-v1.json`, `.uni/revise-journal.jsonl`).

## A. Defail findings — all 12 dispositioned

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| F1 | major | KB load of `u32::MAX` counts → next `learn()` panics (debug) / wraps to 0 (release), corrupting recommendations | **Fixed** — saturating arithmetic + load-time rejection of uncountable counts |
| F2 | major | Declaration validation was opt-in; `DeFail::new` accepted specs that poison classification (e.g. negative `min_confidence` matches everything) | **Fixed** — `DeFail::try_new` validates; `new` skips invalid modes with a `DeclarationSkipped` trace event (no panic); `classify` defense-in-depth |
| F3 | minor | `validate()` accepted weight `0.0` which inference silently drops | **Fixed** — reject `<= 0.0` |
| F4 | minor | Gate reported duplicate prerequisites; docs overclaimed "transitive closure" | **Fixed** — dedup + docs reworded to "plan-order predecessors + direct requires" |
| F5 | nit | Attempt-cap semantics undocumented | **Fixed** — doc lines on `max_attempts` |
| F6 | minor | v1 file whose first record is literally `defail-kb v2` silently loses that record | **Accepted + documented** — deterministic ambiguity, documented in code/README/ARCHITECTURE; test pins behavior |
| F7 | minor | `str::lines` strips raw trailing `\r` on hand-edited banks | **Accepted + documented** — self-written banks always escape `\r` |
| F8 | major | The documented PATH-avoidance fallback (`Backend::CpMkdir`) itself exec'd `mkdir` from PATH | **Fixed** — `std::fs::create_dir_all`; only `Backend::Bank` execs anything, and it is explicit opt-in |
| F9 | major | Staging names pid+seq predictable → collision-DoS by writer in destination dir | **Accepted + scoped** — determinism constraint forbids RNG; documented as out of scope for the single-threaded embedded threat model. Symlink surface itself is closed (`create_new` + rename-replaces) |
| F10 | minor | Post-rename fsync failure reported as save failure; zero-length bank loaded silently | **Fixed** — `StoreError::PersistedButUnconfirmed`; `LoadReport::empty_file` |
| F11 | minor | Doc/behavior mismatches (BadWeight wording, gate prerequisites claim) | **Fixed** |
| F12 | minor | Bank test tautologically passes when `bank` absent; missing negative tests | **Fixed** — explicit skip + 8 new tests |

Post-fix state: **82 tests green** (was 28 at restore), `cargo clippy -- -D warnings`
clean, `deliver --spec deliver.toml --strict` **18/18**, zero deps, no unsafe, no
time/random/thread APIs in `src/`.

## B. Uni tooling gaps (tools that touched defail)

Snapshot: 14/14 tools executed, 13 valid; overall score 73/C, **provisional** (7/17
tools graded); integrity **degraded**. Defects, in severity order:

1. **amber — total output failure (both runs).** `analyze` and `revise` both failed with
   "EOF while parsing a value at line 1 column 0; stdout began: ''" — the analyzer
   produced no output on a valid zero-dependency Cargo project (stderr showed "No
   dependencies found. Is this a Rust project?"). Consequence: no dependency analysis,
   and uni's overall score was computed with a missing weight. Amber should emit valid
   empty JSON for dep-free projects.
2. **lwoodz — wrong scan root + stale revise command + poka crash.** Its `analyze`
   reported "233 deps / 231,626 eligible files / header coverage 4/500" — it scanned the
   home directory, not `/home/sal/defail` (a zero-dependency crate), yielding 34 spurious
   license warnings and grade F. In `revise`, `lwoodz remedy` was **unavailable**:
   "installed lwoodz does not appear to support `remedy`" — uni's remediation command is
   stale against the installed lwoodz. Additionally lwoodz's poka generator crashed
   (poka-core index.rs:99 panic, external generator exit 2), which cascaded: poka failed
   to materialize `traci.toml` for traci as well. Three distinct defects: root detection,
   command freshness, error handling.
3. **traci — config starvation + false-positive-heavy findings.** `traci.toml` missing
   (poka crash above); 137 diagnostics, 72 "critical", dominated by TRC007 "panic path
   cannot be reconstructed" on idiomatic `unwrap` in *examples and test helpers* — noise
   for a crate whose `src/` has zero reachable panics (audit-verified). Its remediation
   was correctly risk-tiered (`ai_generated`, complexity 80/100, requires
   `--confirm-source-rewrite --confirm-ai-patch`).
4. **ferret — false positive on forbidden-token scan.** Flagged "TODO/FIXME left in
   deliver.toml" — the matches are the `forbid_regex` *literals* in the gate spec. A
   token scanner should exempt regex/string literals or gate-spec files.
5. **isopod — inconsistent coverage + N/A control mapping.** `analyze` assessed 3/14
   controls; the `revise` diagnostic pass reported 0/14 — same project, minutes apart.
   Controls like "Outsourced development" fail a solo first-party project. The `harden`
   remediation itself behaved correctly: new-files-only (`SECURITY.md`,
   `.github/dependabot.yml`), applied, no rollback, status honestly "unchanged".
6. **vamos — config discovery mismatch.** Skipped: expects `.vamos/suite.toml`; the
   project's convention (shared across this workspace) ships `vamos.toml` at root.
   vamos should read the root manifest or say so.
7. **uni scoring integrity.** Overall 73/C is computed over 7 graded tools with weights
   summing to 7/17 and marked provisional — fine as a snapshot, but the report UI
   surfaces it without enough prominence that amber's failure silently removed a weight.

### Revise dispositions (P3.3)

- **isopod harden — MERGED** (new files only; verified no source mutation; gates re-run green).
- **lwoodz remedy — NOT RUN** (unavailable: stale uni command vs installed tool).
- **traci / fract AI patches — REJECTED**: model-generated rewrites at complexity 80/100
  against a deterministic, zero-dependency crate whose flagged findings are largely
  false positives (TRC007 on example unwraps; fract already graded A- with every module
  within entropy budget). Risk/reward unacceptable; deterministic human-directed fixes
  in section A supersede them.

## Verdict

Defail: SOTA claim is defensible **within its stated scope** after F1–F12; residual
accepted gaps (F6, F7, F9) are documented scoping decisions, not unknown defects.
Uni: revision loop completed honestly (dry-run → apply → per-remediation verification),
but amber, lwoodz, and ferret require fixes before their uni scores/finding counts can
be trusted on dep-free projects.
