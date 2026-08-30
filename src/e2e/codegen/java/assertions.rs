//! Java assertion rendering helpers.

use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToLowerCamelCase;

use super::assertion_result_shape::{
    try_bare_option_result_assertion, try_bytes_result_assertion, try_not_error_result_assertion,
};
use super::assertion_streaming_fields::try_streaming_virtual_field_assertion;
use super::assertion_synthetic_fields::try_synthetic_field_assertion;
use super::assertion_wildcard::render_wildcard_assertion;
use super::values::json_to_java;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_bytes: bool,
    result_is_option: bool,
    is_streaming: bool,
    streaming_item_type: Option<&str>,
    enum_fields: &std::collections::HashSet<String>,
    assert_enum_types: &std::collections::HashMap<String, String>,
    returns_void: bool,
    fractional_fields: &std::collections::HashSet<String>,
    not_error_may_assert_presence: bool,
) {
    if try_bare_option_result_assertion(out, assertion, result_var, result_is_option) {
        return;
    }

    if try_bytes_result_assertion(out, assertion, result_var, result_is_bytes) {
        return;
    }

    if try_not_error_result_assertion(
        out,
        assertion,
        result_var,
        returns_void,
        is_streaming,
        not_error_may_assert_presence,
    ) {
        return;
    }

    if try_synthetic_field_assertion(out, assertion, result_var, field_resolver, result_is_simple) {
        return;
    }

    if try_streaming_virtual_field_assertion(out, assertion, is_streaming, streaming_item_type) {
        return;
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        out.push_str(&crate::e2e::template_env::render(
            "java/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "skipped",
                field_name => f,
            },
        ));
        return;
    }

    // Bracket-wildcard traversal (`links[].linkType`) means "every element". This must run
    // before `field_expr` is built below, since that path lowers the wildcard to index 0 and
    // would assert on a single element while reading as whole-array coverage. ~keep
    if let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_wildcard_assertion(out, assertion, result_var, field_resolver, f, &array_part, &elem_part);
        return;
    }

    // Determine if this field maps to a sealed-interface type declared in
    // `assert_enum_types`.  When `Some`, the value is the type name (e.g.
    // "FormatMetadata") and the corresponding `{TypeName}Display` helper will
    // be used to produce the display string for assertions.
    let sealed_display_type: Option<String> = assertion.field.as_deref().and_then(|f| {
        let resolved = field_resolver.resolve(f);
        assert_enum_types
            .get(f)
            .or_else(|| assert_enum_types.get(resolved))
            .cloned()
    });
    let is_sealed_display_field = sealed_display_type.is_some();

    // Determine if this field is an enum type (no `.contains()` on enums in Java).
    // Check both the raw fixture field path and the resolved (aliased) path so that
    // `fields_enum` entries can use either form (e.g., `"assets[].category"` or the
    // resolved `"assets[].asset_category"`). The hand-maintained `enum_fields` config is
    // checked first, but a field it never lists (e.g. a recursive struct's own enum field,
    // reached only through its parent's path — `data.kind` on a self-referential
    // `Option<Box<DataNode>>`) must still be rescued from the IR-derived classification, the
    // same way `field_resolver.is_enum` already backs every other backend's equivalent check
    // (csharp/kotlin/dart/gleam/swift/...). Without it, such a field silently falls through to
    // a plain `assertEquals(String, EnumType)` that can never pass, since a `String` is never
    // `.equals()` an enum constant. ~keep
    // NOTE: Sealed-interface types (those in assert_enum_types) are not Java enums
    // and do not have a .getValue() method — exclude them from enum field treatment.
    //
    // A third shape needs the same exclusion: an IR enum with data-carrying variants (e.g. a
    // `#[serde(untagged)]` union) is still an "enum" in the IR, but the Java binding backend
    // renders it as a wrapper class (`gen_java_tagged_union` / `gen_java_untagged_wrapper`),
    // neither of which declares `getValue()`. `java_enum_emits_get_value` answers from the
    // exact predicate the binding backend itself branches on
    // (`backends::java::gen_bindings::emits_get_value`), so this can never disagree with what
    // was actually emitted; it answers `None` when the IR doesn't resolve a concrete enum type
    // for the field (e.g. a `fields_enum`-only config entry), in which case the pre-existing
    // behaviour (assume `getValue()` is available) is kept. ~keep
    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let in_enum_fields = enum_fields.get(f).is_some()
            || enum_fields.get(resolved).is_some()
            || field_resolver.is_enum(f)
            || field_resolver.is_enum(resolved);
        let emits_get_value = field_resolver
            .java_enum_emits_get_value(f)
            .or_else(|| field_resolver.java_enum_emits_get_value(resolved))
            .unwrap_or(true);
        in_enum_fields && !is_sealed_display_field && emits_get_value
    });

    // Determine if this field is an array (List<T>) — needed to choose .toString() for
    // contains assertions, since List.contains(Object) uses equals() which won't match
    // strings against complex record types like StructureItem.
    let (field_is_array, field_is_object) = super::field_shape::classify(assertion.field.as_deref(), field_resolver);

    // A fixture's `equals` assertion carrying a literal JSON `null` against a field the IR
    // proves is a genuine (non-`Option`) collection -- e.g. `Vec<T>` with `#[serde(default,
    // skip_serializing_if = "Vec::is_empty")]` -- can never pass: the generated binding's
    // Jackson deserializer and its `Builder`'s own default both materialize an absent/omitted
    // collection as an empty `List`/`Map`, never `null` -- see
    // `backends::java::gen_bindings::types::builders::gen_builder_nested_class`'s
    // `field_is_optional_in_binding` branch, which only ever defaults to `null` when the IR's
    // `field.optional` is true. Both `is_collection_root` and `is_optional` here read that
    // same IR fact (via `with_ir_collection_map`/`with_ir_fields`), so this is the assertion
    // side asking the binding side's own question, not a second independent guess at the
    // answer. Recognize the case and assert the same emptiness the binding actually produces
    // instead of a null-equality check that can never pass. ~keep
    let is_doomed_null_equals_on_required_collection = assertion.assertion_type == "equals"
        && matches!(assertion.value, Some(serde_json::Value::Null))
        && assertion.field.as_deref().is_some_and(|f| {
            !f.is_empty()
                && field_resolver.is_valid_for_result(f)
                && field_resolver.is_collection_root(field_resolver.resolve(f))
                && !field_resolver.is_optional(field_resolver.resolve(f))
        });
    if is_doomed_null_equals_on_required_collection {
        let accessor = field_resolver.accessor(assertion.field.as_deref().unwrap_or_default(), "java", result_var);
        out.push_str(&format!(
            "        assertTrue({accessor}.isEmpty(), \"expected empty (binding never returns null for a non-optional collection)\");\n"
        ));
        return;
    }

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                let accessor = field_resolver.accessor(f, "java", result_var);
                let resolved = field_resolver.resolve(f);
                // Unwrap Optional fields with a type-appropriate fallback.
                // Map.get() returns nullable, not Optional, so skip .orElse() for map access.
                // NOTE: is_optional() means the field is in optional_fields, but that doesn't
                // guarantee it returns Optional<T> in Java — nested fields like metadata.twitterCard
                // return @Nullable String, not Optional<String>. We detect this by checking
                // if the field path contains a dot (nested access).
                // Fields in `fields_display_as_text` have an `Option<T>` inner type
                // that is not a plain `String` (e.g. `AssistantContent`). Their Java
                // binding exposes a `.text()` accessor returning `String`. Using
                // `Objects::toString` on these would produce the class-name representation,
                // not the textual content.
                let field_is_display_as_text = field_resolver.is_display_as_text(f);
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    // All nullable fields in the Java binding return @Nullable types, not Optional<T>.
                    // Wrap them in Optional.ofNullable() so e2e tests can use .orElse() fallbacks.
                    let optional_expr = format!("java.util.Optional.ofNullable({accessor})");
                    // Enum-typed optional fields need .map(v -> v.getValue()) to coerce to String
                    // before the orElse("") fallback can type-check (Optional<Enum>.orElse("") would
                    // be a type mismatch — Optional<String>.orElse("") is the only safe form).
                    if field_is_enum {
                        match assertion.assertion_type.as_str() {
                            // `is_true`/`is_false` on an Optional field mean "present"/"absent" --
                            // matching not_empty/is_empty, the raw Optional is returned so the
                            // template's presence switch (not a `.map(...).orElse(...)` string
                            // coercion, which produced a non-boolean `assertTrue` argument) decides.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
                            _ => {
                                // `field_is_enum` already excludes sealed-interface types
                                // (is_sealed_display_field), so any remaining enum type
                                // has .getValue() available.
                                format!("{optional_expr}.map(v -> v.getValue()).orElse(\"\")")
                            }
                        }
                    } else if field_is_display_as_text {
                        // Non-String content union (e.g. AssistantContent): call `.text()`
                        // to get the textual representation instead of `Objects::toString`
                        // which would return the class name.
                        match assertion.assertion_type.as_str() {
                            // `is_true`/`is_false` on an Optional field mean "present"/"absent" --
                            // matching not_empty/is_empty, the raw Optional is returned so the
                            // template's presence switch (not a `.map(...).orElse(...)` string
                            // coercion, which produced a non-boolean `assertTrue` argument) decides.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
                            _ => format!("{optional_expr}.map(v -> v.text()).orElse(\"\")"),
                        }
                    } else {
                        match assertion.assertion_type.as_str() {
                            // `not_empty`/`is_empty`/`is_true`/`is_false` on an Optional field all
                            // return the raw Optional so the template's presence switch decides --
                            // for is_true/is_false this replaces a `.map(...).orElse(...)` string
                            // coercion that produced a non-boolean `assertTrue` argument.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
                            // For size/count assertions on Optional<List<T>> fields, use List.of() fallback.
                            "count_min" | "count_equals" => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            // For numeric comparisons on Optional<Long/Integer> fields, coerce
                            // the boxed numeric type to `long` via Number::longValue so the same
                            // code path compiles for both `Optional<Integer>` (e.g. mapped from
                            // Rust `Option<u32>`) and `Optional<Long>` fields.  Using a bare
                            // `.orElse(0L)` would fail for `Optional<Integer>` because the
                            // fallback type would not match the element type.
                            //
                            // Fractional fields (`f32`/`f64`, e.g. `Optional<Double>
                            // qualityScore`) must NOT go through `Number::longValue()` — that
                            // truncates the boxed value to zero before the comparison ever
                            // runs, turning a `[0.0, 1.0]` range assertion into a tautology
                            // (every legal value truncates to `0L`, so both bounds always
                            // hold). Route these through `Number::doubleValue()` instead so
                            // the comparison actually observes the fractional value. ~keep
                            "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                                if field_resolver.is_array(resolved) {
                                    format!("{optional_expr}.orElse(java.util.List.of())")
                                } else if is_fractional_field(fractional_fields, resolved) {
                                    format!("{optional_expr}.map(Number::doubleValue).orElse(0.0)")
                                } else {
                                    format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                }
                            }
                            // For equals on Optional fields, determine fallback based on whether value is numeric.
                            // If the fixture value is a number, coerce via Number::longValue so the
                            // comparison compiles for both Optional<Integer> and Optional<Long>.
                            // Sealed-display fields are handled via the {TypeName}Display helper in
                            // string_expr — keep as Optional here so the helper receives the unwrapped value.
                            "equals" => {
                                if is_sealed_display_field {
                                    // Sealed-interface Optional: keep, will be handled by string_expr path
                                    optional_expr
                                } else if let Some(expected) = &assertion.value {
                                    if expected.is_number() {
                                        format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                    } else {
                                        // `.map(Objects::toString)` collapses Optional<T> to
                                        // Optional<String> before `.orElse("")`, so the result
                                        // is unambiguously a String even when T is `Object`
                                        // (which is the Java mapping for free-form JSON values
                                        // like `Option<serde_json::Value>` — javac otherwise
                                        // infers LUB(Object, String) = Object and breaks
                                        // String-only method calls like .contains()).
                                        format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                    }
                                } else {
                                    format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                }
                            }
                            _ if field_resolver.is_array(resolved) => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            _ => format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")"),
                        }
                    }
                } else {
                    accessor
                }
            }
            _ => result_var.to_string(),
        }
    };

    // For enum fields, string-based assertions need .getValue() to convert the enum to
    // its serde-serialized lowercase string value (e.g., AssetCategory.Image -> "image").
    // All alef-generated Java enums expose a getValue() method annotated with @JsonValue.
    // Optional enum fields are already coerced to String via `.map(v -> v.getValue()).orElse("")`
    // upstream in field_expr; in that case the value is already a String and we must not
    // call .getValue() again. Detect by looking for `.map(v -> v.getValue())` in the expr.
    // Sealed-interface types (is_sealed_display_field) use a pattern-match helper instead.
    let string_expr = if field_is_enum && !field_expr.contains(".map(v -> v.getValue())") {
        format!("{field_expr}.getValue()")
    } else if let Some(ref stype) = sealed_display_type {
        // Sealed-interface type: convert via a generated `{TypeName}Display.toDisplayString`
        // helper that pattern-matches over all variants from the IR.
        // For Optional<T>, unwrap with orElse(null) so the helper can handle null safely.
        let inner_expr = if field_expr.contains("Optional.ofNullable") {
            format!("{field_expr}.orElse(null)")
        } else {
            field_expr.clone()
        };
        format!("{stype}Display.toDisplayString({inner_expr})")
    } else {
        field_expr.clone()
    };

    // Pre-compute context for template
    let assertion_type = assertion.assertion_type.as_str();
    let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let is_numeric_val = assertion.value.as_ref().is_some_and(|v| v.is_number());

    // values_java is consumed by `contains`, `contains_all`, `contains_any`, and
    // `not_contains` loops. Fall back to wrapping the singular `value` so single-entry
    // fixtures still emit one assertion call per value instead of an empty loop.
    let values_java: Vec<String> = assertion
        .values
        .as_ref()
        .map(|values| values.iter().map(json_to_java).collect::<Vec<_>>())
        .or_else(|| assertion.value.as_ref().map(|v| vec![json_to_java(v)]))
        .unwrap_or_default();

    let contains_any_expr = if !values_java.is_empty() {
        values_java
            .iter()
            .map(|v| format!("{string_expr}.contains({v})"))
            .collect::<Vec<_>>()
            .join(" || ")
    } else {
        String::new()
    };

    let length_expr = if result_is_bytes {
        format!("{field_expr}.length")
    } else {
        format!("{field_expr}.length()")
    };

    let n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let call_expr = if let Some(method_name) = &assertion.method {
        build_java_method_call(result_var, method_name, assertion.args.as_ref(), class_name)
    } else {
        String::new()
    };

    let check = assertion.check.as_deref().unwrap_or("is_true");

    let java_check_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();

    let check_n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let is_bool_val = assertion.value.as_ref().is_some_and(|v| v.is_boolean());
    let bool_is_true = assertion.value.as_ref().is_some_and(|v| v.as_bool() == Some(true));

    let method_returns_collection = assertion
        .method
        .as_ref()
        .is_some_and(|m| matches!(m.as_str(), "find_nodes_by_type" | "findNodesByType"));

    let rendered = crate::e2e::template_env::render(
        "java/assertion.jinja",
        minijinja::context! {
            assertion_type,
            java_val,
            string_expr,
            field_expr,
            field_is_enum,
            field_is_array,
            field_is_object,
            is_string_val,
            is_numeric_val,
            values_java => values_java,
            contains_any_expr,
            length_expr,
            n,
            call_expr,
            check,
            java_check_val,
            check_n,
            is_bool_val,
            bool_is_true,
            method_returns_collection,
        },
    );
    out.push_str(&rendered);
}

