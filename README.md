# deliver

`deliver` is a deterministic validator for agent workflows. It checks that
expected files exist, file contents meet simple rules, command gates pass, and
optional Git cleanliness requirements are satisfied.

It is intentionally small: a spec file in, a machine-readable or human-readable
report out.

## Quick Start

```bash
deliver --spec deliver.toml --strict
deliver --file README.md src/*.rs
deliver --json '{"files":[{"path":"README.md"}]}' --format json
```

Use `--base DIR` when the spec should resolve relative paths somewhere other
than the current directory.

## Commands

Besides the default validation run, `deliver` has three subcommands:

```bash
deliver init                       # scaffold a starter deliver.toml
deliver init --output custom.toml  # scaffold at a custom path
deliver validate deliver.toml      # check spec syntax without running checks
deliver completions bash           # print shell completions (bash|zsh|fish)
deliver schema                     # print the spec format as JSON Schema
```

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
deliver --spec deliver.toml
deliver --spec deliver.toml --color always
deliver --spec deliver.toml --color never --progress never
```

`--color auto` enables ANSI color only when stdout is a terminal. `--progress
auto` shows a small spinner on stderr for text output in an interactive
terminal. JSON output remains stable and does not include ANSI styling.

```bash
deliver --spec deliver.toml --format json
```

`--format sarif` emits SARIF 2.1.0 (only failed checks become `results`, for
GitHub code scanning and other SARIF-consuming CI annotators). `--format
junit` emits JUnit XML (every check becomes a `<testcase>`, failing ones
carry a `<failure>` child) for CI test-result dashboards.

```bash
deliver --spec deliver.toml --format sarif
deliver --spec deliver.toml --format junit
```

## Parallel Execution

By default checks run sequentially, in spec order. Pass `--jobs N` to run
file, command, and directory checks (not git checks, which always run last
and sequentially) across up to `N` worker threads; `--jobs 0` uses all
available CPUs. The report's check order is always spec order regardless of
`--jobs`, so output stays deterministic even though execution isn't.

```bash
deliver --spec deliver.toml --jobs 4
```

Command checks that share mutable state (for example, several `cargo`
invocations against the same `target/` directory) may serialize on their own
locks under `--jobs`, which is safe but won't necessarily speed things up —
`--jobs` pays off most for specs with many independent, I/O-bound checks.

## Spec Reference

A TOML spec may include `file`, `command`, `directory`, and `git` checks.

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

Timeouts are enforced while stdout and stderr are captured, so a noisy command
cannot block the validator indefinitely.

### Git Checks

Git checks run in `--base` and can require no tracked-file diffs, no untracked
files, or both. They are useful near the end of an agent task when the expected
deliverable is a clean committed tree.

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
