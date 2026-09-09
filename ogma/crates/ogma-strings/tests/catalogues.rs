//! File-based parser and discovery tests. Fixtures live under the system
//! temp dir and are removed after each test; no external crates.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ogma_model::{CatalogueFormat, codes, Severity};
use ogma_strings::{compare_catalogues, discover_catalogues, units_by_key};

/// A self-cleaning fixture directory.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ogma-strings-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn discover(&self) -> Vec<ogma_model::StringResource> {
        let index = ogma_discovery::scan(&self.root).unwrap();
        discover_catalogues(&index, &self.root)
    }

    fn find(&self, resources: &[ogma_model::StringResource], rel: &str) -> ogma_model::StringResource {
        resources
            .iter()
            .find(|r| r.path == Path::new(rel))
            .unwrap_or_else(|| panic!("catalogue {rel} not discovered"))
            .clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn keys(res: &ogma_model::StringResource) -> Vec<String> {
    units_by_key(res).into_keys().collect()
}

// ---------------------------------------------------------------------------
// Format parsers
// ---------------------------------------------------------------------------

#[test]
fn json_nested_flattens_to_dot_keys() {
    let fx = Fixture::new("json-nested");
    fx.write(
        "locales/en.json",
        r#"{
          "welcome": "Hello",
          "menu": { "file": "File", "edit": { "copy": "Copy" } },
          "ignored_number": 42,
          "ignored_bool": true,
          "ignored_array": ["a", "b"],
          "empty": ""
        }"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/en.json");
    assert_eq!(res.format, CatalogueFormat::Json);
    // Only string leaves become units; numbers/bools/arrays are ignored.
    assert_eq!(keys(&res), vec!["empty", "menu.edit.copy", "menu.file", "welcome"]);
    let units = units_by_key(&res);
    assert_eq!(units["welcome"][0].value.as_deref(), Some("Hello"));
    assert_eq!(units["menu.edit.copy"][0].value.as_deref(), Some("Copy"));
    assert!(units["empty"][0].value.is_none());
}

#[test]
fn arb_embedded_locale_and_metadata_skip() {
    let fx = Fixture::new("arb");
    fx.write(
        "lib/l10n/app_fr.arb",
        r#"{
          "@@locale": "fr",
          "hello": "Bonjour",
          "@hello": { "description": "greeting", "placeholders": { "name": {} } },
          "bye": "Au revoir"
        }"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "lib/l10n/app_fr.arb");
    assert_eq!(res.format, CatalogueFormat::Arb);
    assert_eq!(
        res.embedded_locale.as_ref().map(|l| l.canonical.as_str()),
        Some("fr")
    );
    assert_eq!(keys(&res), vec!["bye", "hello"]);
    assert!(res.declared_locale.is_none() || res.effective_locale().is_some());
}

#[test]
fn toml_dotted_tables_flatten() {
    let fx = Fixture::new("toml");
    fx.write(
        "i18n/en.toml",
        "[menu]\nfile = \"File\"\n\n[menu.edit]\ncopy = \"Copy\"\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "i18n/en.toml");
    assert_eq!(res.format, CatalogueFormat::Toml);
    assert_eq!(keys(&res), vec!["menu.edit.copy", "menu.file"]);
}

#[test]
fn yaml_subset_nesting_quotes_comments() {
    let fx = Fixture::new("yaml");
    fx.write(
        "locales/en.yaml",
        "# a comment\nwelcome: \"Hello\"\nmenu:\n  file: File\n  edit:\n    copy: 'Copy'  # trailing\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/en.yaml");
    assert_eq!(res.format, CatalogueFormat::Yaml);
    assert_eq!(keys(&res), vec!["menu.edit.copy", "menu.file", "welcome"]);
    let units = units_by_key(&res);
    assert_eq!(units["welcome"][0].value.as_deref(), Some("Hello"));
    assert_eq!(units["menu.edit.copy"][0].value.as_deref(), Some("Copy"));
}

#[test]
fn po_with_header_plurals_and_context() {
    let fx = Fixture::new("po");
    fx.write(
        "locales/fr.po",
        r#"msgid ""
msgstr "Project-Id-Version: x\nPlural-Forms: nplurals=2; plural=(n > 1);\n"

msgctxt "menu"
msgid "File"
msgstr "Fichier"

msgid "apple"
msgid_plural "apples"
msgstr[0] "pomme"
msgstr[1] "pommes"
"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/fr.po");
    assert_eq!(res.format, CatalogueFormat::Po);
    assert_eq!(keys(&res), vec!["apple", "menu|File"]);
    let units = units_by_key(&res);
    let arms = &units["apple"];
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].plural_form.as_deref(), Some("arm0"));
    assert_eq!(arms[0].value.as_deref(), Some("pomme"));
    assert_eq!(arms[1].plural_form.as_deref(), Some("arm1"));
    assert_eq!(arms[1].value.as_deref(), Some("pommes"));
    assert_eq!(units["menu|File"][0].value.as_deref(), Some("Fichier"));
}

