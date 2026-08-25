//! Emits the **inbound** plugin trait bridge — Swift implements a Rust trait, Rust calls back.
//!
//! Whereas [`trait_bridge`](super::trait_bridge) generates **outbound** glue (Swift caller →
//! Rust trait object), this module generates the inverse: a Swift class conforms to a
//! protocol, Rust holds a handle, and Rust calls each method on the Swift instance via
//! `extern "Swift"` declarations.
//!
//! This facade preserves the historical `gen_rust_crate::plugin_inbound` module path
//! while keeping inbound generation split by concern.

mod inbound_externs;
mod method_impls;
mod options_fields;
mod wrappers;

use crate::backends::swift::gen_rust_crate::type_bridge::{needs_json_bridge, swift_bridge_rust_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::TypeRef;

pub(crate) use inbound_externs::{emit_extern_block_for_inbound, emit_extern_block_for_inbound_registration};
pub(crate) use options_fields::{
    emit_options_field_factory, emit_options_field_from_impls, emit_options_field_options_helper,
};
pub(crate) use wrappers::{emit_inbound_wrapper, emit_plugin_error_helper};

/// ~keep
/// # The `Named` transport rule for the Swift trait bridge (alef-tasks #308, #309)
///
/// `Named` has no native swift-bridge representation, so it always crosses the inbound
/// (`extern "Swift"`) boundary as JSON. The question every site below answers identically is
/// *what gets JSON-encoded* when `Named` sits inside a container:
///
/// - `Vec<Named>` transports **PER-ELEMENT**: `Vec<String>`, one independently JSON-encoded
///   string per element. Both `extern "Rust"` and `extern "Swift"` blocks support a native
///   `Vec<String>` (swift-bridge's `Vectorizable` conformance for `String`/`RustString`), so
///   there is already a working element-wise path to reuse — see
///   `gen_bindings::plugin_marshal::vec_element_crosses_as_string` for the Swift-side half of
///   this same contract, and `method_impls::is_vec_of_named` for the Rust-side impl-body
///   conversion this rule requires in addition to the extern-block type below.
/// - `Map<_, _>` transports as **ONE JSON BLOB** (`String`), regardless of the value type.
///   swift-bridge has no `Map`/`HashMap` bridging at all: `needs_json_bridge` already treats
///   every `Map(_, _)` as JSON-string, Named-valued or not, so a Map's value being `Named`
///   changes nothing -- the container was already opaque before `Named` entered the picture.
///   Unlike `Vec`, there is no per-entry path to reuse (swift-bridge cannot even declare
///   `HashMap<K, V>` in an extern block), so unifying with the `Vec` rule is not possible; the
///   asymmetry is real; it comes from what swift-bridge already supports natively for each
///   container shape, not from an arbitrary per-call-site choice.
/// - A bare `Named` (or `Optional<Named>`) transports as ONE JSON BLOB (`String`, with `null`
///   for `None`).
///
/// Every site that decides how a `Named`-containing shape crosses this boundary must agree
/// with this rule: the extern-block type (`inbound_bridge_type` below), the impl-body
/// conversion (`needs_inbound_json_bridge` below plus `method_impls::is_vec_of_named`), the
/// outbound protocol/adapter declaration (`trait_bridge::swift_type_name`), and the outbound
/// FFI shim (`plugin_marshal::swift_shim_return_marshal` /
/// `plugin_marshal::vec_element_crosses_as_string`). A prior divergence between the `Vec` arm
/// (which already special-cased `Named` leaves) and the `Map` arm (which recursed into
/// `inbound_bridge_type(k)`/`inbound_bridge_type(v)` instead of asking this same question)
/// produced two independent bugs: alef-tasks #308 (`Vec<Named>` return/param conversions used
/// the wrong branch because `needs_inbound_json_bridge` did not know a `Vec<Named>` needs
/// per-element handling) and #309 (the `Map` arm declared a typed `HashMap<K, V>` extern block
/// that swift-bridge cannot parse, while the impl body -- correctly, per this rule -- treated
/// the whole value as one JSON blob).
///
/// Inbound-specific type bridging.
///
/// All `Named` types are JSON-bridged at the inbound boundary because the Swift side of an
/// `extern "Swift"` shim cannot produce the opaque Rust newtype the way `extern "Rust"`
/// callers do; it has to send a JSON payload that Rust deserialises into the source type.
/// Primitive scalars, `String`, `Vec<u8>`, and `Vec<leaf>` pass through as-is. `Map<_, _>` is
/// always one JSON blob (`String`), never a typed `HashMap`, because swift-bridge cannot
/// declare `HashMap<K, V>` in an extern block at all -- see the rule above.
pub(super) fn inbound_bridge_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_bridge_type(inner)),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => "Vec<u8>".to_string(),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_bridge_type(inner)),
        TypeRef::Map(_, _) => "String".to_string(),
        _ if needs_inbound_json_bridge(ty) => "String".to_string(),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Like [`needs_json_bridge`] but additionally treats every bare `Named` type as JSON-bridged
