use super::*;

#[cfg(test)]
mod wildcard_tests {
    use super::{field_to_dart_accessor, render_assertion_dart};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn array_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &names, &names, &HashSet::new())
    }

    fn render_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion_dart(&mut out, &assertion, "result", false, resolver);
        out
    }

    /// Baseline: a single wildcard still quantifies over the whole list, so the refusal added
    /// for the nested case cannot have been implemented by refusing wildcards generally. ~keep
    #[test]
    fn single_wildcard_still_quantifies_over_every_element() {
        let out = render_contains(&array_resolver("links"), "links[].url", "example.test");
        assert!(out.contains("result.links.any((e) => e.url"), "got: {out}");
    }

    fn array_resolver_with_enum_field(field: &str) -> FieldResolver {
        array_resolver(field.split("[].").next().unwrap_or(field)).with_enum_fields([field.to_string()].into())
    }

    /// Regression: `structure[].kind` is a data-carrying Rust enum. flutter_rust_bridge/freezed
    /// stringifies it as `'StructureKind.function()'` (lowerCamelCase constructor call), which a
    /// fixture's PascalCase variant name (`'Function'`) never case-sensitively matches — and
    /// there is no `wireValue` extension for a data-carrying enum to fall back on (alef only
    /// emits `wireValue` for unit-only enums). `.runtimeType.toString()` instead yields alef's
    /// generated concrete subclass name in the variant's original casing
    /// (`'StructureKind_Function'`), which does contain the fixture's PascalCase value. ~keep
    #[test]
    fn contains_on_an_enum_typed_array_element_field_compares_the_runtime_type_name() {
        let out = render_contains(
            &array_resolver_with_enum_field("structure[].kind"),
            "structure[].kind",
            "Function",
        );
        assert!(
            out.contains("e.kind.runtimeType.toString().contains"),
            "an enum-typed element field must compare against the runtime type name, not \
             toString(), got:\n{out}"
        );
        assert!(!out.contains("e.kind.toString().contains"), "got: {out}");
    }

    /// Negative control: a plain (non-enum) element field, e.g. a `String`, must keep comparing
    /// its actual stringified content — `.runtimeType.toString()` on a `String` yields the type
    /// name `'String'`, not the value, so switching this field too would break every
    /// currently-passing `imports[].source`-style assertion.
    #[test]
    fn contains_on_a_non_enum_array_element_field_still_compares_the_stringified_value() {
        let out = render_contains(&array_resolver("imports"), "imports[].source", "example.test");
        assert!(
            out.contains("e.source.toString().contains"),
            "a non-enum element field must keep comparing its stringified content, got:\n{out}"
        );
        assert!(!out.contains("runtimeType"), "got: {out}");
    }

    /// The evidence that Dart's inner collapse was never even index-0-shaped: the element
    /// accessor renders the surviving bare bracket verbatim. `links![]` is not valid Dart, so
    /// a doubly-nested fixture would have produced a file that fails to analyze. ~keep
    #[test]
    fn the_element_accessor_renders_a_surviving_wildcard_as_invalid_dart() {
        assert_eq!(field_to_dart_accessor("links[].url"), "links![].url");
    }

    /// Pre-guard this test fails: the skip line is absent and the emitted `any((e) => ...)`
    /// closure names `e.links![].url`. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_contains(&array_resolver("pages"), "pages[].links[].url", "example.test");
        assert_eq!(
            out, "    // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }

    fn resolver_with_alias(alias_from: &str, alias_to: &str, result_field: &str) -> FieldResolver {
        let aliases: HashMap<String, String> = [(alias_from.to_string(), alias_to.to_string())].into_iter().collect();
        let result_fields: HashSet<String> = [result_field.to_string()].into_iter().collect();
        FieldResolver::new(
            &aliases,
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    /// Regression for the validation-before-resolution bug: `hreflang[].lang` is aliased to
    /// `metadata.hreflangs[].lang`, which renames the ARRAY HEAD segment (`hreflang` ->
    /// `metadata.hreflangs`), not just the sub-field. Validating the raw, unresolved head
    /// (`"hreflang"`) against `is_valid_for_result` — as the pre-fix code did — checks a name
    /// absent from `result_fields` and wrongly skips the assertion even though the renamed
    /// field exists. ~keep
    #[test]
    fn alias_renaming_the_array_head_segment_still_resolves() {
        let out = render_contains(
            &resolver_with_alias("hreflang[].lang", "metadata.hreflangs[].lang", "metadata"),
            "hreflang[].lang",
            "en",
        );
        assert!(!out.contains("skipped"), "got: {out}");
        assert!(
            out.contains("result.metadata.hreflangs.any((e) => e.lang"),
            "got: {out}"
        );
    }

    /// Control for the test above: a sub-field-only rename (the array head itself,
    /// `assets`, is untouched) must keep resolving too. This is the shape the pre-fix
    /// code's own comment cited as its example, so it passed whether or not the head-rename
    /// case above was fixed — pairing it here guards against a fix that only special-cases
    /// the head. ~keep
    #[test]
    fn alias_renaming_only_the_sub_field_still_resolves() {
        let out = render_contains(
            &resolver_with_alias("assets[].category", "assets[].asset_category", "assets"),
            "assets[].category",
            "books",
        );
        assert!(!out.contains("skipped"), "got: {out}");
        assert!(out.contains("result.assets.any((e) => e.assetCategory"), "got: {out}");
    }
}

#[cfg(test)]
mod is_true_optional_field_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn optional_resolver(field: &str) -> FieldResolver {
        let optional: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        render_assertion_dart(&mut out, assertion, "result", false, resolver);
        out
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        }
    }

    /// `Option<DataNode>` (FRB v2 maps this to `DataNode?`) presence: before the fix this
    /// rendered `expect(result.data, isTrue)`, which requires the value to literally be the
    /// bool `true` -- never the case for a present `DataNode?`.
    #[test]
    fn is_true_on_optional_struct_field_checks_presence() {
        let out = render(&optional_resolver("data"), &is_true_assertion("data"));
        assert_eq!(out, "    expect(result.data, isNotNull);\n");
    }

    #[test]
    fn is_false_on_optional_struct_field_checks_absence() {
        let out = render(
            &optional_resolver("data"),
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Assertion::default()
            },
        );
        assert_eq!(out, "    expect(result.data, isNull);\n");
    }

    #[test]
    fn is_true_on_non_optional_field_is_unchanged() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let out = render(&resolver, &is_true_assertion("active"));
        assert_eq!(out, "    expect(result.active, isTrue);\n");
    }
}

