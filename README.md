# deliver

`deliver` is a deterministic validator for agent workflows. It checks that
expected files exist, file contents meet simple rules, command gates pass, and
optional Git cleanliness requirements are satisfied.

It is intentionally small: a spec file in, a machine-readable or human-readable
report out.

## Quick Start

```bash
deliver --spec deliver.toml --allow-commands --strict
deliver --file README.md src/*.rs
deliver --json '{"files":[{"path":"README.md"}]}' --format json
```

Use `--base DIR` when the spec should resolve relative paths somewhere other
than the current directory.

Specs containing `[[command]]` checks require `--allow-commands`. Review the
spec and its full `extends` chain first: commands run with the current user's
privileges and inherited environment. Specs without command checks run without
this flag. A `deliver.toml` from a pull request or agent should be treated as
executable code, not as trusted configuration.

For a pilot, start from the [example specs](examples/pilot/README.md) and
adapt them to the repository under test.
Candidate build and verification steps are in the
[release candidate procedure](RELEASE_CANDIDATE.md).

## Commands

Besides the default validation run, `deliver` has four subcommands:

```bash
deliver init                       # scaffold a starter deliver.toml
deliver init --output custom.toml  # scaffold at a custom path
deliver validate deliver.toml      # check spec syntax without running checks
deliver completions bash           # print shell completions (bash|zsh|fish)
deliver schema                     # print the spec format as JSON Schema
```

The starter spec from `deliver init` includes a command check, so its printed
run command includes `--allow-commands`.

`deliver validate` resolves the spec's full `extends` chain (see below), parses
it (TOML or JSON, detected from the file extension), and reports an error
without executing any file, command, or git checks — useful for a fast
pre-flight check while authoring a spec.

`deliver schema` prints a JSON Schema (draft-07) describing the spec format,
for editors and agents to validate or autocomplete a `deliver.toml`/
`deliver.json` before ever running `deliver`.

## Output Modes

Text output is designed for terminals and CI logs:

```bash
deliver --spec deliver.toml --allow-commands
deliver --spec deliver.toml --allow-commands --color always
deliver --spec deliver.toml --allow-commands --color never --progress never
```

`--color auto` enables ANSI color only when stdout is a terminal. `--progress
auto` shows a small spinner on stderr for text output in an interactive
terminal. JSON output remains stable and does not include ANSI styling.

```bash
deliver --spec deliver.toml --allow-commands --format json
```

`--format sarif` emits SARIF 2.1.0 (only failed checks become `results`, for
GitHub code scanning and other SARIF-consuming CI annotators). `--format
junit` emits JUnit XML (every check becomes a `<testcase>`, failing ones
carry a `<failure>` child, skipped ones a `<skipped/>`) for CI test-result
dashboards.

```bash
deliver --spec deliver.toml --allow-commands --format sarif
deliver --spec deliver.toml --allow-commands --format junit
```

### Streaming NDJSON

`--format ndjson` streams one JSON line per check the moment it finishes
(with a `"type": "check"` discriminator), then one `"type": "summary"` line
with the run's outcome and tallies. An agent or live dashboard watching a
long-running spec reacts to partial progress instead of polling exit status.
Line shape matches the check objects of `--format json`:

```bash
deliver --spec deliver.toml --allow-commands --format ndjson
```

With `--jobs 1`, check lines arrive in spec order; with more jobs, in
completion order. The summary line is always last, and `--strict` still
drives the exit code.

### GitHub Actions Annotations

`--format github` emits GitHub Actions workflow commands: every failed check
becomes an `::error` annotation (file checks carry a `file` property), and a
final `::notice` line carries the summary — turning a failing gate into PR
annotations instead of a wall of log text. Property values and messages are
escaped per the workflow-command rules:

```bash
deliver --spec deliver.toml --allow-commands --format github
```

Example output:

```text
::error title=file: Cargo.lock,file=Cargo.lock::required file does not exist. Suggestion: create the file or remove this check.
::notice title=deliver::3 checks: 2 passed, 1 failed in 120 ms
```

## Parallel Execution

