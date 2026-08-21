# Deliver: Roadmap to SOTA

_Last assessed: 2026-08-21, against v0.2.0._

## Current State Assessment

**Grade: B+.** The core is small, correct, and well-tested; the gaps that
existed were mostly drift and packaging bugs rather than missing engineering,
and those have now been closed (see below). What's left to reach A-tier is
breadth (parallelism, richer output formats, extensibility), not repair.

**Strengths:**
- Clean, focused Rust codebase with a real module boundary per check kind
  (`file_check`, `command`, `directory_check`, `git_check`, `glob`, `schema`)
- Rich file-check surface: size, regex, line count, encoding, symlink
  policy, SHA-256/BLAKE3 pinning, license headers, JSON/YAML Schema, TOML/JSON
  key presence, and cross-file reference checking for several languages
- `init`/`validate`/`completions`/`schema` subcommands for spec authoring
- Dual output formats (text + JSON), colored/plain and progress-aware
- 50 tests (12 lib unit + property tests, 38 CLI-level integration tests),
  all green; clippy clean with `-D warnings`; `cargo fmt` clean
- CI matrix across stable/beta/nightly, a separate release-build matrix
  (Linux/macOS/Windows), and a Criterion benchmark job

**Fixed this pass (previously real gaps, now closed):**
- Integration tests hardcoded `/home/sal/deliver/target/debug/deliver` as the
  binary path — this passed locally by coincidence but would silently fail in
  CI or on any other machine/checkout path. Now uses
  `env!("CARGO_BIN_EXE_deliver")`.
- CI (`ci.yml`) triggered only on `push`/`pull_request` to `main`/`develop`,
  but the repository's actual branch is `master` — CI has effectively never
  run against real commits. Added `master` to the trigger branches.
- `.pre-commit-hooks.yaml` (the manifest consumers reference via
  `repo: <this repo>`) was wrapped in a `repos:`/`repo: local` block, which is
  the shape of a *consumer's* `.pre-commit-config.yaml`, not a hook-provider
  manifest — pre-commit would fail to parse it. Rewritten as a bare hook list.
- README and the man page never documented the `init`/`validate`/
  `completions` subcommands, `[[directory]]` checks, or most `FileCheck`
  fields added since the initial spec (`encoding`, `forbid_symlinks`,
  `sha256`/`blake3`, `require_license_header`, `json_schema`/`yaml_schema`,
  `require_toml_keys`/`require_json_keys`, `check_references`). Both are now
  complete, and `deliver.toml`'s own self-check greps for the subcommand
  names so this can't silently drift again.
