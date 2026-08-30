//! SECURITY. `rust_default_via_source_deserialize` (via `default_value_for_field_in_type`) builds
//! a minimal JSON probe object and deserializes it through the owning type's real `Deserialize`
//! impl to recover a `#[serde(default = "path")]` field's true value. Every sibling field must be
//! present in that probe UNLESS serde itself would genuinely fill it when its wire key is absent
//! — its own field-level serde default, a container-level `#[serde(default)]`, or `Option<T>`.
//!
//! The predicate used to also treat `sibling.typed_default.is_some()` as such a signal. That is
//! wrong: `#[derive(Default)]` seeds `typed_default = Some(DefaultValue::Empty)` on *every* field
//! of the container (`extract::extractor::types::extract_struct`), including fields with no
//! serde attribute at all, so the old predicate was true for essentially any field on a
//! `derive(Default)` type. That silently dropped every required sibling from the probe, leaving
//! `placeholders` empty and the caller emitting `serde_json::from_str(r#"{}"#)`, which panics at
//! runtime the first time the type is actually missing a required field. `sibling.default` is the
//! narrower, durable signal `extract::extractor::helpers::fields::extract_field` sets only from a
//! real per-field serde default attribute, unaffected by the later `#[derive(Default)]`/manual
//! `impl Default` overwrite of `typed_default` — see
//! `backends::go::gen_bindings::types::helpers::needs_omitempty_pointer`, which gates the
//! equivalent Go decision on the same two facts and documents `has_default` as inadmissible.
//!
//! Lives in its own module because `tests/defaults.rs` next door is already pinned at the repo's
//! 1,000-line file-size cap by the ratchet baseline and must not grow. ~keep

use super::*;

/// A type shaped like a config struct that derives `Default` and also carries a genuine
/// `#[serde(default = "path")]` field. `target` is always the field being solved for.
///
/// `target` carries both `default` (the raw attribute text) and `typed_default`
/// (`FunctionCall`), mirroring `extract::extractor::helpers::fields::extract_field`, which
/// always sets both together from the same `#[serde(default = "path")]` attribute — never
/// only one. `default.is_some()` is what makes the loop in
/// `rust_default_via_source_deserialize` correctly omit `target` itself from its own probe
/// JSON (it is being solved for, not a placeholder), so a fixture missing it here would
/// silently add a spurious `"count":0` placeholder and invalidate every expected string below.
fn derived_config_type(siblings: Vec<FieldDef>) -> TypeDef {
    let target = FieldDef {
        default: Some("serde(default = \"mylib::default_count\")".to_string()),
        typed_default: Some(DefaultValue::FunctionCall("mylib::default_count".to_string())),
        ..make_field("count", TypeRef::Primitive(PrimitiveType::U32))
    };
    let mut fields = siblings;
    fields.push(target);
    TypeDef {
        name: "DerivedConfig".to_string(),
        rust_path: "mylib::DerivedConfig".to_string(),
        has_default: true,
        has_serde: true,
        fields,
        ..Default::default()
    }
}

fn target_field(typ: &TypeDef) -> &FieldDef {
    typ.fields.iter().find(|f| f.name == "count").unwrap()
}

/// Pins the bug this fix targets. `label` is a required, non-`Option` field with no serde
/// attribute of its own — exactly what every plain field on a `derive(Default)` struct looks
/// like once extraction seeds `typed_default = Some(Empty)` for it. It must still be emitted
/// into the probe JSON with a real placeholder, not silently omitted.
///
/// Against the predicate this fix replaces (`sibling.typed_default.is_some() ||
/// sibling.default.is_some()`), `label` has `typed_default = Some(Empty)` and would be treated
/// as already-defaulted, omitting it from the probe entirely and rendering
/// `serde_json::from_str::<mylib::DerivedConfig>(r#"{}"#)...count` instead of the value
/// asserted below — the exact empty-object probe that panics at runtime on real construction.
#[test]
fn derive_default_seeded_empty_is_not_treated_as_a_serde_default() {
    let label = FieldDef {
        typed_default: Some(DefaultValue::Empty),
        ..make_field("label", TypeRef::String)
    };
    let typ = derived_config_type(vec![label]);

    let rendered = default_value_for_field_in_type(target_field(&typ), "rust", &typ);

    assert_eq!(
        rendered,
        "serde_json::from_str::<mylib::DerivedConfig>(r#\"{\"label\":\"\"}\"#)\
         .expect(\"alef-generated default JSON for `DerivedConfig` failed to deserialize\").count",
        "a required sibling seeded `Empty` only by `#[derive(Default)]` must still get a \
         placeholder, not be dropped from the probe: {rendered}"
    );
}

