# Pilot starter specs

These examples show how to start with a small acceptance gate. Copy and adapt
one to the pilot repository; review every command and inherited `extends` spec
before executing it.

```bash
deliver --spec examples/pilot/file-only.toml --base . --strict
deliver --spec examples/pilot/rust.toml --base . --allow-commands --strict
```

`mixed.toml` assumes a Rust `backend/` and a Node `frontend/`; it is a layout
example, not a check for this repository. `file-only.toml` requires `README.md`
and a nonempty `src/` directory. The Rust example uses locked dependencies and
a formatting gate.

Run these in a low-privilege CI job for a repository or spec that has not been
reviewed. `--allow-commands` confirms the caller reviewed the executable spec;
it does not confine the commands to `--base` or remove runner credentials.

For a 30-day pilot, record the current gate and review time before adopting
`deliver`, then compare weekly gate coverage, false results, latency and
reviewer minutes. Use the [scorecard](scorecard.md) with the measurement plan
in [INDIA_EXECUTION.md](../../INDIA_EXECUTION.md).
