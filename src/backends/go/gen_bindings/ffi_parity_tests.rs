//! Parity between what Go *calls* across the C ABI and what the FFI backend *exports*.
//!
//! Go is a consumer of symbols and lowerings the FFI backend decides. Every fact in this file
//! is one both backends must agree on, asserted against the shared helper the FFI backend
//! itself uses rather than against a string Go happens to produce today.

use super::functions::gen_function_wrapper;
use super::methods::gen_method_wrapper;
use crate::codegen::c_consumer;
use crate::core::ir::{FunctionDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

const PREFIX: &str = "demo";

/// Type names whose snake spelling discriminates between `heck::ToSnakeCase` and the acronym-aware
/// `pascal_to_snake` the ABI helpers apply: consecutive capitals, an embedded acronym, digit
/// boundaries, a single-letter leading segment, and a leading underscore. ~keep
const ADVERSARIAL_TYPE_NAMES: &[&str] = &[
    "HTTPServer",
    "URLPath",
    "UTF8Length",
    "Base64Encode",
    "AClient",
    "JSONLD",
    "_Internal",
];

/// Method/function names whose snake spelling discriminates between `heck::ToSnakeCase` and the
/// verbatim method component the FFI backend interpolates. ~keep
const ADVERSARIAL_ITEM_NAMES: &[&str] = &[
    "parseURLPath",
    "utf8Length",
    "Base64Encode",
    "_hidden",
    "parse__inner",
    "a",
    "to_json",
];

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        is_opaque: true,
        ..Default::default()
    }
}

fn instance_method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn free_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn generate_method(typ: &TypeDef, method: &MethodDef) -> String {
    let opaque: HashSet<&str> = HashSet::new();
    let names: HashSet<String> = HashSet::new();
    gen_method_wrapper(typ, method, PREFIX, &opaque, &names, &names, &names)
}

fn generate_function(func: &FunctionDef) -> String {
    let opaque: HashSet<&str> = HashSet::new();
    let names: HashSet<String> = HashSet::new();
    gen_function_wrapper(func, PREFIX, &opaque, &names, &names, &names, &names, &names, &names)
}

/// Every `C.<symbol>(` call site in the generated Go source.
fn called_c_symbols(generated: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut rest = generated;
    while let Some(at) = rest.find("C.") {
        rest = &rest[at + 2..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let (symbol, tail) = rest.split_at(end);
        if tail.starts_with('(') && !symbol.is_empty() {
            symbols.push(symbol.to_string());
        }
        rest = tail;
    }
    symbols
}

/// The Go type declared in the emitted `func ...(...) <type> {` signature line.
fn declared_return_type(generated: &str) -> String {
    let signature = generated
        .lines()
        .find(|line| line.starts_with("func "))
        .unwrap_or_else(|| panic!("no function signature in:\n{generated}"));
    signature
        .rsplit_once(')')
        .unwrap_or_else(|| panic!("malformed signature `{signature}`"))
        .1
        .trim_end_matches('{')
        .trim()
        .to_string()
}

/// The Go type the emitted return *expression* actually produces, read back out of the
/// `func() <type> { ... }()` closure the conversion is wrapped in.
fn expression_result_type(generated: &str) -> String {
    let return_line = generated
        .lines()
        .find(|line| line.trim_start().starts_with("return func() "))
        .unwrap_or_else(|| panic!("no closure-wrapped return in:\n{generated}"));
    let after = return_line
        .trim_start()
        .strip_prefix("return func() ")
        .expect("checked above");
    after
        .split_once(" {")
        .unwrap_or_else(|| panic!("malformed closure `{return_line}`"))
        .0
        .to_string()
}

/// The wrapper's value-producing return, which is its last one. A scalar `Option` return is
/// preceded by the presence gate's early `return nil`, so taking the first `return` would
/// assert against the absent branch rather than the lowering under test. ~keep
fn return_line(generated: &str) -> String {
    generated
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("return "))
        .unwrap_or_else(|| panic!("no return statement in:\n{generated}"))
        .trim()
        .to_string()
}

#[test]
fn should_call_the_exported_symbol_when_a_method_name_defeats_snake_casing() {
    for type_name in ADVERSARIAL_TYPE_NAMES {
        for method_name in ADVERSARIAL_ITEM_NAMES {
            let typ = opaque_type(type_name);
            let method = instance_method(method_name, TypeRef::Unit);
            let generated = generate_method(&typ, &method);

            let exported = c_consumer::method_symbol(PREFIX, type_name, method_name);
            assert!(
                called_c_symbols(&generated).contains(&exported),
                "`{type_name}::{method_name}` calls {:?}, but the FFI backend exports `{exported}`",
                called_c_symbols(&generated)
            );
        }
    }
}

