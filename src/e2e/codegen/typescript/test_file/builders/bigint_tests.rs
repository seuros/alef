//! Regression coverage for BigInt literal emission on the wasm target.
//!
//! wasm-bindgen lowers Rust `u64`/`i64` to a JavaScript `bigint`, and the wasm backend's own
//! `.d.ts` emission says so (`backends::wasm::gen_bindings::is_bigint_primitive`, the predicate
//! behind `primitive_ts_type`). The literal emitted for a value of such a field was decided
//! somewhere else entirely — a hand-maintained `[crates.e2e.call].bigint_fields` list in
//! `alef.toml` — so a field the consumer had not listed got a plain `42` assigned to a `bigint`
//! setter: a TypeScript error, and a `TypeError: Cannot convert a Number to a BigInt` at runtime.
//!
//! The second half of the same defect: values past `Number.MAX_SAFE_INTEGER` were routed through
//! `json_to_js`, which rewrote them as `Number("9007199254740993")`. That is a double, so the
//! precision the BigInt exists to preserve was already lost before the `n` suffix was appended.

use super::*;

fn limits_type_def() -> TypeDef {
    TypeDef {
        name: "Limits".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "max_tokens".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "retries".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "unsigned_values".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "signed_values".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::I64))),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "optional_values".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
                optional: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn build(lang: &str, value: serde_json::Value) -> String {
    let type_defs = [limits_type_def()];
    ts_builder_expression(
        value.as_object().expect("object"),
        if lang == "wasm" { "WasmLimits" } else { "Limits" },
        &Default::default(),
        lang,
        &Default::default(),
        // Deliberately empty: the whole point is that the IR alone must be enough.
        &Default::default(),
        &type_defs,
        &[],
        if lang == "wasm" { "Wasm" } else { "" },
        &[],
        &mut Default::default(),
    )
}

#[test]
fn a_wasm_u64_field_gets_a_bigint_literal_without_being_listed_in_bigint_fields() {
    let expression = build("wasm", serde_json::json!({ "max_tokens": 42 }));

    // Positive first: the builder really did emit an assignment for this field.
    assert!(
        expression.contains("_u0.maxTokens = "),
        "the field must be assigned at all before the literal's form is judged: {expression}"
    );
    assert!(
        expression.contains("_u0.maxTokens = 42n;"),
        "a u64 field must receive a BigInt literal: {expression}"
    );
}

#[test]
fn a_wasm_u64_value_past_the_safe_integer_boundary_keeps_every_digit() {
    let expression = build("wasm", serde_json::json!({ "max_tokens": 9_007_199_254_740_993_u64 }));

    assert!(
        expression.contains("_u0.maxTokens = 9007199254740993n;"),
        "the literal must carry the exact integer, not a rounded double: {expression}"
    );
    assert!(
        !expression.contains("Number("),
        "routing through `Number(\"...\")` loses the precision the BigInt exists for: {expression}"
    );
}

/// Negative control on the type axis. `u32` is inside the safe-integer range and wasm-bindgen
/// lowers it to `number`, so a fix that suffixed every integer would pass the tests above and
/// fail here — `3n` assigned to a `number` setter is the same class of error, mirrored.
#[test]
fn a_wasm_u32_field_stays_a_plain_number() {
    let expression = build("wasm", serde_json::json!({ "retries": 3 }));

    assert!(
        expression.contains("_u0.retries = 3;"),
        "a u32 field must keep a plain numeric literal: {expression}"
    );
    assert!(
        !expression.contains("3n"),
        "a u32 field must not be suffixed: {expression}"
    );
}

/// Negative control on the language axis. NAPI marshals `i64`/`u64` as a JS `number`, not a
/// `bigint`, so the node target must be untouched by this rule.
#[test]
fn a_node_u64_field_stays_a_plain_number() {
    let expression = build("node", serde_json::json!({ "max_tokens": 42 }));

    assert!(
        expression.contains("42"),
        "the field must be emitted at all: {expression}"
    );
    assert!(
        !expression.contains("42n"),
        "NAPI takes a number for u64; a BigInt literal would be a type error: {expression}"
    );
}

#[test]
fn wasm_u64_and_i64_collections_use_bigint_typed_arrays_without_precision_loss() {
    let expression = build(
        "wasm",
        serde_json::json!({
            "unsigned_values": [9_007_199_254_740_993_u64, 42_u64],
            "signed_values": [-9_007_199_254_740_993_i64, -7_i64]
        }),
    );

    assert!(
        expression.contains("_u0.unsignedValues = BigUint64Array.from([9007199254740993n, 42n]);"),
        "Vec<u64> must match wasm-bindgen's BigUint64Array setter exactly: {expression}"
    );
    assert!(
        expression.contains("_u0.signedValues = BigInt64Array.from([-9007199254740993n, -7n]);"),
        "Vec<i64> must match wasm-bindgen's BigInt64Array setter exactly: {expression}"
    );
    assert!(
        !expression.contains("Number("),
        "collection lowering must not pass exact integers through Number: {expression}"
    );
}

#[test]
fn an_optional_wasm_u64_collection_uses_the_same_typed_array_lowering() {
    let expression = build(
        "wasm",
        serde_json::json!({ "optional_values": [9_007_199_254_740_993_u64] }),
    );

    assert!(
        expression.contains("_u0.optionalValues = BigUint64Array.from([9007199254740993n]);"),
        "an optional Vec<u64> must preserve the collection's wasm ABI shape: {expression}"
    );
}

#[test]
fn node_big_integer_collections_remain_plain_number_arrays() {
    let expression = build(
        "node",
        serde_json::json!({
            "unsigned_values": [42_u64],
            "signed_values": [-7_i64],
            "optional_values": [8_u64]
        }),
    );

    assert!(
        expression.contains("unsignedValues: [42]")
            && expression.contains("signedValues: [-7]")
            && expression.contains("optionalValues: [8]"),
        "NAPI collections must stay ordinary JS arrays: {expression}"
    );
    assert!(
        !expression.contains("BigUint64Array")
            && !expression.contains("BigInt64Array")
            && !expression.contains("42n")
            && !expression.contains("-7n"),
        "the wasm-only lowering must not leak into Node: {expression}"
    );
}
