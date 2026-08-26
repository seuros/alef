//! Regression coverage for `emit_test_backend`'s `wasm_type_prefix` parameter: a trait-bridge
//! stub method returning a named enum must annotate and cast through the enum's *exported*
//! identifier -- bare for node (napi-rs does not prefix its types), prefixed for wasm (every
//! wasm-bindgen-emitted enum lives under the crate's binding-class prefix, e.g.
//! `WasmProcessingStage`, never the bare IR name -- see `gen_struct`/`gen_opaque_struct` in
//! `backends/wasm/gen_bindings/types.rs`).
//!
//! Before this fix `emit_ts_stub_method` ignored the target language entirely and always emitted
//! the bare IR name, so a wasm trait-bridge stub referenced a type the wasm package never
//! exports under that name (`Cannot find name 'ProcessingStage'` at `tsc`, since the actual
//! export is `WasmProcessingStage`).
//!
//! Split into its own file (rather than the inline `mod tests` block in `mod.rs`) to keep
//! `mod.rs` under the crate's 1,000-line file cap.

use super::{emit_test_backend, test_method};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};

fn make_fixture(id: &str, input: serde_json::Value) -> crate::e2e::fixture::Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "test fixture",
        "input": input,
        "assertions": []
    }))
    .expect("minimal fixture JSON must parse")
}

/// A trait method returning a `Named` enum (e.g. `PostProcessor::processing_stage()
/// -> ProcessingStage`) must be annotated with the real enum type, not the generic
/// `string` fallback, and its stub body must return a value that actually satisfies
/// that type at the napi-rs bridge boundary.
///
/// The bridge coerces the JS return value to a string and parses it with
/// `serde_json::from_str`, so the emitted body must return the JSON-quoted variant
/// name (`"Early"`, quotes included) cast through `unknown` to satisfy the nominal
/// enum return annotation — see `emit_ts_stub_method`.
#[test]
fn emit_test_backend_ts_named_enum_return_uses_enum_type_and_valid_variant() {
    let bridge = TraitBridgeConfig {
        trait_name: "PostProcessor".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let processing_stage = test_method(
        "processingStage",
        TypeRef::Named("ProcessingStage".to_string()),
        false,
        false,
    );
    let methods = [&processing_stage];

    let enums = [EnumDef {
        name: "ProcessingStage".to_string(),
        variants: vec![
            EnumVariant {
                name: "Early".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Middle".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Late".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let fixture = make_fixture("enum_return_fixture", serde_json::json!({ "name": "my-processor" }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, &enums, "");

    assert!(
        emission
            .setup_block
            .contains("processingStage(): ProcessingStage { return \"\\\"Early\\\"\" as unknown as ProcessingStage; }"),
        "Named enum return must be annotated with the real enum type and return a \
         JSON-quoted first variant cast to that type, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("processingStage(): string"),
        "Named enum return must not fall through to the generic 'string' annotation, got: {}",
        emission.setup_block
    );
    assert_eq!(
        emission.type_imports,
        vec!["ProcessingStage".to_string()],
        "the enum type must be recorded so callers can wire up the import"
    );
}

/// wasm control for the node test above: every wasm-bindgen-emitted enum lives under the
/// crate's binding-class prefix (`WasmProcessingStage`, not `ProcessingStage` — see
/// `emit_test_backend`'s `wasm_type_prefix` doc), so the stub's return-type annotation, its
/// runtime cast, and the recorded import must all reference the prefixed name.
#[test]
fn emit_test_backend_ts_wasm_named_enum_return_uses_prefixed_enum_type() {
    let bridge = TraitBridgeConfig {
        trait_name: "PostProcessor".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let processing_stage = test_method(
        "processingStage",
        TypeRef::Named("ProcessingStage".to_string()),
        false,
        false,
    );
    let methods = [&processing_stage];

    let enums = [EnumDef {
        name: "ProcessingStage".to_string(),
        variants: vec![EnumVariant {
            name: "Early".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let fixture = make_fixture(
        "wasm_enum_return_fixture",
        serde_json::json!({ "name": "my-processor" }),
    );

    let emission = emit_test_backend(&bridge, &methods, &fixture, &enums, "Wasm");

    assert!(
        emission.setup_block.contains(
            "processingStage(): WasmProcessingStage { return \"\\\"Early\\\"\" as unknown as WasmProcessingStage; }"
        ),
        "wasm stub must annotate and cast through the prefixed class, got: {}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("as unknown as ProcessingStage;"),
        "wasm stub must not reference the bare (unexported) enum name, got: {}",
        emission.setup_block
    );
    assert_eq!(
        emission.type_imports,
        vec!["WasmProcessingStage".to_string()],
        "the recorded import must match the exact identifier referenced in the stub body"
    );
}