By default checks run sequentially, in spec order. Pass `--jobs N` to run
file, command, and directory checks (not git checks, which always run last
and sequentially) across up to `N` worker threads; `--jobs 0` uses all
available CPUs. The report's check order is always spec order regardless of
`--jobs`, so output stays deterministic even though execution isn't.

```bash
deliver --spec deliver.toml --allow-commands --jobs 4
```

Command checks that share mutable state (for example, several `cargo`
invocations against the same `target/` directory) may serialize on their own
locks under `--jobs`, which is safe but won't necessarily speed things up —
`--jobs` pays off most for specs with many independent, I/O-bound checks.

## Incremental Validation and Caching

`--incremental` makes repeat runs cheap on large repositories: files whose
worktree state is unchanged since `HEAD` replay their cached results instead
of being re-read and re-validated, and only changed, staged, or untracked
files are checked fresh. It requires `--base` to sit inside a Git repository
with at least one commit and implies the default `--cache-dir`
(`$XDG_CACHE_HOME/deliver` or `~/.cache/deliver`) when none is given.

```bash
deliver --spec deliver.toml --allow-commands --incremental
```

`--cache-dir DIR` turns on the result cache by itself, without the git
awareness: file-check results replay for identical file content. Cache
entries are keyed by the deliver version plus the full check definition
(serialized), so any spec edit — including a `when` clause change — invalidates
prior entries automatically. Replayed results are always marked `(cached)` in
text and JSON reports, so a replay is never mistaken for a fresh validation.
Cache failures are never fatal: an unwritable or corrupt cache only costs
speed. Checks that depend on the filesystem rather than content alone
(`forbid_symlinks`) never replay. `--incremental`/`--cache-dir` apply to
`--spec` and `--json` runs.

## Fail-Fast

`--fail-fast` stops running remaining checks after the first failure. Skipped
checks are reported as skipped (with `fail-fast: remaining checks were not
run`), never as passed, and the run still fails overall. With `--jobs > 1`
the set of checks that still executes before the stop is timing-dependent;
the report is still spec-ordered and deterministic in shape.

```bash
deliver --spec deliver.toml --allow-commands --fail-fast --strict
```

## Spec Reference

A TOML spec may include `file`, `command`, `directory`, `git`, and `plugin`
checks, plus one `[sbom]` gate.

```toml
[[file]]
path = "src/lib.rs"
required = true
min_size_bytes = 100
forbid_regex = ["TODO", "FIXME"]
require_regex = ["pub struct Spec"]

[[command]]
name = "cargo test"
cmd = ["cargo", "test"]
expect_exit = 0
timeout_secs = 300
stdout_contains = ["test result: ok"]

[[directory]]
path = "src"
required = true
forbid_empty = true

[git]
no_uncommitted_changes = false
no_untracked_files = false
```

### File Checks

`path` is resolved relative to `--base`. Optional files pass when absent if
`required = false`. Size limits use bytes. Regex checks run against UTF-8 text
files and fail with a clear message when the regex is invalid.

Beyond size and regex, a file check accepts:

| Field                  | Type            | Effect                                                                 |
|-------------------------|-----------------|-------------------------------------------------------------------------|
| `require_line_count`    | integer         | Fail if the file has fewer than this many lines.                       |
| `encoding`               | `"utf-8"` \| `"ascii"` | Fail if the content does not match the declared encoding.        |
| `forbid_symlinks`        | bool            | Fail if the path is a symlink rather than a regular file.              |
| `sha256` / `blake3`      | hex string      | Fail unless the file's hash matches exactly (supply-chain pinning).    |
| `require_license_header` | string         | Fail unless the first 10 lines contain this substring.                 |
| `json_schema`            | JSON Schema string | Parse the file as JSON and validate it against this schema.        |
| `yaml_schema`            | YAML/JSON Schema string | Parse the file as YAML and validate it against this schema.   |
| `require_toml_keys`      | list of dotted keys | Fail unless every key path exists in the parsed TOML document.    |
| `require_json_keys`      | list of dotted keys | Fail unless every key path exists in the parsed JSON document.    |
| `forbid_secrets`         | bool            | Fail when the file matches a built-in, high-confidence secret pattern (AWS keys, `ghp_`/`glpat_`/`xox`-style provider tokens, Stripe keys, private key blocks, JWTs, assigned `password = "..."`-style values). The report names the pattern and line — never the matched content. |
| `check_references`       | bool            | For `.rs`, `.py`, `.js`/`.ts`/`.jsx`/`.tsx`, and C/C++ files, fail if a local `mod`/`import`/`#include` reference points at a file that does not exist. |