#[test]
fn should_call_the_exported_symbol_when_a_function_name_defeats_snake_casing() {
    for function_name in ADVERSARIAL_ITEM_NAMES {
        let func = free_function(function_name, TypeRef::Unit);
        let generated = generate_function(&func);

        let exported = c_consumer::free_function_symbol(PREFIX, function_name);
        assert!(
            called_c_symbols(&generated).contains(&exported),
            "`{function_name}` calls {:?}, but the FFI backend exports `{exported}`",
            called_c_symbols(&generated)
        );
    }
}

/// Control for the two parity tests above: they are only meaningful if the rows actually
/// separate the two derivations. Go used to build both symbols by snake-casing each component
/// with `heck`; on a table of ordinary `snake_case` names that spells the same string as the ABI
/// helpers, so the parity tests would pass without proving anything. This asserts the emitted
/// symbol is *not* the one the old derivation produced. ~keep
#[test]
fn should_not_spell_the_symbol_the_way_component_wise_snake_casing_would() {
    let typ = opaque_type("HTTPServer");
    let method = instance_method("parseURLPath", TypeRef::Unit);
    let generated = generate_method(&typ, &method);

    let old_derivation = format!(
        "{PREFIX}_{}_{}",
        "HTTPServer".to_snake_case(),
        "parseURLPath".to_snake_case()
    );
    assert_eq!(old_derivation, "demo_http_server_parse_url_path");
    assert_eq!(
        c_consumer::method_symbol(PREFIX, "HTTPServer", "parseURLPath"),
        "demo_http_server_parseURLPath"
    );
    assert!(
        !called_c_symbols(&generated).contains(&old_derivation),
        "still emitting the component-wise snake-cased symbol, which is exported nowhere"
    );
}

/// `Option<Duration>` crosses the C ABI as a bare `u64` of milliseconds — the same shape as
/// `Option<primitive>`, per `backends::ffi::type_map::optional_return_crosses_as_scalar`. Go used
/// to fall through to the `unmarshal{TypeName}` catch-all and emit `unmarshalU64(ptr)`, a helper
/// the generated package never declares. ~keep
#[test]
fn should_convert_an_optional_duration_as_a_scalar_when_lowering_the_return() {
    let func = free_function("get_timeout", TypeRef::Optional(Box::new(TypeRef::Duration)));
    let generated = generate_function(&func);

    assert_eq!(declared_return_type(&generated), "*uint64");
    assert_eq!(
        return_line(&generated),
        "return func() *uint64 { v := uint64(ptr); return &v }()"
    );
    assert_eq!(
        declared_return_type(&generated),
        expression_result_type(&generated),
        "declared return type and return expression disagree"
    );
    assert!(
        !generated.contains("unmarshalU64"),
        "still calling the undeclared unmarshalU64 helper:\n{generated}"
    );
}

/// A bare `Duration` return is the same defect one level up: `c_return_type` maps it to `u64`,
/// so the value is a scalar, not something to unmarshal. ~keep
#[test]
fn should_convert_a_bare_duration_as_a_scalar_when_lowering_the_return() {
    let func = free_function("get_elapsed", TypeRef::Duration);
    let generated = generate_function(&func);

    assert_eq!(declared_return_type(&generated), "uint64");
    assert_eq!(return_line(&generated), "return uint64(ptr)");
    assert!(
        !generated.contains("unmarshalU64"),
        "still calling the undeclared unmarshalU64 helper:\n{generated}"
    );
}

/// A C function returns one value, so `Option<Option<T>>` reaches Go with a single level of
/// nullability. The return expression has always produced one pointer; the *declaration* used to
/// say `**int64`, a type nothing in the generated file can produce. Asserting only the
/// declaration — or only the expression — would have passed while they disagreed. ~keep
#[test]
fn should_declare_one_pointer_level_when_a_nested_option_returns_a_primitive() {
    let func = free_function(
        "find_offset",
        TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
            PrimitiveType::I64,
        ))))),
    );
    let generated = generate_function(&func);

    assert_eq!(declared_return_type(&generated), "*int64");
    assert_eq!(
        return_line(&generated),
        "return func() *int64 { v := int64(ptr); return &v }()"
    );
    assert_eq!(
        declared_return_type(&generated),
        expression_result_type(&generated),
        "declared return type and return expression disagree"
    );
    assert!(
        !generated.contains("**int64"),
        "still declaring a double pointer:\n{generated}"
    );
}

/// The nested-option collapse must not leak into the single-level case, which really is one
/// pointer's worth of nullability and already agreed. ~keep
#[test]
fn should_keep_one_pointer_level_when_a_single_option_returns_a_primitive() {
    let func = free_function(
        "find_index",
        TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
    );
    let generated = generate_function(&func);

    assert_eq!(declared_return_type(&generated), "*int64");
    assert_eq!(declared_return_type(&generated), expression_result_type(&generated));
}
