//! Table-driven coverage for `function_default::fold_constant_default_functions`: folding a
//! `#[serde(default = "path")]` field's `FunctionCall` down to the literal the named function's
//! body computes, when that body is a single constant-foldable statement.

use super::*;
use crate::core::ir::PrimitiveType;

/// Parse `source`, build the module-scoped indexes exactly as `extractor::mod::extract_items`
/// does, and run `fold_constant_default_functions` over a single synthetic field whose
/// `typed_default` is `FunctionCall(path)` and whose declared type is `field_ty`. Returns the
/// resulting `typed_default` — unchanged when folding was not possible.
fn folded_default(source: &str, path: &str, field_ty: TypeRef) -> DefaultValue {
    let file: syn::File = syn::parse_str(source).expect("valid module source");
    let literal_consts = collect_literal_consts(&file.items);
    let constructors = collect_constructors(&file.items);
    let free_functions = collect_free_functions(&file.items);

    let mut fields = vec![FieldDef {
        name: "field".to_string(),
        ty: field_ty,
        typed_default: Some(DefaultValue::FunctionCall(path.to_string())),
        ..Default::default()
    }];

    fold_constant_default_functions(&mut fields, &free_functions, &constructors, &literal_consts);

    fields.remove(0).typed_default.expect("typed_default is always Some")
}

/// (source, path, field type, expected folded default). Every case here must actually change the
/// value away from `FunctionCall` — the "stays unfolded" cases have their own dedicated tests
/// below, since asserting a *negative* against this table would silently pass if the whole
/// mechanism were disabled.
fn foldable_cases() -> Vec<(&'static str, &'static str, TypeRef, DefaultValue)> {
    vec![
        (
            // The exact shape from `xberg::OcrElement::page_number`: a private, free,
            // zero-argument function next to the struct it defaults for.
            "fn default_page_number() -> u32 { 1 }",
            "default_page_number",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(1),
        ),
        (
            "pub fn default_page_number() -> u32 { 1 }",
            "default_page_number",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(1),
            // Folding must not be gated on visibility: a literal renders in every target
            // language, unlike `PublicFunctionCall`, which only ever produces a real value for
            // the "rust"-emitting backends. See `function_default`'s module doc.
        ),
        (
            r#"fn default_model() -> String { "gpt".to_string() }"#,
            "default_model",
            TypeRef::String,
            DefaultValue::StringLiteral("gpt".to_string()),
        ),
        (
            "fn default_timeout() -> u32 { return 30; }",
            "default_timeout",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(30),
        ),
        (
            r#"
                const DEFAULT_PAGE: u32 = 1;
                fn default_page_number() -> u32 { DEFAULT_PAGE }
            "#,
            "default_page_number",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(1),
        ),
        (
            r#"
                pub struct Settings;
                impl Settings {
                    fn default_retry_limit() -> u32 { 3 }
                }
            "#,
            "Settings::default_retry_limit",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(3),
        ),
        (
            // A fully qualified associated-function path: only the last two segments
            // (`Settings::default_retry_limit`) are resolved, mirroring
            // `postprocess::resolve_public_default_functions`.
            r#"
                pub struct Settings;
                impl Settings {
                    pub fn default_retry_limit() -> u32 { 3 }
                }
            "#,
            "crate::settings::Settings::default_retry_limit",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(3),
        ),
        (
            "fn default_items() -> Vec<u32> { Vec::new() }",
            "default_items",
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
            DefaultValue::Empty,
        ),
    ]
}

#[test]
fn foldable_function_defaults_resolve_to_their_literal_value() {
    for (source, path, field_ty, expected) in foldable_cases() {
        let resolved = folded_default(source, path, field_ty);
        assert_eq!(resolved, expected, "path `{path}` against source:\n{source}");
    }
}

/// A body of more than one statement is refused wholesale, exactly like
/// `mutation::read_struct_body` refuses an unproven `impl Default` shape: a second statement
/// could branch, early-return, or hide a side effect this pass cannot see through.
#[test]
fn multi_statement_body_is_not_folded() {
    let source = r#"
        fn default_page_number() -> u32 {
            let base = 1;
            base
        }
    "#;
    let resolved = folded_default(source, "default_page_number", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(
        resolved,
        DefaultValue::FunctionCall("default_page_number".to_string()),
        "a multi-statement body must be left exactly as `extract_field` recorded it"
    );
}

/// A function whose tail calls another function this pass cannot evaluate must leave the field's
/// original `FunctionCall(path)` untouched — never downgraded to `Unresolved`, which would erase
/// the one thing the field extractor did successfully read (the callable path itself).
#[test]
fn body_that_folds_to_unresolved_keeps_the_original_function_call() {
    let source = "fn computed_value() -> u32 { read_from_disk() }";
    let resolved = folded_default(source, "computed_value", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(
        resolved,
        DefaultValue::FunctionCall("computed_value".to_string()),
        "an unfoldable body must not overwrite the recorded function path with a guess"
    );
}

/// A function taking arguments can never be the real `#[serde(default = "path")]` target — serde
/// itself requires `fn() -> T` — so this pass must not treat an arity mismatch as a resolution.
#[test]
fn function_with_arguments_is_not_folded() {
    let source = "fn default_page_number(seed: u32) -> u32 { seed }";
    let resolved = folded_default(source, "default_page_number", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(resolved, DefaultValue::FunctionCall("default_page_number".to_string()));
}

/// Same reasoning as the arity guard: `async fn` cannot be a real `default = "path"` target.
#[test]
fn async_function_is_not_folded() {
    let source = "async fn default_page_number() -> u32 { 1 }";
    let resolved = folded_default(source, "default_page_number", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(resolved, DefaultValue::FunctionCall("default_page_number".to_string()));
}

/// A path this pass cannot resolve at all (no matching free function or associated function in
/// the module) must leave the field exactly as recorded, not manufacture an `Unresolved`.
#[test]
fn unresolvable_path_is_not_folded() {
    let source = "pub struct Unrelated;";
    let resolved = folded_default(source, "default_page_number", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(resolved, DefaultValue::FunctionCall("default_page_number".to_string()));
}

/// `#[cfg(test)]` functions do not exist in a normal build; folding one's value into a normal
/// build's default would be exactly the fabrication the rest of this module refuses to commit.
#[test]
fn cfg_test_gated_function_is_not_resolved() {
    let source = r#"
        #[cfg(test)]
        fn default_page_number() -> u32 { 1 }
    "#;
    let resolved = folded_default(source, "default_page_number", TypeRef::Primitive(PrimitiveType::U32));
    assert_eq!(
        resolved,
        DefaultValue::FunctionCall("default_page_number".to_string()),
        "a #[cfg(test)]-gated function must never be read as if it were the real default"
    );
}
