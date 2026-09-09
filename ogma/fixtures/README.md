# Ogma conformance fixtures

Static, deterministic fixture repositories used by the `ogma-fixture-tests`
harness crate (`tests/harness/`). Each fixture is a miniature application
tree audited by `ogma_core::run_audit`, with its expected outcome declared in
a `fixture.toml` expectation file. Every fixture is audited twice and both
the expectations and byte-identical `render_json` output are asserted.

## Taxonomy

| Fixture | Shape | Policy | Expected outcome |
| --- | --- | --- | --- |
| `complete-react` | i18next/react, en + fully translated fr | `allow_fallback` | PASS; en + fr native (golden text report) |
| `partial-translations` | i18next/react, fr missing 2 keys, 1 untranslated value, 1 placeholder mismatch | default | FAIL; fr non-native (`-002`, `-005`, `-006`, `-011`, `-012`) |
| `false-catalogue` | fr.json byte-identical to en.json | default | FAIL; fr non-native (`-006` per key, `-011`) |
| `fallback-chain` | en + fr + fr-CA, all complete | `allow_fallback` | PASS; all native (fr-CA → fr regional chain still tails into en) |
| `hard-coded-rust` | Rust CLI: `println!` + `set_text` + `panic!`, en + de complete | `allow_fallback` | PASS; global `-001` cites `src/main.rs`; `panic!` text absent from all violations |
| `pluralisation-po` | gettext en/fr with `Plural-Forms`, fr plural arms complete | `allow_fallback` | PASS; both native |
| `pluralisation-broken` | gettext fr plural entry with empty `msgstr[1]` | `allow_fallback` | FAIL; fr non-native (`-004`) |
| `rtl-app` | ar + en complete, no direction handling | `allow_fallback` + `allow_unknown` | PASS; ar native. Also audited under the default policy by the harness: FAIL, ar non-native |
| `regional-locales` | i18next `fallbackLng: "en-GB"`, en-GB + en-US + fr-FR complete | `allow_fallback` | PASS; all native; baseline `en` LIKELY |
| `mixed-language-source` | JS with hard-coded Cyrillic strings, no catalogues | default | PASS; baseline `ru` LIKELY from script evidence; global `-001` |
| `generated-files` | Rust + `src/generated.rs` with a "Code generated" marker | default | PASS; `-001` cites only `src/main.rs` |
| `vendor-dirs` | JS app + `vendor/foreign/lib.js` | default | PASS; `-001` cites only `src/app.js` |
| `dynamic-keys` | `t(dynamicKey)` / `` t(`prefix.${name}`) `` only | `allow_fallback` | PASS; no extraction, orphan analysis skipped (empty `source_keys`) |
| `django-app` | Django: `LANGUAGE_CODE`, `{% trans %}`, gettext `_()`, en + fr `.po` | `allow_fallback` | PASS; baseline `en` PROVEN; framework `django` detected; `-001` cites `templates/base.html` |

## Expectation file format (`fixture.toml`)

```toml
description = "what this fixture proves"          # human-readable, also scanned as a TOML catalogue unit

[policy]                     # optional; omitted keys keep Policy::default()
baseline = "en-GB"           # optional explicit baseline ("auto" default)
required = ["fr-FR"]         # optional policy-required locales
translation_coverage = 0.95  # optional
allow_fallback = true        # optional
require_pluralisation = true # optional
require_locale_formatting = true # optional
require_accessibility = false # optional
allow_unknown = true         # optional

[expect]
baseline = "en"              # expected baseline language code, or "none" (undetected)
baseline_confidence = "PROVEN"  # optional: UNKNOWN | LIKELY | PROVEN
locales = ["en", "fr"]       # exact canonical locale set
overall = "PASS"             # PASS | FAIL | UNKNOWN
frameworks = ["django", "generic"]  # optional exact framework list
absent_text = ["internal invariant"] # optional strings forbidden in any violation message/detail

[expect.native]              # locales expected native; every reported locale
fr = true                    # NOT listed here is expected non-native

[expect.violations]          # exact violation-code sets per scope
global = ["OGMA-I18N-001"]
en = []
fr = ["OGMA-I18N-002", "OGMA-I18N-005"]

[expect.violation_files]     # optional: exact file sets per GLOBAL code
"OGMA-I18N-001" = ["src/main.rs"]
```

Scopes in `[expect.violations]` must cover `"global"` plus every reported
locale; code sets are compared exactly (order-independent).

## Notes and accepted engine behaviors encoded here

- **Hard-coded strings are Warnings.** Global `OGMA-I18N-001` findings never
  fail the audit by themselves; only non-native locales or Error-severity
  global violations do.
- **Baseline self-comparison is silent.** The engine filters
  `OGMA-I18N-006` (and friends) when a catalogue is compared against itself,
  so baseline locales report no structural violations even under the default
  policy.
- **Latin-script baselines need config hints.** Script guessing returns
  `None` for Latin text, so fixtures without a `defaultLocale`-class hint and
  with symmetric catalogue naming report baseline `none`/`UNKNOWN`; the
  working baseline then falls back to `en`. Fixtures with a `fallbackLng`
  hint detect their baseline at `LIKELY` (Medium evidence; `fallbackLng` is
  not a Strong default declaration, and baseline detection reports a
  *language*, not a locale).
- **Fallback chains always tail into the baseline.** A regional chain like
  `fr-CA → fr` is still followed by `fr-CA → en` when the baseline is
  another language, so any two-language repository requires
  `allow_fallback = true` for non-baseline locales to be native.
- **`fixture.toml` is part of the scanned tree.** The harness audits the
  fixture directory as-is; `fixture.toml` is classified a Resource and
  parsed as a (locale-less) TOML catalogue. In fixtures without real
  catalogues it becomes the baseline linguistic surface. This is
  deterministic and encoded in the expectations.
- **RTL gating** is exercised both ways: `rtl-app` passes with
  `allow_unknown = true` and fails under the default policy (secondary
  hardcoded harness check).

## Running

```sh
cargo test -p ogma-fixture-tests            # audit every fixture, twice, and assert
UPDATE_GOLDEN=1 cargo test -p ogma-fixture-tests  # regenerate golden/complete-react.txt
```
