# Ogma

**Deterministic Multilingual Application Conformance Engine.**

Ogma determines what human language an application is fundamentally written
for, discovers its internationalisation architecture, and proves — with
reproducible, machine-readable evidence — whether additional languages are
*natively supported*.

Ogma is not a translation service. A locale is not natively supported merely
because `fr.json` exists. Ogma answers:

> Can this application genuinely operate in language X without falling back
> to language Y, breaking linguistic semantics, or exposing untranslated
> application behaviour?

All conformance decisions are deterministic: identical source tree,
configuration, Ogma version and rules produce identical results. No LLM
judgement is involved in pass/fail decisions.

## Architecture

Rust workspace:

| Crate | Responsibility |
|---|---|
| `ogma-model` | Canonical domain types (the contract between crates) |
| `ogma-discovery` | Deterministic repository traversal and file classification |
| `ogma-parser` | Framework adapters, string extraction, locale-semantics analysis |
| `ogma-locale` | Locale parsing/normalisation, fallback chains, language profiles |
| `ogma-strings` | Translation catalogue parsing and structural analysis |
| `ogma-policy` | Explicit, composable policy files |
| `ogma-conformance` | Native-support evaluation across named check dimensions |
| `ogma-core` | Audit pipeline orchestration + incremental cache |
| `ogma-report` | Text, JSON and SARIF renderers + finding explanations |
| `ogma-integrations` | Git hooks and CI workflow templates |
| `ogma-cli` | The `ogma` binary |

Layering: CLI → policy → conformance → {strings, locale, parser} →
discovery. The analysis separates **discovery → analysis → assessment →
policy → enforcement**, so the same engine supports permissive auditing and
strict CI enforcement.

## Build

```bash
cargo build --workspace
cargo test --workspace
```

## CLI

```bash
ogma init                # write a default ogma.toml policy
ogma detect              # discovered application + language architecture
ogma audit               # full analysis (text; --format json|sarif)
ogma check               # enforcement: exit 0 compliant / 1 violation /
                         #   2 analysis error / 3 invalid configuration
ogma explain OGMA-I18N-001   # explain a finding code or path:line
ogma policy show|validate|init
ogma locales             # locale topology (coverage, fallback chains)
ogma strings             # linguistic surface inventory
ogma report --format json|sarif
ogma install-hook        # pre-commit hook running `ogma check`
ogma ci-template         # GitHub Actions workflow YAML
```

Example output:

```text
OGMA 0.1
Deterministic Multilingual Conformance

Application
  Name:        ExampleApp
  Baseline:    en-GB
  Frameworks:  react, generic
  Locales:     4

Locale          Coverage    Native    Status
------------------------------------------------
en-GB           100.0%      YES       PASS
fr-FR            99.8%      YES       PASS
de-DE            97.1%      NO        FAIL

Violations

DE-DE
  OGMA-I18N-001  src/settings.rs:184  17 hard-coded user-facing strings
  OGMA-I18N-004  locales/de.json      3 missing plural forms

Overall: FAILED
```

## Native support model

A locale is native only when every policy-relevant check passes across the
**translation coverage, structural integrity (placeholder consistency),
pluralisation, fallback independence, locale formatting, text direction,
accessibility and surface coverage** dimensions. Score alone never
determines conformance — policies impose hard requirements (e.g. one
prohibited cross-language fallback invalidates native support).

Confidence is explicit: `PROVEN / LIKELY / UNKNOWN`. `UNKNOWN` never passes
unless the policy sets `allow_unknown = true`.

## Policies

See `policies/strict.toml` and `policies/permissive.toml`. Minimal example:

```toml
[application]
baseline = "auto"

[locales]
required = ["en-GB", "fr-FR", "de-DE"]

[requirements]
translation_coverage = 1.0
allow_fallback = false
require_pluralisation = true
require_locale_formatting = true
require_accessibility = true
allow_unknown = false
```

## Conformance model invariants

- Deterministic output ordering everywhere; no timestamps or randomness.
- Vendored and generated content is classified and excluded from
  application-language analysis.
- Repositories are untrusted input: bounded reads, no code execution, no
  network access.
- Incremental analysis via `.ogma/` content-hash cache; re-runs only
  re-analyse changed files. Delete `.ogma/` or pass a non-incremental
  config to force a full scan.

## Testing

Fixture-driven: `fixtures/` contains checked-in synthetic applications
(false catalogues, fallback chains, plural breakage, RTL, vendor/generated
contamination, dynamic keys, per-framework behaviour), each with an
expectation file asserting baseline language, locale set, per-locale
violation codes, native flags and overall status. `tests/harness` runs the
audit engine against every fixture, asserts the expectations and
byte-identical determinism across repeated runs, and checks golden text
output for a reference fixture. Regenerate goldens with
`UPDATE_GOLDEN=1 cargo test -p ogma-fixture-tests`.

## Scope

Ogma deliberately does **not** translate, generate localisations, spellcheck
or manage content. Its responsibility ends at:
**Discover → Analyse → Prove → Enforce.**