### Directory Checks

`path` is resolved relative to `--base`. Optional directories pass when
absent if `required = false`. `min_files` and `max_files` bound the number of
regular files directly inside the directory (non-recursive); `forbid_empty`
fails when the directory contains zero files.

### Command Checks

`cmd` may be an array of arguments or a simple whitespace-separated string.
Prefer the array form for reproducible behavior. Commands run from `--base`
joined with the check's `cwd`, inherit the current environment, and may add or
override environment variables through `env`.

Timeouts are enforced while stdout and stderr are captured. Each stream is
limited to 1 MiB; exceeding that limit fails the check. On Unix, a timed-out
command's process group is killed. On Windows, the direct child is killed;
descendant cleanup needs a job-object implementation before claiming full
process-tree containment. A check also fails if its output pipes remain open
more than one second after the direct child exits. `--allow-commands` is an execution consent flag,
not a sandbox. Run untrusted repositories in an isolated, low-privilege CI job.

File and directory paths and command working directories must stay beneath
`--base`, including through symlinks. Unknown spec fields now cause a parse
error, preventing a misspelled requirement from being silently skipped.
JSON Schema checks reject external HTTP and file references; their schemas
must be self-contained. Command checks may still access the network.

### Check Weights

Any `[[file]]`, `[[command]]`, or `[[directory]]` check may set an optional
`weight` — a number declaring its relative importance, defaulting to `1.0`.
Weights never change pass/fail; they feed the weighted completion figure
appended to the text summary and the `weight` field in JSON reports (omitted
for unweighted checks):

```toml
[[command]]
name = "cargo test"
cmd = ["cargo", "test"]
weight = 4.0

[[file]]
path = "README.md"
required = true
weight = 1.0
```

When at least one check carries a weight, the text summary gains a weighted
completion percentage, computed as passing weight over total weight, with
unweighted checks counting as `1.0`. For example:

`14 checks: 9 passed, 5 failed in 120 ms · 64% complete by weight`

Specs where no check sets a weight keep the plain summary line.

### Conditional Checks (`when`)

Any `[[file]]`, `[[command]]`, or `[[directory]]` check may set an optional
`when` clause with skip conditions. When a declared condition does not hold,
the check is skipped rather than failed: it reports `pass = true` with a
`skipped` flag in JSON reports and a `<skipped/>` marker in JUnit XML, never
fails the run, and is excluded from weighted completion. Declared conditions
combine with AND; an empty `when` always runs.

`file_exists` names a base-relative path that must exist (like `weight`, this
composes with `extends`: a shared base policy can carry Cargo-specific checks
that only activate in Rust repositories). `os` must match the running
operating system (`linux`, `macos`, or `windows`, case-insensitive):

```toml
[[file]]
path = "Cargo.lock"
required = true
when = { file_exists = "Cargo.toml", os = "linux" }
```

A `file_exists` path that would escape `--base` never matches, and the skip
message says so. Skipped checks stay visible in the text report, prefixed with
`-` and the reason, so a false pass through an over-eager condition can't
happen silently:

`- file: Cargo.lock [file] skipped: when.file_exists "Cargo.toml" not found`

### Git Checks

Git checks run in `--base` and can require no tracked-file diffs, no untracked
files, or both. They are useful near the end of an agent task when the expected
deliverable is a clean committed tree.

### SBOM Check

A `[sbom]` section adds one Software Bill of Materials presence and shape
gate. It searches `*.spdx`, `*.spdx.json`, `*.cdx.json`, and `bom.json` under
`--base` for SPDX (JSON or tag-value) or CycloneDX documents whose core
fields are present (`bomFormat`/`specVersion`/`components` for CycloneDX;
`spdxVersion`/`SPDXID`/`packages` for SPDX JSON; a `SPDXVersion:` record for
tag-value):

```toml
[sbom]
required = true              # fail when no SBOM exists (default)
formats = ["spdx", "cyclonedx"]  # accepted formats; empty = any
```