#[cfg(test)]
mod is_empty_branch_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(resolver: &FieldResolver, field: &str) -> String {
        let assertion = Assertion {
            assertion_type: "is_empty".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion_dart(&mut out, &assertion, "result", false, resolver);
        out
    }

    fn no_arrays_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    /// Regression: `not_empty` branches on `is_array` so a struct-shaped field never has
    /// `.isEmpty` called on it directly (structs have no such getter -- `NoSuchMethodError`
    /// at runtime). `is_empty` had no such branch and called `.isEmpty` unconditionally via
    /// `anyOf(isNull, isEmpty)`. `document` here is not in `array_fields`, so it takes the
    /// non-collection path.
    #[test]
    fn is_empty_on_struct_field_does_not_call_isempty_directly() {
        let out = render(&no_arrays_resolver(), "document");
        assert_eq!(out, "    expect((result.document?.toString() ?? ''), isEmpty);\n");
    }

    /// Control: a field the resolver classifies as an array keeps the original
    /// `anyOf(isNull, isEmpty)` form, since `List`/`Map`/`String` all have a real
    /// `.isEmpty` getter.
    #[test]
    fn is_empty_on_array_field_keeps_anyof_isnull_isempty() {
        let array_fields: HashSet<String> = ["items".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &array_fields,
            &HashSet::new(),
        );
        let out = render(&resolver, "items");
        assert_eq!(out, "    expect(result.items, anyOf(isNull, isEmpty));\n");
    }
}

#[cfg(test)]
mod enum_wire_value_assertion_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn enum_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(names)
    }

    fn optional_enum_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &names,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(names)
    }

    fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        render_assertion_dart(&mut out, assertion, "result", false, resolver);
        out
    }

    fn equals_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }
    }

    /// Regression: an enum `equals` assertion must compare the fixture's serde wire literal
    /// VERBATIM against the binding's `.wireValue` getter. The prior `_alefE2eText` helper
    /// reconstructed a wire value from `.toString()` via an unconditional camelCase ->
    /// snake_case heuristic, so it could never reproduce a wire value with no `rename_all`
    /// (e.g. `KeyValue`, which stays PascalCase on the wire) -- it always emitted `key_value`.
    #[test]
    fn equals_on_enum_field_asserts_wire_value_verbatim() {
        let out = render(&enum_resolver("kind"), &equals_assertion("kind", "KeyValue"));
        assert_eq!(out, "    expect(result.kind.wireValue, equals('KeyValue'));\n");
    }

    /// `Option<Enum>` maps to `Enum?` in FRB Dart -- `.wireValue` needs safe navigation.
    #[test]
    fn equals_on_optional_enum_field_uses_safe_navigation() {
        let out = render(&optional_enum_resolver("kind"), &equals_assertion("kind", "KeyValue"));
        assert_eq!(out, "    expect(result.kind?.wireValue, equals('KeyValue'));\n");
    }

    #[test]
    fn not_equals_on_enum_field_asserts_wire_value_verbatim() {
        let assertion = Assertion {
            assertion_type: "not_equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("Sequence".to_string())),
            ..Assertion::default()
        };
        let out = render(&enum_resolver("kind"), &assertion);
        assert_eq!(out, "    expect(result.kind.wireValue, isNot(equals('Sequence')));\n");
    }
}
