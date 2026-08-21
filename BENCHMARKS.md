# Benchmarks

This document records reproducible performance measurements for `deliver`.
The goal is to show that `deliver` is fast enough to run on every commit,
scalable enough for large projects, and materially faster than the ad-hoc
shell scripts it replaces.

All numbers below come from `cargo bench` on the reference machine listed in
[Environment](#environment). You can reproduce them by running:

```bash
cargo build --release
cargo bench
```

## Environment

| Property        | Value                                              |
|-----------------|----------------------------------------------------|
| OS              | Linux pop-os 6.18.7-76061807-generic x86_64        |
| CPU cores       | 12                                                 |
| Rust toolchain  | rustc 1.96.1 (31fca3adb 2026-06-26)                |
| Cargo           | cargo 1.96.1 (356927216 2026-06-26)                |
| `deliver`       | 0.1.2, built with `--release`                      |

## Summary

- **1,000 file existence checks** complete through the full CLI in **~15 ms**.
- **100 file checks with JSON schema validation** complete in **~7 ms**.
- `deliver` is **~10× faster** than an equivalent shell script for 10 files
  and **~38× faster** for 100 files.
- Spec parsing stays cheap: **~3.7 ms** for a 1,000-file TOML spec.

## Detailed Results

### 1. End-to-end CLI throughput

These benchmarks run the actual `deliver` release binary against a generated
TOML spec and a temporary directory of files. They include parsing, file I/O,
reporting, and process startup overhead.

| Files | Mean time | Time per file |
|------:|----------:|--------------:|
| 10    | 2.49 ms   | 249 µs        |
| 100   | 4.90 ms   | 49 µs         |
| 1,000 | 15.31 ms  | 15 µs         |

*Takeaway:* As the workload grows, per-file cost drops because fixed startup
overhead is amortized. A 1,000-file validation still finishes in a fraction of
a frame.

### 2. Check complexity scaling

All rows validate the same 100 files but with progressively richer checks.
This shows that adding richer validation has a modest, predictable cost.

| Check type    | Mean time | Overhead vs. existence |
|--------------:|----------:|-----------------------:|
| Existence     | 4.89 ms   | —                      |
| Regex         | 6.09 ms   | +1.20 ms (+25%)        |
| SHA-256       | 5.09 ms   | +0.20 ms (+4%)         |
| JSON schema   | 7.30 ms   | +2.41 ms (+49%)        |
| TOML keys     | 4.81 ms   | -0.08 ms (-2%)         |

*Takeaway:* Even advanced checks like JSON schema validation keep the total
runtime in the single-digit millisecond range for 100 files.

### 3. `deliver` vs. ad-hoc shell validation

Benchmark group: `deliver_vs_shell`.

For a fair comparison, the shell script performs the same three checks that
`deliver` does for each file: existence, required regex match (`OK`), and
forbidden regex match (`TODO`).

| Files | `deliver` | Shell script | Speedup |
|------:|----------:|-------------:|--------:|
| 10    | 2.81 ms   | 28.23 ms     | ~10×    |
| 100   | 6.62 ms   | 253.29 ms    | ~38×    |

*Takeaway:* A single-purpose, compiled validator removes the per-process
overhead of repeatedly spawning `test` and `grep`, and the gap widens as the
project grows.

### 4. Library-level throughput

These benchmarks exercise the `deliver` library directly without CLI or
process overhead, useful for isolating core algorithm performance.

| Benchmark       | 10      | 100      | 1,000    |
|----------------:|--------:|---------:|---------:|
| Glob expansion  | 14.9 µs | 109.9 µs | 1.53 ms  |
| Quick file check| 43.4 µs | 431.1 µs | 4.53 ms  |
| Spec validation | 20.3 µs | 83.0 µs  | —        |
| Spec parse      | 40.0 µs | 397.1 µs | 3.70 ms  |

*Takeaway:* Core operations scale roughly linearly with input size, and spec
parsing — the first thing a user pays for on every run — is measured in
microseconds for typical specs.

## Reproducing

```bash
# Build the release binary first; CLI benchmarks use target/release/deliver.
cargo build --release

# Run the full Criterion suite. Results are emitted to stdout and written to
# target/criterion/ as HTML and JSON reports.
cargo bench
```

## Notes

- Times are Criterion mean estimates.
- The shell benchmark is intentionally simple (`test -f`, `grep -q`) to match
  what a team might cobble together before adopting `deliver`.
- Benchmarks are executed serially; `deliver` itself currently runs checks
  sequentially, so these numbers represent current behavior, not a ceiling.
