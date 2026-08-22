//! IR-derived `fields_c_types` inference for scalar-primitive leaf fields (`bool`, integers,
//! floats).
//!
//! Mirrors `enum_field_inference`'s rationale for the other class of field an operator's
//! `fields_c_types` commonly forgets to declare: a plain scalar like `used_cache: bool`.
//! Nothing about a bare `bool`/`u32`/`f64` field looks like it needs a `fields_c_types` entry,
//! so it is easy to leave undeclared. Without one, `emit_nested_accessor`'s leaf arm and its
//! non-nested sibling in `test_function.rs` both still emit the `char*` default even though the
//! FFI accessor actually returns a scalar (`bool` crosses the ABI as `int32_t` — see
//! `backends::ffi::trait_bridge::FfiBridgeGenerator::c_param_type`), so every assertion against
//! the field is built against a local the compiler declared `char*` for a value that is really
//! an `int32_t`: `render_assertion`'s `equals` arm falls through to its `strcmp` default,
//! comparing a pointer-typed local against an integer literal — an undeclared `bool` leaf field's
//! equals-assertion segfaults at runtime because `strcmp`'s second argument is never a valid
//! `const char*`. ~keep
use heck::ToSnakeCase;
use std::collections::HashMap;

use crate::core::ir::{TypeDef, TypeRef};

use super::trait_bridge_snippet::c_type;

/// Peel `Optional` wrappers down to the inner type reference, mirroring
/// `enum_field_inference::peel_optional`: a C leaf accessor is emitted identically for a
/// primitive and `Option<primitive>` (the FFI already collapses "absent" into a sentinel), so
/// optionality must not hide a field's real shape from this inference either.
fn peel_optional(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => peel_optional(inner),
        other => other,
    }
}