#[test]
fn pot_all_msgstr_empty_becomes_none() {
    let fx = Fixture::new("pot");
    fx.write(
        "locales/messages.pot",
        "msgid \"\"\nmsgstr \"Project-Id-Version: x\\n\"\n\nmsgid \"hello\"\nmsgstr \"\"\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/messages.pot");
    assert_eq!(res.format, CatalogueFormat::Pot);
    let units = units_by_key(&res);
    assert_eq!(units["hello"][0].value, None);
}

#[test]
fn po_plural_without_header_marks_raw_arms() {
    let fx = Fixture::new("po-rawarm");
    fx.write(
        "locales/fr.po",
        "msgid \"apple\"\nmsgid_plural \"apples\"\nmsgstr[0] \"pomme\"\nmsgstr[1] \"pommes\"\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/fr.po");
    let units = units_by_key(&res);
    assert_eq!(units["apple"][0].plural_form.as_deref(), Some("rawarm0"));
}

#[test]
fn properties_duplicates_kept_and_locale_directive() {
    let fx = Fixture::new("properties");
    fx.write(
        "i18n/messages_fr.properties",
        "locale=fr\n# comment\na=un\nb: deux\na=une\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "i18n/messages_fr.properties");
    assert_eq!(
        res.embedded_locale.as_ref().map(|l| l.canonical.as_str()),
        Some("fr")
    );
    let units = units_by_key(&res);
    assert_eq!(units["a"].len(), 2);
    assert_eq!(units["b"][0].value.as_deref(), Some("deux"));

    // Duplicate is reported by compare.
    let baseline = ogma_model::StringResource {
        path: "messages_en.properties".into(),
        format: CatalogueFormat::Properties,
        declared_locale: None,
        embedded_locale: None,
        units: vec![
            ogma_model::TranslationUnit {
                key: "a".into(), value: Some("one".into()), locale: None,
                plural_form: None, placeholders: vec![],
            },
            ogma_model::TranslationUnit {
                key: "b".into(), value: Some("two".into()), locale: None,
                plural_form: None, placeholders: vec![],
            },
        ],
    };
    let violations = compare_catalogues(&baseline, &res, &[], &BTreeSet::new());
    assert!(violations.iter().any(|v| v.code == codes::DUPLICATE_KEY && v.severity == Severity::Warning));
}

#[test]
fn csv_header_comments_plural_column() {
    let fx = Fixture::new("csv");
    fx.write(
        "locales/fr.csv",
        "key,value,plural_form\n# comment row\nwelcome,Bienvenue,\nitems_one,un,one\nitems_other,autres,other\nwelcome,Bonsoir,\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/fr.csv");
    let units = units_by_key(&res);
    assert_eq!(units["welcome"].len(), 2); // duplicate kept
    // Explicit plural column + key suffix: grouped under the stripped base.
    assert_eq!(keys(&res), vec!["items", "welcome"]);
    let arms = &units["items"];
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].plural_form.as_deref(), Some("one"));
    assert_eq!(arms[1].plural_form.as_deref(), Some("other"));
}

