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

## Spec Reference

A TOML spec may include `file`, `command`, and `git` checks.

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

[git]
no_uncommitted_changes = false
no_untracked_files = false
```

### File Checks

`path` is resolved relative to `--base`. Optional files pass when absent if
`required = false`. Size limits use bytes. Regex checks run against UTF-8 text
files and fail with a clear message when the regex is invalid.

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

## Exit Codes

`deliver` exits with `2` for usage, parse, and setup errors. Validation
failures exit with `1` only when `--strict` is set; otherwise the report carries
the pass/fail result and the process exits `0`.
