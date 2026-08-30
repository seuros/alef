//! Cross-backend guard: the TypeScript declaration a Node-family backend writes for an
//! `#[serde(untagged)]` data enum describes a JSON document, so its object keys are serde WIRE
//! names — never the host-language property names a `#[napi(object)]` / `#[wasm_bindgen]` wrapper
//! would expose.
//!
//! Nothing here is a hand-written expected key. [`SampleInput`] and [`SampleRenamedInput`] are
//! real Rust enums carrying the same serde attributes as the IR fixtures, so `serde_json` produces
//! the key every backend is measured against — the only authority there is.
//!
//! Why this shape and not any other: an untagged data enum is the one enum lowering with no
//! wrapper object on either side. napi routes it to `gen_untagged_data_enum_as_value_wrapper`
//! (a `#[serde(transparent)]` newtype over `serde_json::Value`) and converts with
//! `serde_json::from_value(val.0)` straight into the CORE type; wasm overrides it to `JsValue`
//! and converts with `serde_wasm_bindgen::from_value` into the same core type. There is no
//! `js_name` anywhere on either path, so a `.d.ts` naming those keys `to_node_name(&field.name)`
//! described a shape neither deserializer accepts. Both conversions end in `unwrap_or_default()`,
//! so the mismatch does not raise — it silently yields the enum's Default, which is why this has
//! to be caught at generation time.
//!
//! `backends::go::gen_bindings::types::field_shape::go_data_enum_variant_field` is the sibling
//! that has always kept the two apart, returning the host name and the JSON key as separate
//! values from one function. ~keep

use crate::backends::napi::NapiBackend;
use crate::backends::wasm::WasmBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};

/// The ground truth for the no-rename case. Mirrored field-for-field by [`fixture_enum`].
#[derive(serde::Serialize)]
#[serde(untagged)]
enum SampleInput {
    Bare(String),
    Detailed { max_chars: u32 },
}

/// The ground truth for the container-rule case: `rename_all_fields` cases a struct variant's
/// FIELD names. It is a different serde namespace from `rename_all`, which cases VARIANT names
/// and must not reach a field. Mirrored by [`renamed_fixture_enum`].
#[derive(serde::Serialize)]
#[serde(untagged, rename_all = "kebab-case", rename_all_fields = "SCREAMING_SNAKE_CASE")]
enum SampleRenamedInput {
    Detailed { max_chars: u32 },
}

const ENUM_NAME: &str = "SampleInput";
const RENAMED_ENUM_NAME: &str = "SampleRenamedInput";
/// The Rust field name. Deliberately multi-word so the host spelling (`maxChars`) and the serde
/// spelling (`max_chars`) cannot coincide — a single-word field would let a wrong emitter pass.
const FIELD: &str = "max_chars";
/// The host-language property name napi-rs and wasm-bindgen would expose for [`FIELD`] if this
/// enum went through a wrapper object. It must appear nowhere in either backend's output for an
/// untagged enum, because no wrapper object exists on that path.
const HOST_SPELLING: &str = "maxChars";

/// The single JSON key serde writes for [`FIELD`], read out of serde's own output.
fn serde_key_for(json: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(json).expect("fixture serializes to JSON");
    let object = value.as_object().expect("the struct variant serializes to an object");
    let mut keys: Vec<&String> = object.keys().collect();
    assert_eq!(keys.len(), 1, "the fixture variant has exactly one field: {json}");
    keys.pop().expect("one key").clone()
}

fn plain_serde_key() -> String {
    serde_key_for(&serde_json::to_string(&SampleInput::Detailed { max_chars: 4 }).expect("serializes"))
}

fn renamed_serde_key() -> String {
    serde_key_for(&serde_json::to_string(&SampleRenamedInput::Detailed { max_chars: 4 }).expect("serializes"))
}

fn detailed_variant() -> EnumVariant {
    EnumVariant {
        name: "Detailed".to_string(),
        fields: vec![FieldDef {
            name: FIELD.to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..FieldDef::default()
        }],
        ..EnumVariant::default()
    }
}

/// The IR mirror of [`SampleInput`].
fn fixture_enum() -> EnumDef {
    EnumDef {
        name: ENUM_NAME.to_string(),
        rust_path: format!("sample_core::{ENUM_NAME}"),
        has_serde: true,
        serde_untagged: true,
        variants: vec![
            EnumVariant {
                name: "Bare".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
            detailed_variant(),
        ],
        ..EnumDef::default()
    }
}

/// The IR mirror of [`SampleRenamedInput`].
fn renamed_fixture_enum() -> EnumDef {
    EnumDef {
        name: RENAMED_ENUM_NAME.to_string(),
        rust_path: format!("sample_core::{RENAMED_ENUM_NAME}"),
        has_serde: true,
        serde_untagged: true,
        serde_rename_all: Some("kebab-case".to_string()),
        rename_all_fields: Some("SCREAMING_SNAKE_CASE".to_string()),
        variants: vec![detailed_variant()],
        ..EnumDef::default()
    }
}

fn api_with(enum_def: EnumDef) -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        enums: vec![enum_def],
        ..ApiSurface::default()
    }
}