/// Derive `fields_c_types`-shaped entries directly from the IR struct definitions, for every
/// field whose declared Rust type (after peeling `Option`) is a scalar primitive.
///
/// Returned entries are proposals, not the sole source of truth: the caller unions them under
/// any operator-declared entry via `HashMap::entry().or_insert()`, exactly as
/// `enum_fields_c_types_from_ir`'s entries are unioned — an explicit `fields_c_types` override
/// always wins. ~keep
pub(super) fn primitive_fields_c_types_from_ir(type_defs: &[TypeDef]) -> HashMap<String, String> {
    let mut derived = HashMap::new();
    for type_def in type_defs {
        let parent_snake = type_def.name.to_snake_case();
        for field in &type_def.fields {
            let primitive_ty = peel_optional(&field.ty);
            if !matches!(primitive_ty, TypeRef::Primitive(_)) {
                continue;
            }
            derived.insert(
                format!("{parent_snake}.{}", field.name.to_snake_case()),
                c_type(primitive_ty),
            );
        }
    }
    derived
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, PrimitiveType};

    /// Regression: an undeclared `bool` leaf field must not fall through to the `char*`
    /// default. `FetchResult.used_cache` is a real `bool` field, but
    /// `[crates.e2e.fields_c_types]` never declared `"fetch_result.used_cache"`. Without
    /// IR-derived inference, the leaf lookup falls through to the `char*` default even
    /// though the accessor returns `int32_t`.
    #[test]
    fn primitive_fields_c_types_from_ir_recovers_an_undeclared_bool_leaf_field() {
        let type_defs = vec![TypeDef {
            name: "FetchResult".into(),
            fields: vec![FieldDef {
                name: "used_cache".into(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];

        let derived = primitive_fields_c_types_from_ir(&type_defs);

        assert_eq!(
            derived.get("fetch_result.used_cache").map(String::as_str),
            Some("int32_t"),
            "got: {derived:?}"
        );
    }

    /// `Option<bool>` must be recognized the same as a bare `bool` field: the FFI accessor
    /// returns the same sentinel-scalar shape either way.
    #[test]
    fn primitive_fields_c_types_from_ir_sees_through_optional() {
        let type_defs = vec![TypeDef {
            name: "FetchResult".into(),
            fields: vec![FieldDef {
                name: "used_cache".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];

        let derived = primitive_fields_c_types_from_ir(&type_defs);

        assert_eq!(
            derived.get("fetch_result.used_cache").map(String::as_str),
            Some("int32_t")
        );
    }

    /// Every non-bool primitive maps to the real cbindgen stdint spelling, not a guessed name.
    #[test]
    fn primitive_fields_c_types_from_ir_maps_every_primitive_kind() {
        let type_defs = vec![TypeDef {
            name: "Metrics".into(),
            fields: vec![
                FieldDef {
                    name: "retries".into(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "score".into(),
                    ty: TypeRef::Primitive(PrimitiveType::F64),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        }];

        let derived = primitive_fields_c_types_from_ir(&type_defs);

        assert_eq!(derived.get("metrics.retries").map(String::as_str), Some("uint32_t"));
        assert_eq!(derived.get("metrics.score").map(String::as_str), Some("double"));
    }

    /// A field whose type is a struct/enum (`Named`) must not be swept in — only genuinely
    /// scalar-primitive fields get an inferred entry, mirroring
    /// `enum_fields_c_types_from_ir_ignores_non_enum_named_fields`.
    #[test]
    fn primitive_fields_c_types_from_ir_ignores_named_fields() {
        let type_defs = vec![TypeDef {
            name: "FetchResult".into(),
            fields: vec![FieldDef {
                name: "usage".into(),
                ty: TypeRef::Named("Usage".into()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];

        let derived = primitive_fields_c_types_from_ir(&type_defs);

        assert!(derived.is_empty(), "got: {derived:?}");
    }

    /// End-to-end through the real generator entry point (`render_test_file`, not a hand-rolled
    /// stand-in): with zero `fields_c_types` config, a `bool` leaf field's `equals` assertion
    /// must declare the local as the real ABI-accurate scalar type and compare it directly —
    /// never fall through to `strcmp` against an accessor the compiler declared `char*` for a
    /// value that is actually an `int32_t`.
    #[test]
    fn equals_assertion_on_an_undeclared_bool_field_compares_the_scalar_not_strcmp() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::{FunctionDef, PrimitiveType as Prim};
        use crate::e2e::codegen::call_ir::CallIr;
        use crate::e2e::config::E2eConfig;
        use crate::e2e::field_access::FieldResolver;
        use crate::e2e::fixture::{Assertion, Fixture};

        let fixture = Fixture {
            id: "fetch_used_cache".into(),
            description: "Fetch reports whether the cache was used".into(),
            assertions: vec![Assertion {
                assertion_type: "equals".into(),
                field: Some("used_cache".into()),
                value: Some(serde_json::json!(true)),
                ..Default::default()
            }],
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "fetch".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let type_defs = vec![TypeDef {
            name: "FetchResult".into(),
            fields: vec![FieldDef {
                name: "used_cache".into(),
                ty: TypeRef::Primitive(Prim::Bool),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let functions = [FunctionDef {
            name: "fetch".into(),
            return_type: TypeRef::Named("FetchResult".into()),
            ..FunctionDef::default()
        }];
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );

        let rendered = super::super::render_test_file(
            "fetching",
            &[&fixture],
            "sample.h",
            "sample",
            "result",
            &e2e,
            "c",
            &resolver,
            &config,
            &type_defs,
            &[],
            &[],
            CallIr {
                functions: &functions,
                type_defs: &type_defs,
            },
        )
        .expect("an undeclared bool leaf field must still render an e2e test function");

        assert!(
            rendered.contains("int32_t used_cache = sample_fetch_result_used_cache(result);"),
            "the local must be declared with the real ABI scalar type, not char*: {rendered}"
        );
        assert!(
            rendered.contains("assert(used_cache == 1 && \"equals assertion failed\");"),
            "a bool equals-assertion must compare the scalar directly: {rendered}"
        );
        assert!(
            !rendered.contains("strcmp(used_cache"),
            "a scalar accessor must never reach strcmp: {rendered}"
        );
    }

    /// An `Option<bool>` leaf field asserted `equals: true` must compare exactly, never widen
    /// into the "0 means unset" numeric-optional shape `render_assertion` uses for genuinely
    /// numeric optional fields (`assert((field == 0 || field == 1))`, which is vacuously true
    /// for ANY 0/1 boolean and can never catch a real mismatch). Before the fix, `is_numeric`
    /// keyed off the literal string `"bool"` — which the derived `fields_c_types` entry never
    /// spells (it spells the real ABI type, `int32_t`) — so an optional bool field's `equals`
    /// silently degraded into this vacuous form.
    #[test]
    fn optional_bool_leaf_field_equals_does_not_widen_into_the_vacuous_numeric_optional_form() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::ir::{FunctionDef, PrimitiveType as Prim};
        use crate::e2e::codegen::call_ir::CallIr;
        use crate::e2e::config::E2eConfig;
        use crate::e2e::field_access::FieldResolver;
        use crate::e2e::fixture::{Assertion, Fixture};

        let fixture = Fixture {
            id: "fetch_used_cache".into(),
            description: "Fetch reports whether the cache was used".into(),
            assertions: vec![Assertion {
                assertion_type: "equals".into(),
                field: Some("used_cache".into()),
                value: Some(serde_json::json!(true)),
                ..Default::default()
            }],
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "fetch".into();
        // `render_test_file` rebuilds its own per-call `FieldResolver` from
        // `e2e_config.effective_fields_optional`, discarding whatever `optional_fields` the
        // `resolver` argument below was constructed with — optionality has to be declared here,
        // on the config, not on the resolver passed in. ~keep
        e2e.fields_optional.insert("used_cache".into());
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let type_defs = vec![TypeDef {
            name: "FetchResult".into(),
            fields: vec![FieldDef {
                name: "used_cache".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(Prim::Bool))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let functions = [FunctionDef {
            name: "fetch".into(),
            return_type: TypeRef::Named("FetchResult".into()),
            ..FunctionDef::default()
        }];
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );

        let rendered = super::super::render_test_file(
            "fetching",
            &[&fixture],
            "sample.h",
            "sample",
            "result",
            &e2e,
            "c",
            &resolver,
            &config,
            &type_defs,
            &[],
            &[],
            CallIr {
                functions: &functions,
                type_defs: &type_defs,
            },
        )
        .expect("an undeclared optional bool leaf field must still render an e2e test function");

        assert!(
            rendered.contains("assert(used_cache == 1 && \"equals assertion failed\");"),
            "must compare exactly, not widen to accept 0 as a stand-in for unset: {rendered}"
        );
        assert!(
            !rendered.contains("used_cache == 0 || used_cache == 1"),
            "must not degrade into the vacuous numeric-optional form, which passes for either \
             boolean value: {rendered}"
        );
    }
}
