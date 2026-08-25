use crate::core::ir::TypeRef;

/// Inverse of the sanitization in [`core_to_binding`] for `Vec<_>` fields:
/// given a sanitized binding-side type, emit the expression that rebuilds the
/// core-side value. The default fallback assumes the sanitized form is
/// `Vec<String>` of JSON-serialized elements (the `Vec<Json>` shape); the
/// `Vec<Vec<String>>` special case rebuilds `Vec<(String, String)>` from
/// 2-element inner Vecs (the `parse_homogeneous_tuple` shape — see
/// `core_to_binding::field_conversion_to_binding_cfg`).
pub(crate) fn sanitized_vec_field_to_core_expr(name: &str, ty: &TypeRef) -> String {
    if let TypeRef::Vec(outer_inner) = ty
        && let TypeRef::Vec(inner) = outer_inner.as_ref()
        && matches!(inner.as_ref(), TypeRef::String)
    {
        return format!(
            "{name}.iter().filter_map(|inner| {{ let mut it = inner.iter().cloned(); Some((it.next()?, it.next()?)) }}).collect()"
        );
    }
    // `.map(...).collect()`, not `.filter_map(...).collect()`: a parse failure on one element
    // must not silently shrink the Vec, which would shift every later element's index.
    // `unwrap_or_default()` keeps the count aligned with the source, on the same `T: Default`
    // assumption the sibling scalar sanitized-field fallbacks already rely on. ~keep
    format!("{name}.iter().map(|s| serde_json::from_str(s).unwrap_or_default()).collect()")
}

/// Binding→core inverse for a sanitized `Map<String, String>` field: given an access
/// expression for the binding-side `HashMap<String, String>`, emit the expression that
/// rebuilds the core-side map. Mirrors the plain (non-sanitized) `String`-keyed map
/// conversion in `binding_to_core::field_conversion_to_core` (`k.into(), v.into()`) — the
/// sanitized binding representation is the same `HashMap<String, String>` shape, so the same
/// inverse applies. Returns `None` for any other `ty`, so callers fall back to a form that
/// always compiles instead of guessing at an inverse this helper does not support.
pub(crate) fn sanitized_map_field_to_core_expr(access: &str, ty: &TypeRef) -> Option<String> {
    if let TypeRef::Map(k, v) = ty
        && matches!(k.as_ref(), TypeRef::String)
        && matches!(v.as_ref(), TypeRef::String)
    {
        return Some(format!(
            "{access}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()"
        ));
    }
    None
}

/// Core→binding inverse pair for [`sanitized_vec_field_to_core_expr`] /
/// [`sanitized_map_field_to_core_expr`]: given a bare (already-destructured) core-side value
/// expression, emit the expression that rebuilds the sanitized binding-side representation.
/// Supports the same two shapes as their binding→core counterparts — `Vec<Vec<String>>`
/// (rebuilt from `Vec<(String, String)>`) and `Map<String, String>` — and returns `None` for
/// every other sanitized shape, so callers can fall back to the pre-#218 `Default::default()`
/// / `None` output, which always compiles, instead of re-parsing a rendered conversion string.
pub(crate) fn sanitized_field_to_binding_expr(access: &str, ty: &TypeRef) -> Option<String> {
    if let TypeRef::Vec(outer_inner) = ty
        && let TypeRef::Vec(inner) = outer_inner.as_ref()
        && matches!(inner.as_ref(), TypeRef::String)
    {
        return Some(format!(
            "{access}.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()"
        ));
    }
    if let TypeRef::Map(k, v) = ty
        && matches!(k.as_ref(), TypeRef::String)
        && matches!(v.as_ref(), TypeRef::String)
    {
        return Some(format!(
            "{access}.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression coverage for task #405: `sanitized_vec_field_to_core_expr`'s default
    // (`Vec<Json>`) fallback used to `.filter_map(...).ok())`, which silently drops any element
    // that fails to deserialize instead of keeping the Vec's length aligned with the source.
    #[test]
    fn default_fallback_preserves_count_on_parse_failure() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Thing".to_string())));

        let result = sanitized_vec_field_to_core_expr("items", &ty);

        assert_eq!(
            result,
            "items.iter().map(|s| serde_json::from_str(s).unwrap_or_default()).collect()"
        );
        assert!(
            !result.contains("filter_map"),
            "must not drop elements that fail to deserialize, got: {result}"
        );
    }

    #[test]
    fn vec_vec_string_special_case_is_unaffected() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));

        let result = sanitized_vec_field_to_core_expr("pairs", &ty);

        assert_eq!(
            result,
            "pairs.iter().filter_map(|inner| { let mut it = inner.iter().cloned(); Some((it.next()?, it.next()?)) }).collect()"
        );
    }
}
