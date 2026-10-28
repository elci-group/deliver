# Changelog

All notable changes to `deliver` are documented here. The project follows
[Semantic Versioning](https://semver.org/) from 1.0.0: the CLI behavior, the
spec format, the JSON report shape, the attestation bundle layout, the
plugin protocol, and the library API are all compatibility surfaces
(additive changes bump the minor version; breaking changes bump the major).

## 1.0.0 — 2026-10-28

The first stable release. The full roadmap to SOTA (phases 1–7) is shipped;
from here, stability is the feature. Everything listed under 0.2.0–0.7.0
below is part of the 1.0 contract.

### Added

- API stability guarantees: Semantic Versioning for the library crate, a
  documented stability policy in the crate docs, `#![deny(missing_docs)]`
  so every public item is specified, crate metadata (MSRV 1.74,
  repository, keywords, categories), `PROTOCOL_VERSION` exported, and a
  compile-time trait inventory test pinning the derive/`Send`/`Sync`
  guarantees of all public types plus deterministic report serialization.
- Documentation site under `docs/` (single page, standard-library-only
  loopback server on `127.0.0.1:8743`), kept in sync with `README.md` and
  the man page through the project's own drift guards.
- `CHANGELOG.md` (this file).

### Commitments

- Semantic Versioning across five documented compatibility surfaces: the
  CLI behavior, the spec file format (fail-closed, so additive fields are
  minor changes), the JSON report shape, the attestation bundle layout,
  and the plugin wire protocol (v1). Breaking any of them requires a major
  version.
- The shipped security controls remain in force: `--allow-commands` consent
  for command and plugin checks, `--base` path containment, bounded
  command/plugin output and timeouts with process-group termination,
  self-contained schemas only, secret findings that never echo matched
  content, and checksummed, attested release binaries (threat model in
  `INDIA_EXECUTION.md`).

## 0.7.0 — 2026-10-27

- Plugin checks (`[[plugin]]`, protocol v1) with documentation and a
  runnable example; `--allow-commands` now covers plugins.
- Library event API: `Spec::validate_with_options_and_events` streams each
  `CheckResult` as it completes.

## 0.6.0 — 2026-10-27

- `--attest`/`--attest-key`, `deliver keygen`, `deliver attest verify`.
- `forbid_secrets` file-check field.
- `[sbom]` presence/shape gate.

## 0.5.0 — 2026-10-27

- `--format ndjson` streaming output and `--format github` annotations.
- Wired the previously dormant `cache`/`incremental` modules into the
  crate, fixing a changed-path prefix bug and a `strip_prefix` type error
  found by their first-executed tests.

## 0.4.0 — 2026-10-27

- `--incremental` and `--cache-dir`: git-diff-aware incremental validation
  and a content-hash result cache (keyed by deliver version + check
  definition, replay results marked `(cached)`).
- `--fail-fast`: skip remaining checks after the first failure.

## 0.3.0 — 2026-09-24 through 2026-10-27

- Advanced validation: JSON/YAML schema checks, directory structure and
  symlink checks, encodings, SHA-256/BLAKE3 pinning, license headers,
  cross-file reference validation, and TOML/JSON key-path queries.
- Spec composition (`extends`), `deliver schema`, per-check weights with
  weighted completion, git branch and commit-message checks.
- Output: SARIF 2.1.0 and JUnit XML; parallel execution (`--jobs`).
- `when` conditional checks with skip semantics across text/JSON/SARIF/
  JUnit reports and weighted completion.
- Packaging fixes: pre-commit hook manifest rewritten as a provider list,
  CI trigger branches corrected, docs/spec drift closed and guarded by the
  project's own spec, `[[directory]]` key alias fixed.
- Versioned release-candidate workflow, verification and rollback
  procedure, and pilot starter materials.

## 0.2.0 — 2026-09-24

- Foundation: `deliver init` wizard, `deliver validate`, shell completions,
  suggestion-carrying errors, integration and property-based test suites,
  CI matrix across stable/beta/nightly, Criterion benchmarks, pre-commit
  hook integration, and `glob` crate adoption over a custom matcher.
