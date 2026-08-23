//! An empty string on a `String`-typed config field must reach every binding.
//!
//! A fixture writing `bm25_query = ""` is asserting a behaviour of the core: an *empty* query is
//! not an *absent* one, and the core's validation rejects only the absent case. Every e2e backend
//! forwards the key verbatim — except PHP, which ran fixture input through a blunt
//! `filter_empty_enum_strings` that dropped *every* `""` regardless of the field's declared type.
//! The value never reached the binding, so the PHP test exercised the default-config path and
//! failed while the other twenty backends passed. Only one of PHP's three call sites had been
//! made type-aware; the handle-arg and `options_via = "json"` sites had not.
//!
//! Dropping `""` is correct for exactly one shape: an enum-typed field, where `""` names no
//! variant and would fail deserialization. That is what `field_is_string_typed` decides, and
//! deciding it needs the IR — which is why the blunt, type-blind filter could never be right.
//!
//! The sweep iterates [`all_generators`] rather than naming languages, so a backend added later
//! is covered on the day it is added. It compares two renders of the same fixture — one carrying
//! the empty-string field, one omitting it — because a backend that drops the value produces
//! byte-identical output for both, which no single-render substring assertion detects.

use alef::core::config::NewAlefConfig;
use alef::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use alef::e2e::codegen::{E2eCodegen, all_generators};
use alef::e2e::fixture::{Fixture, FixtureGroup};

/// The config struct the fixture's `config` object deserializes into.
const OPTIONS_TYPE: &str = "WidgetOptions";
/// An always-present sibling key. Its appearance in a render is the proof that the backend
/// emits the config object at all — without it, a backend that renders no config (the C and
/// Homebrew projects among them) would be indistinguishable from one that drops the field. ~keep
const MARKER_FIELD: &str = "probe_marker";
/// The `Option<String>` field carrying `""`.
const LABEL_FIELD: &str = "probe_label";
/// The enum-typed field carrying `""`, which every backend may legitimately drop.
const MODE_FIELD: &str = "probe_mode";

/// Anti-vacuity floor, deliberately well under the observed count (48 at the time of writing).
///
/// A sweep that renders nothing passes every assertion it makes. This floor turns "examined
/// nothing" into a failure; the observed count is printed when it trips. ~keep
const MINIMUM_EXERCISED_RENDERS: usize = 40;

const JSON_OBJECT_ARG: &str = r#"
[[crates.e2e.call.args]]
name = "options"
field = "input.config"
type = "json_object"
optional = true
"#;

const HANDLE_ARG: &str = r#"
[[crates.e2e.call.args]]
name = "engine"
field = "config"
type = "handle"
optional = false
"#;

const PHP_OPTIONS_VIA_JSON: &str = r#"
[crates.e2e.call.overrides.php]
options_via = "json"
options_type = "WidgetOptions"
"#;

fn toml_src(extra: &str) -> String {
    format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "MyLib"
result_var = "result"
async = false
returns_result = true
options_type = "{OPTIONS_TYPE}"
{extra}
"#
    )
}

fn type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: OPTIONS_TYPE.to_string(),
        rust_path: format!("mylib::{OPTIONS_TYPE}"),
        has_serde: true,
        has_default: true,
        fields: vec![
            FieldDef {
                name: MARKER_FIELD.to_string(),
                ty: TypeRef::Primitive(PrimitiveType::I64),
                ..FieldDef::default()
            },
            FieldDef {
                name: LABEL_FIELD.to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                ..FieldDef::default()
            },
            FieldDef {
                name: MODE_FIELD.to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("WidgetMode".to_string()))),
                optional: true,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }]
}

fn enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: "WidgetMode".to_string(),
        rust_path: "mylib::WidgetMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fast".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Thorough".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }]
}

fn fixture(input: serde_json::Value) -> Fixture {
    Fixture {
        id: "widget_probe".to_string(),
        category: Some("widgets".to_string()),
        description: "widget probe".to_string(),
        input,
        assertions: serde_json::from_value(serde_json::json!([{ "type": "not_error" }]))
            .expect("the probe assertion must deserialize"),
        ..Fixture::default()
    }
}