- The man page's `.TH` version/date (`0.1.0`, "July 2026") had drifted from
  `Cargo.toml` (`0.1.2`); both files (and `VERSION`) are now kept in lockstep
  and the self-check's `cargo build`/`cargo test` gates don't catch this
  class of drift, so it's now also covered by regex checks on the docs.
  `deliver.toml`'s command gates were also missing `cargo clippy`/`cargo fmt
  --check`, which CI runs but the project's own spec didn't — added both.
- `Spec.directories` only deserialized from the TOML key `directories`, but
  every example in the codebase (README, `deliver init`'s scaffold) used
  `[[directory]]` (singular). That key was silently ignored — `deliver init`
  generated a spec whose directory check never ran. Added `alias = "directory"`.

## Shipped this pass: two new features

**Spec composition (`extends`).** A spec can set `extends = "path.toml"`,
resolved relative to the declaring file, to inherit from a parent spec. File/
command/directory checks accumulate; `git` cleanliness flags combine with OR
so a child can only add requirements, never relax the parent's. Cycles are
detected and rejected. This lets an org keep one or more shared base policies
(e.g. `policies/rust-lib.toml`) and have each repo's `deliver.toml` extend it,
instead of copy-pasting the same checks everywhere.

**`deliver schema`.** Prints a hand-maintained JSON Schema (draft-07) for the
spec format, so editors and agents can validate or autocomplete
`deliver.toml`/`deliver.json` before ever invoking `deliver`. Round-trips
through `jsonschema` in tests to guarantee it stays a valid, compilable
schema and actually accepts real specs.

## Five Areas for Feature Improvement

Prioritized by leverage for `deliver`'s actual audience — agents and CI,
where determinism and speed matter more than breadth of integrations.

1. **Performance & scale.** Checks run strictly sequentially today; on a
   spec with many independent command checks (the slowest kind — each pays
   process spawn + full timeout budget) that's wasted wall-clock. Parallel
   execution (rayon or a thread pool, capped by a `--jobs` flag), keeping the
   report's check order stable in output, is the highest-leverage change for
   large specs.
2. **CI-native reporting.** Text and JSON cover humans and generic tooling,
   but not the two formats CI systems actually consume for inline
   annotations: SARIF 2.1.0 (GitHub code scanning) and JUnit XML (every CI
   dashboard). This is what turns a failing `deliver` check into a PR
   annotation instead of a wall of log text.
3. **Explain / fix mode.** Failure messages already carry a "Suggestion:"
   clause (`file_check.rs`, `command.rs`), which is a good foundation. The
   next step is `deliver --explain` to expand *why* a check is configured the
   way it is (spec provenance — which file in an `extends` chain declared a
   given check) and, for the mechanical cases (missing directory, wrong
   `require_regex`), an opt-in `--fix` that performs the fix and re-runs.
4. **Richer Git/VCS checks.** Today `git` only checks tree cleanliness.
   Commit-message format (Conventional Commits, ticket-ID patterns) and
   branch-name policy are common agent-workflow gates ("did the agent branch
   and commit correctly") that fit naturally next to `no_uncommitted_changes`.
5. **Extensibility for check authors.** All five check kinds are hardcoded in
   `lib.rs`/`main.rs`. A narrow, low-risk extension point — a `[[command]]`
   check is already an escape hatch for "run anything," but there's no way to
   add a first-class check *kind* without a fork. A minimal plugin surface
   (start with: a check kind that shells out to a small subprocess protocol
   over stdin/stdout JSON, no WASM/Lua yet) would let teams add
   organization-specific checks without waiting on this project.

## Five New SOTA Feature Concepts

Chosen to be genuinely additive to what's already implemented (not just
"finish phase 2/3 below") and in keeping with `deliver`'s stated purpose:
_agents that need deterministic proof_.

1. **Spec composition / `extends`** — ✅ shipped this pass (see above).
2. **`deliver schema`** — ✅ shipped this pass (see above).
3. **Conditional checks (`when`).** A per-check `when` clause — `file_exists
   = "Cargo.toml"`, `os = "linux"` — that skips a check rather than failing
   it when the condition doesn't hold. This gives spec authors branching
   logic (e.g. "only run the Cargo-specific checks if this is a Rust repo")
   without a plugin system, and composes naturally with `extends`: a shared
   base policy can include checks that only activate for repos that match.
4. **Streaming NDJSON reports (`--format ndjson`).** The current JSON output
   is a single object emitted after every check finishes — an agent watching
   a long-running spec (many slow command checks) gets no signal until the
   whole run completes. NDJSON mode would emit one JSON line per check the
   moment it finishes, so an agent or a live dashboard can react — and show
   partial progress — in real time rather than polling exit status.
5. **Deliverable attestation bundle (`deliver --attest`).** On a passing
   `--strict` run, emit a small signed/timestamped JSON receipt (report hash
   + spec hash + git commit + wall-clock) that an agent can hand back as
   portable, checkable proof that a specific commit passed a specific spec at
   a specific time — the natural conclusion of `deliver`'s framing as
   deterministic proof-of-work for agents, and a lightweight stepping stone
   toward the SLSA-style provenance already on the wishlist below, without
   committing to a signing-key/PKI story yet (start with a content hash and
   local keypair, make remote signing pluggable later).

## Phased Roadmap

### Phase 1: Foundation (v0.2.0) — mostly shipped
- [x] `deliver init` wizard for spec scaffolding
- [x] `deliver validate` to check spec syntax
- [x] Shell completions (bash, zsh, fish)
- [x] Error messages with suggestions
- [x] Integration test suite (38 tests)
- [x] Property-based tests with proptest
- [x] CI/CD pipeline (GitHub Actions) — now actually triggers on `master`
- [x] Pre-commit hook integration — manifest format fixed
- [x] Performance benchmarking suite (Criterion)
- [x] Replace custom glob with `glob` crate
- [x] Spec composition (`extends`)
- [x] `deliver schema`

### Phase 2: Advanced Validation (v0.3.0) — mostly shipped
- [x] JSON Schema validation
- [x] YAML Schema validation
- [x] Directory structure checks
- [x] Symlink validation
- [x] File encoding validation (UTF-8, ASCII)
- [x] Hash verification (SHA-256, BLAKE3)
- [x] License header validation
- [x] Cross-file reference validation (Rust/Python/JS/TS/C/C++)
- [x] TOML/JSON key-path queries
- [ ] Git commit message validation
- [ ] Git branch validation
- [ ] Conditional checks (`when`)

### Phase 3: Performance & Parallelism (v0.4.0)
- [ ] Parallel check execution (`--jobs`)
- [ ] Incremental validation (git-diff-aware: only re-check what changed)
- [ ] Result caching keyed by file content hash
- [ ] Early termination on first failure (opt-in)

### Phase 4: Output & Reporting (v0.5.0)
- [ ] SARIF 2.1.0 output
- [ ] JUnit XML output
- [ ] Streaming NDJSON output
- [ ] GitHub Actions inline annotations

### Phase 5: Proof & Supply Chain (v0.6.0)
- [ ] Deliverable attestation bundle (`--attest`)
- [ ] Secret-pattern detection as a first-class check (beyond `forbid_regex`)
- [ ] SBOM presence/shape validation

### Phase 6: Extensibility (v0.7.0)
- [ ] Subprocess-protocol plugin check kind
- [ ] Plugin documentation and examples

### Phase 7: Polish (v1.0.0)
- [ ] API stability guarantees for the `deliver` library crate
- [ ] Documentation site
- [ ] 1.0 release

## Competitive Positioning

### vs agent-validator
**Our advantages:** simpler, more focused; zero heavyweight runtime deps;
Rust performance; easier to embed.
**To catch up:** AI review gates, agent skill integration, trusted snapshots,
execution state tracking.

### vs Plimsoll
**Our advantages:** more general-purpose (not just agent traces); broader
validation types; easier spec format.
**To catch up:** agent trace validation, runtime governor, reliability
metrics, multiple output formats.

### vs alint
**Our advantages:** focused on deliverables vs. repo shape; command
validation; git integration.
**To catch up:** large bundled rule catalogs, cross-file relations, auto-fix.

### vs Macaron
**Our advantages:** simpler, general-purpose, lower barrier to entry.
**To catch up:** supply-chain security focus, SLSA compliance, reproducible
builds, multi-ecosystem support.

## Conclusion

The foundation is solid and, as of this pass, actually verified end-to-end —
build, test, clippy, and fmt all gate the project's own spec, and the
integration-test/CI/docs drift that had crept in is closed. The path from
here is breadth: parallel execution, CI-native report formats, and a narrow
extensibility story, in that order of leverage for `deliver`'s actual users.