/// for inbound transport. `Vec<Named>` is deliberately NOT covered here -- it is the one
/// per-element exception the rule above carves out, and `method_impls::is_vec_of_named` is the
/// matching predicate its callers must check first. `Vec<other-leaf>` stays a typed Vec (e.g.
/// `Vec<String>`, `Vec<u8>`) when the inner type is a primitive/leaf.
pub(super) fn needs_inbound_json_bridge(ty: &TypeRef) -> bool {
    if needs_json_bridge(ty) {
        return true;
    }
    matches!(ty, TypeRef::Named(_))
}

/// Returns `true` when `ty` is `Vec<Named>` -- the one shape that crosses the inbound boundary
/// per-element (`Vec<String>`, each element independently JSON-encoded) rather than as one
/// blob. See the canonical rule above `inbound_bridge_type` for why `Vec` gets this exception
/// and `Map` does not.
pub(super) fn is_vec_of_named(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)))
}

/// Returns true when the trait bridge config declares a Plugin super-trait.
pub(super) fn has_plugin_super(bridge_config: &TraitBridgeConfig) -> bool {
    bridge_config
        .super_trait
        .as_deref()
        .map(|s| s == "Plugin" || s.ends_with("::Plugin"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbound_bridge_type_optional_vec_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
            "MyCustomType".to_string(),
        )))));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Option<Vec<String>>",
            "Optional<Vec<Named>> should become Option<Vec<String>> for JSON bridging"
        );
    }

    #[test]
    fn test_inbound_bridge_type_optional_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("MyStruct".to_string())));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "String",
            "Optional<Named> should become String for JSON bridging"
        );
    }

    #[test]
    fn test_inbound_bridge_type_vec_string_in_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Option<Vec<String>>",
            "Optional<Vec<String>> should pass through unchanged"
        );
    }

    /// alef-tasks #309: the extern block cannot declare `HashMap<K, V>` at all -- swift-bridge
    /// has no Map bridging -- so the whole map crosses as one JSON blob, matching what
    /// `needs_inbound_json_bridge` already decides for every `Map(_, _)`.
    #[test]
    fn test_inbound_bridge_type_map_named_value_is_one_blob() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("SinkStats".to_string())));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "String",
            "Map<_, Named> must become one JSON String blob, not a typed HashMap"
        );
    }

    /// The one-blob rule for `Map` does not depend on the value being `Named` -- swift-bridge
    /// cannot bridge `HashMap<K, V>` for any `K`/`V`, so every Map is a blob.
    #[test]
    fn test_inbound_bridge_type_map_primitive_value_is_also_one_blob() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)),
        );

        let result = inbound_bridge_type(&ty);
        assert_eq!(result, "String", "Map<_, primitive> must also become one JSON String blob");
    }

    #[test]
    fn test_is_vec_of_named_true_for_vec_named() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string())));
        assert!(is_vec_of_named(&ty));
    }

    #[test]
    fn test_is_vec_of_named_false_for_vec_string() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        assert!(!is_vec_of_named(&ty));
    }

    #[test]
    fn test_is_vec_of_named_false_for_bare_named() {
        let ty = TypeRef::Named("SinkStats".to_string());
        assert!(!is_vec_of_named(&ty));
    }

    #[test]
    fn test_inbound_bridge_type_vec_u8() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Bytes));

        let result = inbound_bridge_type(&ty);
        assert_eq!(result, "Vec<u8>", "Vec<u8> (Bytes) should remain Vec<u8>");
    }

    #[test]
    fn test_inbound_bridge_type_vec_named() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string())));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Vec<String>",
            "Vec<Named> should become Vec<String> for JSON bridging"
        );
    }
}