/// Every emitted file of one backend's project, concatenated, or the message it failed with.
///
/// A backend that cannot render this minimal fixture is not a failure of the rule under test —
/// its render simply never mentions the marker, so the sweep skips it rather than asserting
/// against an error string.
fn render(generator: &dyn E2eCodegen, extra: &str, input: serde_json::Value) -> String {
    let config: NewAlefConfig = toml::from_str(&toml_src(extra)).expect("the e2e config under test must parse");
    let e2e = config.crates[0]
        .e2e
        .clone()
        .expect("the crate declares an [e2e] section");
    let resolved = config.resolve().expect("the config under test must resolve").remove(0);
    let groups = vec![FixtureGroup {
        category: "widgets".to_string(),
        fixtures: vec![fixture(input)],
    }];
    match generator.generate(&groups, &e2e, &resolved, &type_defs(), &enums(), &[], &[]) {
        Ok(files) => files
            .iter()
            .map(|file| format!("{}\n{}", file.path.display(), file.content))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(error) => format!("__RENDER_FAILED__ {error:#}"),
    }
}

/// A snake_case IR field name as each backend may spell it: snake, lower camel, upper camel.
fn mentions(haystack: &str, field: &str) -> bool {
    let camel: String = field
        .split('_')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect();
    let mut pascal = camel.clone();
    if let Some(first) = pascal.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    haystack.contains(field) || haystack.contains(&camel) || haystack.contains(&pascal)
}

/// The three ways a config object reaches a call, each hitting a different PHP emission path.
fn axes() -> [(&'static str, String); 3] {
    [
        ("json_object", JSON_OBJECT_ARG.to_string()),
        (
            "json_object+options_via_json",
            format!("{JSON_OBJECT_ARG}{PHP_OPTIONS_VIA_JSON}"),
        ),
        ("handle", HANDLE_ARG.to_string()),
    ]
}

#[test]
fn every_backend_that_emits_a_config_forwards_an_empty_string_on_a_string_typed_field() {
    let generators = all_generators();
    assert!(
        generators.len() >= 20,
        "the backend list returned only {} generators — a sweep over nothing passes for a healthy tree",
        generators.len()
    );

    let mut exercised = 0usize;
    let mut php_axes_exercised = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (axis, extra) in &axes() {
        for generator in &generators {
            let language = generator.language_name();
            let without = render(
                generator.as_ref(),
                extra,
                serde_json::json!({ "config": { MARKER_FIELD: 7 } }),
            );
            if !mentions(&without, MARKER_FIELD) {
                continue;
            }
            exercised += 1;
            if language == "php" {
                php_axes_exercised += 1;
            }

            let with = render(
                generator.as_ref(),
                extra,
                serde_json::json!({ "config": { MARKER_FIELD: 7, LABEL_FIELD: "" } }),
            );
            if !mentions(&with, LABEL_FIELD) {
                failures.push(format!(
                    "{language} ({axis}): dropped the `{LABEL_FIELD}` key entirely; rendered:\n{with}"
                ));
            } else if with == without {
                failures.push(format!(
                    "{language} ({axis}): rendered identically with and without `{LABEL_FIELD} = \"\"`, \
                     so the empty string never reached the binding"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} backend/axis combinations dropped an empty string on a String-typed field:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        exercised >= MINIMUM_EXERCISED_RENDERS,
        "only {exercised} backend/axis combinations emitted the config object at all (floor \
         {MINIMUM_EXERCISED_RENDERS}) — the sweep examined too little to prove anything"
    );
    assert_eq!(
        php_axes_exercised,
        axes().len(),
        "PHP is the backend this regression came from; it must be exercised on all {} config axes, \
         not {php_axes_exercised}",
        axes().len()
    );
}

/// The complement: `""` on an *enum*-typed field names no variant and must still be dropped,
/// or PHP's `from_json` fails to deserialize the config it is handed.
#[test]
fn php_still_drops_an_empty_string_on_an_enum_typed_field() {
    let php = all_generators()
        .into_iter()
        .find(|generator| generator.language_name() == "php")
        .expect("the PHP e2e generator is registered");

    for (axis, extra) in &axes() {
        let rendered = render(
            php.as_ref(),
            extra,
            serde_json::json!({ "config": { MARKER_FIELD: 7, MODE_FIELD: "" } }),
        );
        assert!(
            mentions(&rendered, MARKER_FIELD),
            "({axis}) the PHP render must emit the config object for this check to mean anything; got:\n{rendered}"
        );
        assert!(
            !mentions(&rendered, MODE_FIELD),
            "({axis}) an empty string on an enum-typed field must be dropped, not forwarded; got:\n{rendered}"
        );
    }
}