/// A container-level `#[serde(default)]` makes every field on the type absent-tolerant, so a
/// sibling with no attribute of its own may still be omitted from the probe.
#[test]
fn container_level_serde_default_omits_an_unmarked_sibling() {
    let label = make_field("label", TypeRef::String);
    let mut typ = derived_config_type(vec![label]);
    typ.serde_container_default = true;

    let rendered = default_value_for_field_in_type(target_field(&typ), "rust", &typ);

    assert_eq!(
        rendered,
        "serde_json::from_str::<mylib::DerivedConfig>(r#\"{}\"#)\
         .expect(\"alef-generated default JSON for `DerivedConfig` failed to deserialize\").count",
        "a container-level `#[serde(default)]` must omit every sibling, including one with no \
         attribute of its own: {rendered}"
    );
}

/// A field's own `#[serde(default = "path")]` is the genuine signal `sibling.default` records;
/// that sibling may be omitted from the probe.
#[test]
fn field_level_named_serde_default_omits_its_own_sibling() {
    let user_agent = FieldDef {
        default: Some("serde(default = \"mylib::default_user_agent\")".to_string()),
        typed_default: Some(DefaultValue::FunctionCall("mylib::default_user_agent".to_string())),
        ..make_field("user_agent", TypeRef::String)
    };
    let typ = derived_config_type(vec![user_agent]);

    let rendered = default_value_for_field_in_type(target_field(&typ), "rust", &typ);

    assert_eq!(
        rendered,
        "serde_json::from_str::<mylib::DerivedConfig>(r#\"{}\"#)\
         .expect(\"alef-generated default JSON for `DerivedConfig` failed to deserialize\").count",
        "a field carrying its own named serde default must be omitted from the probe: {rendered}"
    );
}

/// `Option<T>` fields are always absent-tolerant regardless of any serde default attribute.
#[test]
fn optional_sibling_is_omitted_regardless_of_default() {
    let nickname = FieldDef {
        optional: true,
        ..make_field("nickname", TypeRef::Optional(Box::new(TypeRef::String)))
    };
    let typ = derived_config_type(vec![nickname]);

    let rendered = default_value_for_field_in_type(target_field(&typ), "rust", &typ);

    assert_eq!(
        rendered,
        "serde_json::from_str::<mylib::DerivedConfig>(r#\"{}\"#)\
         .expect(\"alef-generated default JSON for `DerivedConfig` failed to deserialize\").count",
        "an Option<T> sibling must be omitted from the probe: {rendered}"
    );
}

/// When the narrowed predicate correctly treats a nested named-type sibling as genuinely
/// required (no own serde default, not optional, not covered by a container default), and that
/// type has no safe JSON placeholder, generation must fail loudly rather than silently emit an
/// empty-object probe that panics at runtime on the first real construction.
#[test]
fn required_nested_named_sibling_fails_generation_instead_of_probing_empty() {
    let owner = FieldDef {
        typed_default: Some(DefaultValue::Empty),
        ..make_field("owner", TypeRef::Named("Author".to_string()))
    };
    let typ = derived_config_type(vec![owner]);

    let message = default_value_for_field_in_type(target_field(&typ), "rust", &typ);

    assert!(
        message.starts_with("compile_error!"),
        "a required sibling with no safe placeholder must fail generation, not probe `{{}}`: {message}"
    );
    for needle in ["mylib", "DerivedConfig", "count", "mylib::default_count"] {
        assert!(
            message.contains(needle),
            "the failure must name `{needle}` so the author can act on it: {message}"
        );
    }
}