/// Leaf segment of a (possibly dotted / bracketed) resolved field path, e.g.
/// `"results[0].quality_score"` -> `"quality_score"`.
fn leaf_field_name(path: &str) -> &str {
    let last_dot = path.rsplit('.').next().unwrap_or(path);
    last_dot.split('[').next().unwrap_or(last_dot)
}

/// True when `resolved`'s leaf field name is known (via [`fractional_scalar_fields`]) to
/// carry an `f32`/`f64` Rust type — directly or through `Option<T>`.
fn is_fractional_field(fractional_fields: &std::collections::HashSet<String>, resolved: &str) -> bool {
    fractional_fields.contains(leaf_field_name(resolved))
}

/// Field names (bare leaf, e.g. `"quality_score"`) whose Rust type — or `Option<T>` inner
/// type — is `f32`/`f64` on at least one IR type in `type_defs`.
///
/// Consulted before defaulting an `Optional` numeric-range coercion to
/// `Number::longValue()`: that truncates a fractional value to zero before the comparison
/// runs, turning e.g. a `[0.0, 1.0]` range assertion on a `Double` `qualityScore` into a
/// tautology (every legal value truncates to `0L`, so both bounds always hold). ~keep
pub(super) fn fractional_scalar_fields(type_defs: &[crate::core::ir::TypeDef]) -> std::collections::HashSet<String> {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let mut fractional = std::collections::HashSet::new();
    for type_def in type_defs {
        for field in &type_def.fields {
            let ty = match &field.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(
                ty,
                TypeRef::Primitive(PrimitiveType::F32) | TypeRef::Primitive(PrimitiveType::F64)
            ) {
                fractional.insert(field.name.clone());
            }
        }
    }
    fractional
}

/// Build a Java call expression for a `method_result` assertion on a sample_language Tree.
///
/// Maps method names to the appropriate Java static/instance method calls.
pub(super) fn build_java_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    class_name: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode().childCount()"),
        "root_node_type" => format!("{result_var}.rootNode().kind()"),
        "named_children_count" => format!("{result_var}.rootNode().namedChildCount()"),
        "has_error_nodes" => format!("{class_name}.treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{class_name}.treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("{class_name}.treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.findNodesByType({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let escaped_query = escape_java(query_source);
            format!("{class_name}.runQuery({result_var}, \"{language}\", \"{escaped_query}\", source)")
        }
        _ => {
            format!("{result_var}.{}()", method_name.to_lower_camel_case())
        }
    }
}