#[test]
fn xliff_units_locale_and_unescape() {
    let fx = Fixture::new("xliff");
    fx.write(
        "translations/fr.xliff",
        r#"<?xml version="1.0"?>
<xliff version="1.2"><file source-language="en" target-language="fr-FR" datatype="plaintext">
<body>
<trans-unit id="welcome"><source>Hello &amp; welcome</source><target>Bonjour &amp; bienvenue</target></trans-unit>
<trans-unit id="menu.file">
  <source>File</source>
  <target>Fichier</target>
</trans-unit>
</body></file></xliff>
"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "translations/fr.xliff");
    assert_eq!(res.format, CatalogueFormat::Xliff);
    assert_eq!(
        res.embedded_locale.as_ref().map(|l| l.canonical.as_str()),
        Some("fr-FR")
    );
    let units = units_by_key(&res);
    assert_eq!(units["welcome"][0].value.as_deref(), Some("Bonjour & bienvenue"));
    assert_eq!(units["menu.file"][0].value.as_deref(), Some("Fichier"));
}

#[test]
fn resx_data_blocks() {
    let fx = Fixture::new("resx");
    fx.write(
        "Strings.fr.resx",
        r#"<?xml version="1.0"?>
<root><data name="welcome" xml:space="preserve"><value>Bonjour</value></data>
<data name="menu.file"><value>Fichier</value></data></root>
"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "Strings.fr.resx");
    assert_eq!(res.format, CatalogueFormat::Resx);
    let units = units_by_key(&res);
    assert_eq!(units["welcome"][0].value.as_deref(), Some("Bonjour"));
    assert_eq!(units["menu.file"][0].value.as_deref(), Some("Fichier"));
}

#[test]
fn apple_strings_pairs_with_comments() {
    let fx = Fixture::new("strings");
    fx.write(
        "fr.lproj/Localizable.strings",
        "/* a comment\n spanning lines */\n\"welcome\" = \"Bonjour\";\n\"menu.file\" = \"Fichier\";\n",
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "fr.lproj/Localizable.strings");
    assert_eq!(res.format, CatalogueFormat::AppleStrings);
    assert_eq!(
        res.declared_locale.as_ref().map(|l| l.canonical.as_str()),
        Some("fr")
    );
    let units = units_by_key(&res);
    assert_eq!(units["welcome"][0].value.as_deref(), Some("Bonjour"));
}

#[test]
fn apple_stringsdict_plural_arms() {
    let fx = Fixture::new("stringsdict");
    fx.write(
        "fr.lproj/Localizable.stringsdict",
        r#"<?xml version="1.0"?>
<plist version="1.0"><dict>
<key>items</key>
<dict>
  <key>NSStringLocalizedFormatKey</key>
  <string>%#@items@</string>
  <key>items</key>
  <dict>
    <key>NSStringFormatSpecTypeKey</key><string>NSStringPluralRuleType</string>
    <key>one</key><string>une pomme</string>
    <key>other</key><string>%d pommes</string>
  </dict>
</dict>
</dict></plist>
"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "fr.lproj/Localizable.stringsdict");
    assert_eq!(res.format, CatalogueFormat::AppleStringsdict);
    let units = units_by_key(&res);
    let arms = &units["items"];
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].plural_form.as_deref(), Some("one"));
    assert_eq!(arms[0].value.as_deref(), Some("une pomme"));
    assert_eq!(arms[1].plural_form.as_deref(), Some("other"));
}

// ---------------------------------------------------------------------------
// Locale inference and discovery filtering
// ---------------------------------------------------------------------------

#[test]
fn locale_inference_matrix() {
    let fx = Fixture::new("locale-inf");
    fx.write("locales/fr.json", "{}");
    fx.write("locales/messages_fr.json", "{}");
    fx.write("app-en-US.json", "{}");
    fx.write("locales/en/messages.json", "{}");
    fx.write("i18n/fr-CA.lproj/Localizable.strings", "\"a\" = \"b\";");
    fx.write("locales/de-AT/app.json", "{}");
    fx.write("data/blob.json", "{}");

    let resources = fx.discover();
    let declared = |rel: &str| {
        fx.find(&resources, rel)
            .declared_locale
            .map(|l| l.canonical)
    };
    assert_eq!(declared("locales/fr.json").as_deref(), Some("fr"));
    assert_eq!(declared("locales/messages_fr.json").as_deref(), Some("fr"));
    assert_eq!(declared("app-en-US.json").as_deref(), Some("en-US"));
    assert_eq!(declared("locales/en/messages.json").as_deref(), Some("en"));
    assert_eq!(
        declared("i18n/fr-CA.lproj/Localizable.strings").as_deref(),
        Some("fr-CA")
    );
    assert_eq!(declared("locales/de-AT/app.json").as_deref(), Some("de-AT"));
    // No locale signal at all.
    assert!(declared("data/blob.json").is_none());
}

#[test]
fn manifest_exclusions() {
    let fx = Fixture::new("manifests");
    fx.write("package.json", r#"{"name": "app"}"#);
    fx.write("tsconfig.json", "{}");
    fx.write(".eslintrc.json", "{}");
    fx.write("locales/package.json", r#"{"hello": "bonjour"}"#);
    fx.write("locales/fr.json", "{}");

    let resources = fx.discover();
    let paths: Vec<String> = resources
        .iter()
        .map(|r| r.path.to_string_lossy().into_owned())
        .collect();
    assert!(!paths.iter().any(|p| p == "package.json"));
    assert!(!paths.iter().any(|p| p == "tsconfig.json"));
    assert!(!paths.iter().any(|p| p == ".eslintrc.json"));
    // Under an i18n-ish dir the manifest name is kept.
    assert!(paths.iter().any(|p| p == "locales/package.json"));
    assert!(paths.iter().any(|p| p == "locales/fr.json"));
}

#[test]
fn oversized_and_unparseable_files_skipped() {
    let fx = Fixture::new("skips");
    fx.write("locales/en.json", "not json at all {{{");
    let big = "x".repeat(3 * 1024 * 1024);
    fx.write("locales/big.json", &big);
    fx.write("locales/ok.json", "{}");
    let resources = fx.discover();
    let paths: Vec<&Path> = resources.iter().map(|r| r.path.as_path()).collect();
    assert_eq!(paths, vec![Path::new("locales/ok.json")]);
}

#[test]
fn discovery_is_deterministic_and_sorted() {
    let fx = Fixture::new("determinism");
    fx.write("locales/b.json", "{}");
    fx.write("locales/a.json", "{}");
    fx.write("i18n/c.yaml", "k: v\n");
    let first = fx.discover();
    let second = fx.discover();
    assert_eq!(first, second);
    let paths: Vec<&Path> = first.iter().map(|r| r.path.as_path()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn plural_suffix_convention_json() {
    let fx = Fixture::new("plural-suffix");
    fx.write(
        "locales/fr.json",
        r#"{"items_one": "un", "items_other": "autres", "plain_key": "v"}"#,
    );
    let resources = fx.discover();
    let res = fx.find(&resources, "locales/fr.json");
    let units = units_by_key(&res);
    assert_eq!(units["items"].len(), 2);
    assert_eq!(units["items"][0].plural_form.as_deref(), Some("one"));
    assert!(units.contains_key("plain_key"));
}

#[test]
fn end_to_end_compare_po_target_against_json_baseline() {
    let fx = Fixture::new("e2e");
    fx.write(
        "locales/en.json",
        r#"{"hello": "Hello", "apples_one": "one apple", "apples_other": "{n} apples"}"#,
    );
    fx.write(
        "locales/fr.po",
        r#"msgid ""
msgstr "Plural-Forms: nplurals=2; plural=(n > 1);\n"

msgid "hello"
msgstr "Bonjour"

msgid "apples"
msgid_plural "apples"
msgstr[0] "une pomme"
msgstr[1] "{n} pommes"
"#,
    );
    let resources = fx.discover();
    let baseline = fx.find(&resources, "locales/en.json");
    let target = fx.find(&resources, "locales/fr.po");
    let profile = ogma_locale::profile_for(&ogma_model::Language::new("fr"));
    let violations = compare_catalogues(
        &baseline,
        &target,
        &profile.required_plural_categories,
        &BTreeSet::new(),
    );
    assert!(violations.is_empty(), "unexpected violations: {violations:?}");
}
