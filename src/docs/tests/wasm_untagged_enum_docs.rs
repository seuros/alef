//! Regression coverage for the WASM docs/binding disagreement on `#[serde(untagged)]` data
//! enums.
//!
//! Before this fix, `render_enum` rendered a `| Value | Description |` table for every enum
//! regardless of language or lowering, framing it as a fixed set of named, referenceable
//! values. For a WASM untagged data enum (e.g. `EmbeddingInput`) that framing was never true:
//! `enums::gen_enum` is never called for these (see its own `~keep` note in
//! `backends/wasm/gen_bindings/enums.rs`), so there is no `Wasm{Enum}` class or member a JS/TS
//! caller could reference by name. Since `252096144` the WASM binding instead emits a real
//! structural TypeScript union via `ts_union.rs`, but the docs page never consulted that
//! lowering decision -- it kept rendering the same misleading Value/Description table alongside
//! it. These tests pin the fix: the docs page must embed the SAME text
//! `docs_ts_type_for_untagged_enum` computes (the exact function the WASM backend calls to emit
//! the `.d.ts`), not a second, independently-derived description of it.

use super::*;
use crate::core::ir::EnumVariant;

/// `enum EmbeddingInput { Single(String), Multiple(Vec<String>) }` with `#[serde(untagged)]` --
/// mirrors the fixture already used for the WASM backend's own untagged-enum coverage in
/// `backends/wasm/gen_bindings/untagged_enum_tests.rs`.
fn embedding_input_enum() -> EnumDef {
    EnumDef {
        name: "EmbeddingInput".to_string(),
        rust_path: "test_lib::EmbeddingInput".to_string(),
        variants: vec![
            EnumVariant {
                name: "Single".to_string(),
                fields: vec![make_field("_0", TypeRef::String, false, None)],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Multiple".to_string(),
                fields: vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)), false, None)],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        doc: "Text or a batch of text.".to_string(),
        ..Default::default()
    }
}

fn embedding_request_type() -> TypeDef {
    TypeDef {
        name: "EmbeddingRequest".to_string(),
        rust_path: "test_lib::EmbeddingRequest".to_string(),
        fields: vec![make_field(
            "input",
            TypeRef::Named("EmbeddingInput".to_string()),
            false,
            None,
        )],
        has_serde: true,
        doc: "Request body.".to_string(),
        ..empty_type("EmbeddingRequest")
    }
}

fn config_for_languages(languages: &str) -> ResolvedCrateConfig {
    config_from_toml(&format!(
        r#"
[workspace]
languages = {languages}

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#
    ))
}

#[test]
fn wasm_docs_embed_the_actual_ts_union_not_a_value_table() {
    let config = config_for_languages(r#"["wasm"]"#);
    let mut api = make_minimal_api("1.0.0");
    api.enums = vec![embedding_input_enum()];
    api.types = vec![embedding_request_type()];

    // Derive the expected text from the SAME shared function the WASM backend itself calls to
    // emit the `.d.ts` (see `backends/wasm/gen_bindings/mod.rs`'s `untagged_ts_plan` /
    // `ts_union::build_untagged_enum_ts_plan_for_api`) -- pins the docs page against that
    // decision instead of restating it.
    let expected_ts = crate::backends::wasm::docs_ts_type_for_untagged_enum(&api.enums[0], &api, &config)
        .expect("EmbeddingInput must lower to a structural TS union in the WASM binding");
    assert_eq!(
        expected_ts, "export type WasmEmbeddingInput = string | string[];",
        "sanity-check the shared function's own output before asserting the docs page against it"
    );

    let files = generate_docs(&api, &config, &[Language::Wasm], "out").unwrap();
    let wasm = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-wasm"))
        .unwrap();

    assert!(
        wasm.content.contains(&expected_ts),
        "the WASM docs page must embed the exact structural TS union the backend computes:\n{}",
        wasm.content
    );
    assert!(
        !wasm.content.contains("| Value | Description |"),
        "an untagged data enum has no named, referenceable values in the WASM binding -- the \
         docs page must not render the Value/Description table for it:\n{}",
        wasm.content
    );
    assert!(
        !wasm.content.contains("`Single`"),
        "the old per-variant table framed variant names as importable/referenceable values, \
         which is not true for a WASM untagged data enum -- it must not appear in the WASM \
         page:\n{}",
        wasm.content
    );
}

#[test]
fn other_language_docs_are_unaffected_and_keep_the_per_variant_value_table() {
    let config = config_for_languages(r#"["python"]"#);
    let mut api = make_minimal_api("1.0.0");
    api.enums = vec![embedding_input_enum()];
    api.types = vec![embedding_request_type()];

    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let python = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();

    assert!(
        python.content.contains("| Value | Description |"),
        "a non-WASM language's docs page must keep its existing per-variant table -- this fix \
         is WASM-specific:\n{}",
        python.content
    );
    assert!(
        python.content.contains("`SINGLE`"),
        "Python's own per-variant framing must be unaffected:\n{}",
        python.content
    );
}

#[test]
fn fieldless_wasm_enum_still_uses_the_value_table() {
    // A genuinely fieldless enum keeps its `Wasm{Enum}` C-style representation (see
    // `enums::gen_enum`), so its docs page must be unaffected by this change: it really does
    // have a fixed set of named, referenceable values.
    let config = config_for_languages(r#"["wasm"]"#);
    let mut api = make_minimal_api("1.0.0");
    api.enums = vec![EnumDef {
        name: "Role".to_string(),
        rust_path: "test_lib::Role".to_string(),
        variants: vec![
            EnumVariant {
                name: "User".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Assistant".to_string(),
                ..Default::default()
            },
        ],
        has_serde: true,
        is_copy: true,
        ..Default::default()
    }];

    let files = generate_docs(&api, &config, &[Language::Wasm], "out").unwrap();
    let wasm = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-wasm"))
        .unwrap();

    assert!(
        wasm.content.contains("| Value | Description |"),
        "a fieldless enum keeps its WASM C-style enum representation and must keep the \
         Value/Description table:\n{}",
        wasm.content
    );
}

#[test]
fn docs_ts_type_for_untagged_enum_returns_none_when_not_lowered_to_a_union() {
    let config = config_for_languages(r#"["wasm"]"#);
    let api = make_minimal_api("1.0.0");

    let mut fieldless = embedding_input_enum();
    for variant in &mut fieldless.variants {
        variant.fields.clear();
        variant.is_tuple = false;
    }
    assert!(
        crate::backends::wasm::docs_ts_type_for_untagged_enum(&fieldless, &api, &config).is_none(),
        "a fieldless untagged enum has nothing to lose to JsValue and keeps the ordinary \
         wasm-bindgen C-style enum -- it must not get a TS union note"
    );

    let mut internally_tagged = embedding_input_enum();
    internally_tagged.serde_untagged = false;
    internally_tagged.serde_tag = Some("type".to_string());
    assert!(
        crate::backends::wasm::docs_ts_type_for_untagged_enum(&internally_tagged, &api, &config).is_none(),
        "an internally-tagged data enum takes the discriminator-struct path, not the \
         structural-union path"
    );
}
