//! Translation catalogue discovery, parsing and structural analysis for Ogma.
//!
//! This crate turns a [`ogma_discovery::RepositoryIndex`] into parsed
//! [`StringResource`] catalogues and provides deterministic structural
//! comparison between a baseline catalogue and a target catalogue.
//!
//! # Catalogue discovery
//!
//! [`discover_catalogues`] considers only [`FileClass::Resource`] files whose
//! extension is a known catalogue extension: `json`, `yaml`, `yml`, `toml`,
//! `po`, `pot`, `arb`, `resx`, `strings`, `stringsdict`, `properties`, `csv`,
//! `xliff`, `xlf`. Files larger than 2 MiB are skipped entirely. Well-known
//! manifest filenames (`package.json`, `Cargo.toml`, `pubspec.yaml`, `Gemfile`,
//! `go.mod`, `pom.xml`, `tsconfig.json`, `.eslintrc.json`, ...) are excluded
//! from catalogue discovery even though the indexer classifies them as
//! resources, *unless* the file sits under an i18n-ish directory
//! (`locales`, `locale`, `i18n`, `l10n`, `translations`, `translation`,
//! `langs`, `lang`, `intl`). Hidden dotfiles follow the same rule.
//!
//! Files that cannot be parsed as catalogues are skipped silently at
//! discovery level (documented); per-unit structural problems are instead
//! reported as [`Violation`]s by [`compare_catalogues`].
//!
//! # Format support notes
//!
//! - JSON/ARB: nested objects are flattened to dot-separated keys; only
//!   string leaves become units (numbers, booleans, nulls and arrays are
//!   ignored). Duplicate keys inside one JSON object are not detectable
//!   (serde limitation): the last value wins.
//! - TOML: dotted/nested tables are flattened to dot-separated keys.
//! - YAML: deliberately minimal subset — `key: value` lines,
//!   indentation-based nesting (any consistent step), full-line `#`
//!   comments, single/double-quoted values, blank lines. No anchors, flow
//!   collections, multiline scalars or tags.
//! - PO/POT: `msgctxt`, `msgid`, `msgid_plural`, `msgstr`, `msgstr[n]`,
//!   multi-line string continuations. The first entry with an empty
//!   `msgid` is the header; `Plural-Forms: nplurals=K` is read from it.
//!   Keys are `msgid`, prefixed with `msgctxt|` when a context exists.
//!   Each plural arm becomes one unit sharing the entry's key.
//! - properties: `key=value` / `key: value`, `#`/`!` comments. A first-line
//!   `locale=xx` directive declares the embedded locale and is not itself
//!   emitted as a unit.
//! - CSV: comma-separated lines; the first row is treated as a header when
//!   its first cell lowercases to `key`. Columns are `key,value[,plural_form]`.
//!   No quoted-comma support. Rows whose first cell starts with `#` are
//!   comments.
//! - XLIFF: line-scanned `<trans-unit id="..">` blocks with `<source>` and
//!   `<target>` children (may span lines). `&amp; &lt; &gt; &quot; &#39;` are
//!   unescaped. The catalogue locale comes from `target-language` (falling
//!   back to `source-language`) on the `<file>` element.
//! - RESX: `<data name="KEY">` blocks containing `<value>`.
//! - `.strings`: `"KEY" = "VALUE";` pairs with `/* */` comments stripped.
//! - `.stringsdict`: lite support — top-level `<key>` entries and, per entry,
//!   the `<string>` values of inner dictionary keys that name CLDR plural
//!   categories. Complex variable structures beyond that are not modelled.
//!
//! # Plural conventions
//!
//! A unit whose key ends in `_zero|_one|_two|_few|_many|other` or
//! `@zero|@one|@two|@few|@many|@other` — where the suffix is a known CLDR
//! category and stripping it leaves a non-empty base — is one arm of a plural
//! group: `plural_form` is the category and the effective key is the base.
//!
//! PO plural arms instead use the entry's arm index: `plural_form` is
//! `arm{i}` when the file's header declares `nplurals` (arm units are padded
//! with `None`-valued units up to the declared count so the declared
//! plurality is recoverable from the units alone), or `rawarm{i}` when the
//! file has plural entries but no `Plural-Forms` header. All arms of one PO
//! entry share the entry's key, so [`units_by_key`] groups them.
//!
//! # Line numbers
//!
//! `ogma_model::TranslationUnit` carries no line field and
//! `compare_catalogues` receives no repository root, so violations
//! conventionally have `line: None` in v1. This is a documented limitation,
//! not a correctness gap: all emitted violations are structural facts.

mod compare;
mod discovery;
mod parsers;
mod placeholders;

pub use compare::{compare_catalogues, coverage, units_by_key};
pub use discovery::discover_catalogues;
pub use placeholders::extract_placeholders;
