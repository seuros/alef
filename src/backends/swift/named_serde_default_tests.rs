//! SECURITY. `emit_decoder_init` decides whether an absent JSON key is filled with a Swift
//! type-based zero (`[]`, `[:]`, `0`) or left to throw a visible `DecodingError`. A field carrying
//! `#[serde(default = "path")]` reaches the IR as `DefaultValue::FunctionCall` — alef records the
//! function's *name*, never its return value — so the zero is a claim about a value alef does not
//! have, and the emitter correctly declines it.
//!
//! That refusal was unreachable in practice. `extract::extractor::types` blanket-overwrote every
//! field's `typed_default` with `DefaultValue::Empty` whenever the container derived `Default`,
//! and `Empty` licenses the type-based zero. A named allow-list or deny-list default therefore
//! decoded to `[]` — an allow-list that permits nothing, or a deny-list that fails open. These
//! tests pin the deferral against the IR shape the extractor now produces.
//!
//! Lives here rather than in `gen_bindings/dto.rs`'s inline `mod tests` because that file is at
//! its recorded file-size ceiling (`tests/file_size_baseline.txt`) and may not grow, as is
//! `gen_bindings/mod.rs` at the 1,000-line cap. ~keep

use crate::backends::swift::gen_bindings::dto::emit_decoder_init;
use crate::backends::swift::type_map::SwiftMapper;
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeRef};

fn decode_body(name: &str, ty: TypeRef, typed_default: DefaultValue) -> String {
    let field = FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(typed_default),
        ..Default::default()
    };
    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &mut out);
    out
}

#[test]
fn a_named_serde_default_on_a_vec_defers_to_rust_instead_of_decoding_an_empty_array() {
    let out = decode_body(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::FunctionCall("default_scheme_allowlist".to_string()),
    );

    assert!(
        !out.contains("?? []"),
        "alef never evaluates default_scheme_allowlist(); `?? []` decodes an absent key into an \
         empty allow-list that permits nothing:\n{out}"
    );
    assert!(
        out.contains("try container.decode([String].self, forKey: .schemeAllowlist)"),
        "an unreadable default must leave the key required so an absent key is a visible \
         DecodingError:\n{out}"
    );
}

#[test]
fn a_named_serde_default_on_a_map_defers_to_rust_instead_of_decoding_an_empty_dictionary() {
    let out = decode_body(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::PublicFunctionCall("sample_crate::Policy::header_overrides".to_string()),
    );

    assert!(
        !out.contains("?? [:]"),
        "a resolved function-call default is still a value alef has not read:\n{out}"
    );
}

/// Discrimination control for both tests above. `Empty` genuinely IS `Default::default()`, so the
/// empty array and empty dictionary are exact for it and must still be emitted. Without this, a
/// change that stopped emitting `??` fallbacks for collections at all would satisfy the assertions
/// above while stripping the fallback off every ordinary `#[derive(Default)]` field. ~keep
#[test]
fn an_empty_default_still_decodes_to_the_swift_collection_zero() {
    let vec_out = decode_body(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        vec_out.contains("?? []"),
        "`Empty` is the type's own default and keeps the empty-array fallback:\n{vec_out}"
    );

    let map_out = decode_body(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        map_out.contains("?? [:]"),
        "`Empty` keeps the empty-dictionary fallback:\n{map_out}"
    );

    let scalar_out = decode_body(
        "redirect_limit",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::Empty,
    );
    assert!(
        scalar_out.contains("?? 0"),
        "`Empty` keeps the scalar zero fallback:\n{scalar_out}"
    );
}
