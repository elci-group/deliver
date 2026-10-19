# India roadmap execution log

_Started 14 October 2026. Source plan: [India market readiness and roadmap](INDIA_MARKET_READINESS.md). This log tracks work that actually happened; targets in the roadmap are not marked complete merely because a document or feature exists._

## Current gate

**Phase 0 in progress; phase 1 engineering in progress.** No external pilot, design partner, procurement review or regulatory certification has occurred.

| Roadmap item | State | Evidence / next acceptance step |
|---|---|---|
| Product baseline and documentation drift | **Partly done** | `README.md`, man page and historical benchmark wording updated for the new command trust requirement and `--jobs` context. CI now has macOS/Windows test jobs, but they have not run from this checkout. Performance baseline remains to be measured on release builds. |
| Threat model | **Initial model written** | See below. Independent security review and race-resistant containment remain open. |
| Public India prospect research | **Shortlist prepared** | [Seven researched organizations](INDIA_PROSPECTS.md), each with an official source and qualification question. All are uncontacted. |
| Buyer interviews | **Not started** | Need 15–20 interviews, a named product owner and permissioned outreach channels. |
| Design partners and pilots | **Not started** | Need five qualified partners, then three scoped pilots across at least two repositories each. |
| Explicit command trust | **Implemented locally** | CLI requires `--allow-commands` for specs containing commands, including inherited specs. Library callers still control their own trust boundary. |
| Fail closed on unknown fields | **Implemented locally** | Serde rejects unknown fields in all spec check types. |
| Check-path containment | **Implemented with limits** | File and directory targets and command working directories reject lexical parent/absolute paths and symlinks escaping `--base`. This is a pre-use check, not a race-free OS sandbox. |
| Command output and timeout bounds | **Implemented with limits** | Each stream capped at 1 MiB; pipe close has a bounded grace period. Unix timeouts kill the child's process group; Windows only kills the direct child. A full Windows job-object implementation remains open. |
| Schema reference egress | **Disabled locally** | `jsonschema` is built without its default HTTP/file resolver features; external references fail. Trusted command checks can still use the network. |
| Two-repository pilot starter | **Templates prepared** | [File-only, Rust and mixed-repository specs](examples/pilot/README.md) exist and parse; a [30-day scorecard](examples/pilot/scorecard.md) defines the baseline, weekly measures and day-30 decision. No customer repository has run them. |
| Release provenance and verification | **Workflow and procedure prepared** | [Manual candidate workflow](.github/workflows/release-candidate.yml) builds and tests three platforms, writes checksums, and requests GitHub attestations. [Verification and rollback procedure](RELEASE_CANDIDATE.md) is written. No workflow run or independent verification has occurred. |
| Evidence bundle and policy fleet | **Not started** | Phase 2/3 work requires pilot evidence before claiming enterprise readiness. |

## Initial threat model and trust boundary

**Assets:** repository contents, runner environment secrets, CI credentials, check result integrity, runner availability, and future evidence records. **Inputs:** TOML/JSON spec, inherited spec chain, selected file paths, command arguments/environment, repository files, and subprocess output. **Actors:** trusted maintainers; agent-authored changes; external pull requests; compromised dependencies or release artifacts. **Sensitive boundary:** a command check can run any executable the runner can run. `--allow-commands` requires a deliberate caller choice but grants no isolation. A trusted spec can still read outside `--base` through its command. Run untrusted code only under a separately isolated runner with limited credentials and network access.

**Implemented controls:** command opt-in, unknown-field rejection, path guard for data checks and command working directories, bounded output memory, bounded wait for inherited output pipes, Unix group termination on timeout, and omission of expected stdout/stderr values from failed command reports. **Open attack paths:** path replacement after validation; explicit command access to host files/network; children escaping the process group; Windows descendant processes; external `extends` chain trust; other report fields that may reveal sensitive paths or patterns; unverified release binaries and mutable evidence. These are phase-1/2 blockers, not accepted enterprise claims.

## Pilot package to prepare next

1. Publish a version-pinned binary for each supported platform with independent checksum/signature verification and rollback instructions.
2. Make a two-repository starter package: one file-only spec and one trusted command spec; record the explicit trust decision.
3. Give each pilot owner a 30-day measurement sheet: install-to-first-pass time, gate coverage, adjudicated false results, p50/p95 gate latency, reviewer minutes per release, maintenance time, and evidence completeness.
4. Require a before/after sample of the current CI script and release review path. Keep source and raw logs inside the pilot customer's environment.
5. Hold weekly issue reviews and a final go/no-go review against the thresholds in the market roadmap.

## Interview script for a public-source prospect

1. Which release decision needs manual review, and who makes it?
2. Where are acceptance checks defined today, and how many repositories share them?
3. What is the most recent false pass, false failure, or missing evidence incident?
4. Does CI run on GitHub Actions, GitLab, Jenkins or another system? Can a local binary run without network access?
5. What source, log, credential or residency constraints would block a pilot?
6. What measured improvement would justify rollout to ten repositories or a paid support agreement?

## Outreach draft for review

> Subject: Research interview on release gates across engineering teams
>
> Hello — I am researching how India-based platform and delivery teams define
> and verify release acceptance checks across repositories. We are developing
> `deliver`, a local CLI that runs a versioned acceptance spec and
> reports which checks passed. I would value 20 minutes to understand your
> current workflow, where review evidence is hard to produce, and whether a
> small two-repository pilot would be useful. This is a research request, not
> a claim that your team needs the product. No repository or customer data is
> needed for the initial conversation.
>
> If there is a better owner for release engineering or platform standards,
> please let me know.

This draft has not been sent. Verify the recipient, channel and organizational
fit before adapting it to any prospect. No contact data belongs in this repo.

## Gate for the next update

Local verification on 14 October 2026: the project `deliver.toml` passed all
12 checks, including `cargo test`, `cargo build`, Clippy with warnings denied,
and formatting. The test suite has 78 passing tests (18 library, 9 binary,
51 integration). All three pilot specs parsed; the file-only and Rust examples
passed against this checkout. `git diff --check` passed. The release candidate
workflow parsed as YAML; its hosted jobs have not run from this checkout.

Next, obtain a named product owner and a permissioned prospect workflow. Do
not advance to a hosted control plane or a compliance claim on the basis of
public research alone.