fn joined_content(files: &[GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
}

/// The public `index.d.ts` overlay — napi's declaration of the union.
fn napi_declaration(enum_def: EnumDef) -> String {
    NapiBackend
        .generate_type_stubs(&api_with(enum_def), &ResolvedCrateConfig::default())
        .map(|files| joined_content(&files))
        .unwrap_or_default()
}

/// wasm emits its structural union as a `typescript_custom_section` inside the generated Rust,
/// so the binding output is where its declaration lives.
fn wasm_declaration(enum_def: EnumDef) -> String {
    WasmBackend
        .generate_bindings(&api_with(enum_def), &ResolvedCrateConfig::default())
        .map(|files| joined_content(&files))
        .unwrap_or_default()
}

/// Both Node-family backends must declare the key serde actually writes, and neither may declare
/// the host spelling. Reverting either backend's `wire_field_name` call to `to_node_name` fails
/// here on the `HOST_SPELLING` assertion, not merely on a disagreement between the two.
#[test]
fn should_declare_the_serde_wire_key_for_an_untagged_struct_variant_in_every_node_family_backend() {
    let wire_key = plain_serde_key();
    assert_eq!(wire_key, FIELD, "serde writes an unrenamed field verbatim");
    assert_eq!(
        serde_json::to_string(&SampleInput::Bare("x".to_string())).expect("serializes"),
        "\"x\"",
        "an untagged newtype variant serializes as its inner value, with no wrapper object"
    );

    for (backend, declaration) in [
        ("napi", napi_declaration(fixture_enum())),
        ("wasm", wasm_declaration(fixture_enum())),
    ] {
        assert!(
            !declaration.is_empty(),
            "{backend} must emit a declaration for the fixture"
        );
        assert!(
            declaration.contains(&format!("{wire_key}: number")),
            "{backend} must declare the serde wire key `{wire_key}`:\n{declaration}"
        );
        assert!(
            !declaration.contains(HOST_SPELLING),
            "{backend} must not declare the host property name `{HOST_SPELLING}` for a value that \
             never passes through a wrapper object:\n{declaration}"
        );
    }
}

/// `rename_all_fields` is the container rule that cases a struct variant's field names;
/// `rename_all` cases variant names and must never reach a field. Both backends must resolve the
/// same key, and it must be the one serde writes — not the enum's `rename_all` applied to a
/// field, and not the host spelling.
#[test]
fn should_apply_rename_all_fields_and_not_rename_all_to_an_untagged_variants_field_keys() {
    let wire_key = renamed_serde_key();
    assert_eq!(
        wire_key, "MAX_CHARS",
        "serde applies rename_all_fields, not rename_all, to a struct variant's fields"
    );

    let napi = napi_declaration(renamed_fixture_enum());
    let wasm = wasm_declaration(renamed_fixture_enum());

    for (backend, declaration) in [("napi", &napi), ("wasm", &wasm)] {
        assert!(
            declaration.contains(&format!("{wire_key}: number")),
            "{backend} must declare `{wire_key}`:\n{declaration}"
        );
        assert!(
            !declaration.contains(HOST_SPELLING),
            "{backend} must not fall back to the host spelling `{HOST_SPELLING}`:\n{declaration}"
        );
        assert!(
            !declaration.contains("max-chars"),
            "{backend} must not apply the enum's variant-name `rename_all` to a field:\n{declaration}"
        );
    }
}

/// The ground truth for the non-identifier key case. `#[serde(rename = "content-type")]` is
/// ordinary serde and its wire name is not a legal TypeScript identifier. Mirrored by
/// [`header_fixture_enum`].
#[derive(serde::Serialize)]
#[serde(untagged)]
enum SampleHeaderInput {
    Detailed {
        #[serde(rename = "content-type")]
        content_type: String,
    },
}

const HEADER_ENUM_NAME: &str = "SampleHeaderInput";

/// The IR mirror of [`SampleHeaderInput`].
fn header_fixture_enum() -> EnumDef {
    EnumDef {
        name: HEADER_ENUM_NAME.to_string(),
        rust_path: format!("sample_core::{HEADER_ENUM_NAME}"),
        has_serde: true,
        serde_untagged: true,
        variants: vec![EnumVariant {
            name: "Detailed".to_string(),
            fields: vec![FieldDef {
                name: "content_type".to_string(),
                ty: TypeRef::String,
                serde_rename: Some("content-type".to_string()),
                ..FieldDef::default()
            }],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }
}

/// Declaring the wire key instead of the host name is only half the fix: a wire name is not an
/// identifier and cannot be interpolated bare. `content-type: string` parses as a subtraction, so
/// the emitted member is a syntax error that takes its whole declaration down with it -- for wasm,
/// the single shared `typescript_custom_section` carrying every union in the crate.
///
/// Both backends resolve the key through `codegen::naming::ts_property_key`, so this fails in
/// both if either stops calling it. Reverting that call emits the bare key and trips the second
/// assertion; reverting the quoting rule inside the renderer trips it for both backends at once.
#[test]
fn should_quote_an_untagged_variant_key_that_is_not_a_typescript_identifier() {
    let wire_key = serde_key_for(
        &serde_json::to_string(&SampleHeaderInput::Detailed {
            content_type: "text/plain".to_string(),
        })
        .expect("serializes"),
    );
    assert_eq!(wire_key, "content-type", "serde writes the renamed key verbatim");

    for (backend, declaration) in [
        ("napi", napi_declaration(header_fixture_enum())),
        ("wasm", wasm_declaration(header_fixture_enum())),
    ] {
        assert!(
            !declaration.is_empty(),
            "{backend} must emit a declaration for the fixture"
        );
        assert!(
            declaration.contains("\"content-type\": string"),
            "{backend} must declare the kebab-case wire key as a quoted property key:\n{declaration}"
        );
        assert!(
            !declaration.contains("content-type: string"),
            "{backend} must not emit the wire key bare -- it is not a legal TypeScript \
             identifier:\n{declaration}"
        );
        assert!(
            !declaration.contains("contentType"),
            "{backend} must not fall back to the host spelling for a value that never passes \
             through a wrapper object:\n{declaration}"
        );
    }
}
