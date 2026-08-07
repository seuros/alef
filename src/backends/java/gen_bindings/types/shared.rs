use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::TypeRef;

/// Resolve a TypeRef to its Java type, replacing unknown/excluded Named types with `Object`.
///
/// When a field references a type that was excluded from code generation (e.g. `#[alef(skip)]`),
/// we fall back to `TypeRef::Json`, which [`JavaMapper::json`](crate::backends::java::type_map::JavaMapper::json)
/// maps to `Object`, to preserve the object structure without requiring a Java type definition.
/// Jackson deserializes untyped `Object` fields to `LinkedHashMap`/`ArrayList`/boxed-primitive
/// values at runtime, so this remains Jackson-safe without a `JsonNode` dependency. Emitting
/// `JsonNode` instead would be a breaking change for existing Java consumers of generated DTOs,
/// so it is intentionally not done here.
pub(super) fn resolve_field_type(ty: &TypeRef, visible_types: &std::collections::HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if !visible_types.contains(name.as_str()) => TypeRef::Json,
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(resolve_field_type(inner, visible_types))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(resolve_field_type(inner, visible_types))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(resolve_field_type(k, visible_types)),
            Box::new(resolve_field_type(v, visible_types)),
        ),
        _ => ty.clone(),
    }
}

pub(super) fn is_options_field_bridge(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> bool {
    trait_bridges.iter().any(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
    })
}

pub(super) fn options_field_bridge_trait_name(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> Option<String> {
    trait_bridges.iter().find_map(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        if bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
        {
            Some(bridge.trait_name.clone())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::backends::java::type_map::java_type;

    #[test]
    fn resolve_field_type_replaces_excluded_named_with_json() {
        let visible: HashSet<&str> = HashSet::new();
        let resolved = resolve_field_type(&TypeRef::Named("Excluded".to_string()), &visible);
        assert_eq!(resolved, TypeRef::Json);
    }

    #[test]
    fn resolve_field_type_keeps_visible_named_types() {
        let visible: HashSet<&str> = HashSet::from(["Visible"]);
        let resolved = resolve_field_type(&TypeRef::Named("Visible".to_string()), &visible);
        assert_eq!(resolved, TypeRef::Named("Visible".to_string()));
    }

    // Regression guard for the doc comment above: excluded/unknown Named types must keep
    // rendering as `Object`, not `JsonNode` — switching to `JsonNode` would be a breaking
    // change for existing generated-DTO consumers.
    #[test]
    fn excluded_named_type_renders_as_object_not_json_node() {
        let visible: HashSet<&str> = HashSet::new();
        let resolved = resolve_field_type(&TypeRef::Named("Excluded".to_string()), &visible);
        assert_eq!(java_type(&resolved), "Object");
    }

    #[test]
    fn resolve_field_type_recurses_through_optional_vec_and_map() {
        let visible: HashSet<&str> = HashSet::new();
        let opt = TypeRef::Optional(Box::new(TypeRef::Named("Excluded".to_string())));
        assert_eq!(
            resolve_field_type(&opt, &visible),
            TypeRef::Optional(Box::new(TypeRef::Json))
        );

        let vec_ty = TypeRef::Vec(Box::new(TypeRef::Named("Excluded".to_string())));
        assert_eq!(
            resolve_field_type(&vec_ty, &visible),
            TypeRef::Vec(Box::new(TypeRef::Json))
        );

        let map_ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("Excluded".to_string())),
        );
        assert_eq!(
            resolve_field_type(&map_ty, &visible),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json))
        );
    }
}