Detection is deliberately shallow: the gate proves a well-formed SBOM
exists, not that its contents are accurate. With `required = false`, absence
passes but a malformed SBOM still fails.

### Plugin Checks

`[[plugin]]` checks are the extension point for organization-specific check
kinds without forking `deliver`. A plugin is any executable speaking a
one-line JSON protocol on stdin/stdout (protocol v1):

```toml
[[plugin]]
name = "org gate"
cmd = ["org-gate", "--strict"]   # string form works too
timeout_secs = 60
weight = 2.0
when = { os = "linux" }
```

The runner writes one request line to the plugin's stdin —
`{"protocol": 1, "name": "...", "base": "/abs/path"}` — closes stdin, and
expects one JSON response object with exit 0:
`{"pass": true|false, "message": "..."}`. `pass` is required; `message`
becomes the report message; extra fields are ignored. A nonzero exit,
unparseable stdout, a missing `pass`, or output over 1 MiB per stream fails
the check with the plugin's stderr quoted (truncated).

Plugins are **arbitrary code**: specs containing them require the same
explicit `--allow-commands` trust decision as command checks, and there is
no sandboxing. They run in the parallel pool (report order stays spec
order), support `weight`/`when`/`cwd`/`timeout_secs`, accumulate across
`extends`, and are documented with a runnable example in
[examples/plugin](examples/plugin/README.md). `deliver schema` includes a
`plugin_check` definition for editor support.

### Deliverable Attestations

`--attest PATH` writes a bundle after a **passing** run (runs that do not
pass never produce a file, so a bundle's existence is itself the claim): a
timestamped receipt carrying the spec hash, the report hash and tallies, the
git commit when `--base` is inside a repository, and — with `--attest-key` —
an Ed25519 signature over the payload. Attestation status goes to stderr, so
report output on stdout stays machine-parseable.

```bash
deliver keygen --out keys           # writes keys.key (keep private) and keys.pub
deliver --spec deliver.toml --allow-commands --attest bundle.json --attest-key keys.key
deliver attest verify bundle.json --pub keys.pub
```

`deliver attest verify` always checks payload integrity (SHA-256 of the
canonical payload); with `--pub` it authenticates the signature against the
supplied key, otherwise it only confirms the signature is self-consistent
with its embedded key. Verification proves the bundle is intact and who
signed it — it does not re-run the checks.

### Spec Composition (`extends`)

A spec may set a top-level `extends = "path/to/parent.toml"`, resolved
relative to the directory containing the spec that declares it (not
`--base`). This lets an organization keep a shared base policy — say,
`policies/rust-lib.toml` with `no TODO`/`no unwrap()` checks and a `cargo
test` gate — and have each project's `deliver.toml` extend it with
project-specific checks:

```toml
# deliver.toml
extends = "../policies/rust-lib.toml"

[[file]]
path = "README.md"
required = true
```

Merging is additive only: file, command, and directory checks from the parent
run first, then the child's; `git` cleanliness flags combine with OR, so a
child spec can only add requirements on top of its parent, never relax one.
`extends` chains may be nested arbitrarily deep; circular chains are
rejected with an error. `extends` is only honored when loading a spec from a
file (`--spec` or `deliver validate`), not for an inline `--json` string.

## Exit Codes

`deliver` exits with `2` for usage, parse, and setup errors. Validation
failures exit with `1` only when `--strict` is set; otherwise the report carries
the pass/fail result and the process exits `0`.

## Documentation & Stability

The rendered documentation lives in `docs/` — a single-page site served
locally by a standard-library-only server:

```bash
cd docs/server && cargo run --release -- 127.0.0.1:8743
```

`CHANGELOG.md` tracks notable changes per release. From 1.0 the project
follows Semantic Versioning across five compatibility surfaces: CLI
behavior, the spec format, JSON reports, attestation bundles, and the
plugin wire protocol. The library crate documents its stability policy in
the crate root, denies missing documentation on public items, declares
MSRV 1.74 in `Cargo.toml`, and pins its derive/`Send`/`Sync` guarantees
with a compile-time inventory test (`tests/stable_api.rs`).
