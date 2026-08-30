//! SECURITY. `kotlin_field_default` is what `dto.rs` calls to render a `data class` constructor
//! parameter's default suffix. A field carrying `#[serde(default = "path")]` reaches the IR as
//! `DefaultValue::FunctionCall` — alef records the function's *name*, never its return value —
//! so Kotlin has nothing to spell and correctly emits no default, leaving the parameter required
//! and letting Rust's own serde default supply the value.
//!
//! That refusal was unreachable in practice. `extract::extractor::types` blanket-overwrote every
//! field's `typed_default` with `DefaultValue::Empty` whenever the container derived `Default`,
//! and the `Empty` arm below renders `emptyList()`/`emptyMap()`. A named allow-list or deny-list
//! default therefore arrived at Rust as an empty collection the caller never chose: fail-open for
//! a deny-list, and a total denial for an allow-list. These tests pin the deferral against the
//! IR shape the extractor now produces.
//!
//! `tests.rs` in this same directory is at this repo's 1,000-line file-size cap (grandfathered),
//! so this coverage lives in its own module instead of growing that file. ~keep

use crate::core::ir::{DefaultValue, PrimitiveType, TypeRef};
use std::collections::{HashMap, HashSet};

fn render(ty: &TypeRef, typed_default: Option<&DefaultValue>) -> String {
    super::types::kotlin_field_default(ty, false, typed_default, &HashMap::new(), &HashSet::new())
}

#[test]
fn a_named_serde_default_on_a_vec_defers_to_rust_instead_of_emitting_an_empty_list() {
    let ty = TypeRef::Vec(Box::new(TypeRef::String));
    let named = DefaultValue::FunctionCall("default_scheme_allowlist".to_string());

    let rendered = render(&ty, Some(&named));

    assert_eq!(
        rendered, "",
        "alef never evaluates default_scheme_allowlist(); ` = emptyList()` would ship an empty \
         allow-list in place of the real one, got `{rendered}`"
    );
}

#[test]
fn a_named_serde_default_on_a_map_defers_to_rust_instead_of_emitting_an_empty_map() {
    let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    let resolved = DefaultValue::PublicFunctionCall("sample_crate::Policy::header_overrides".to_string());

    let rendered = render(&ty, Some(&resolved));

    assert_eq!(
        rendered, "",
        "a resolved function-call default is still a value alef has not read, got `{rendered}`"
    );
}

#[test]
fn a_named_serde_default_on_a_scalar_defers_to_rust_instead_of_emitting_a_zero() {
    let ty = TypeRef::Primitive(PrimitiveType::U32);
    let named = DefaultValue::FunctionCall("default_redirect_limit".to_string());

    let rendered = render(&ty, Some(&named));

    assert_eq!(
        rendered, "",
        "`= 0` is a claim about a value alef does not have, got `{rendered}`"
    );
}

/// Discrimination control for all three tests above. `Empty` genuinely IS `Default::default()`,
/// so the empty collection and the scalar zero are exact for it and must still be emitted. Without
/// this, a change that stopped rendering collection defaults altogether would satisfy every
/// assertion above while stripping the defaults off every ordinary `#[derive(Default)]` field. ~keep
#[test]
fn an_empty_default_still_renders_the_kotlin_zero_for_each_shape() {
    assert_eq!(
        render(&TypeRef::Vec(Box::new(TypeRef::String)), Some(&DefaultValue::Empty)),
        " = emptyList()"
    );
    assert_eq!(
        render(
            &TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            Some(&DefaultValue::Empty)
        ),
        " = emptyMap()"
    );
    assert_eq!(
        render(&TypeRef::Primitive(PrimitiveType::U32), Some(&DefaultValue::Empty)),
        " = 0"
    );
}
