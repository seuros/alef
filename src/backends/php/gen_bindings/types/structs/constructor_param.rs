//! `php_field_can_be_constructor_param` — the WIDER predicate deciding which struct fields
//! become `#[php(constructor)]` parameters. Split out of `structs.rs` to keep that file under
//! the repo's 1,000-line file-size cap; this predicate's own doc comment (below) is long
//! enough on its own to justify a dedicated file, and it has no state or helpers that aren't
//! either parameters or calls back into `super`.

use super::is_php_prop_scalar_with_enums;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

/// Returns true if `ty` is representable as a promoted `#[php(constructor)]` parameter.
///
/// This is the WIDER predicate the real extension's constructor uses to decide which fields
/// become constructor params — a superset of [`is_php_prop_scalar`] (every prop-scalar field
/// can be a param, but some non-prop-scalar fields — e.g. `Vec<Named>` of any non-untagged-enum
/// element type (opaque, enum, or a plain `#[php_class]` struct), `Bytes`, bare `Json`, or a bare
/// nested `#[php_class]` struct — can be params too even though they are not `#[php(prop)]`
/// properties). `Vec<Named>` of a plain struct decodes through `ZendHashTable` element-by-element
/// (`gen_php_function_params` in `helpers/params.rs`, and the `php_vec_named_struct_let_binding.jinja`
/// let-binding this module already emits for it); a bare `Json` field decodes a JSON `String` param
/// via `serde_json::from_str`. Both are fallible — see `param_conversion_is_fallible` — so a
/// constructor accepting either returns `PhpResult<Self>` instead of the ordinarily-infallible
/// bare `Self`. `TypeRef::Optional` already recurses into this same arm, so `Option<Json>` needs
/// no separate handling.
///
/// A bare `Named` type that is neither an enum (mapped to PHP `string`) nor opaque (bridged via
/// `&mut Opaque` / a wither method) nor an untagged data enum (mapped to `serde_json::Value`,
/// which has no ext-php-rs `FromZval` impl) is itself a `#[php_class]`-registered mirror
/// struct — either an ordinary struct or a *tagged* data enum, both of which `PhpMapper::named`
/// (`type_map.rs`) maps to "their own flat PHP class" rather than to a scalar. ext-php-rs 0.15's
/// `impl<T: RegisteredClass> FromZval for &ZendClassObject<T>`
/// (`ext-php-rs-0.15.15/src/types/class_object.rs`) lets a constructor take such a class by
/// reference — the exact mechanism `gen_php_function_params` (`helpers/params.rs`) already uses
/// for every other Named-struct function/method parameter in this backend. Before this arm, such
/// a field was never a constructor param at all: it was silently defaulted or (once the
/// no-fabrication check landed) refused generation outright whenever the owning type had no
/// `Default` to read it back from. Accepting it here makes the field honestly settable instead.
///
/// `untagged_data_enum_names` must be the SAME set `PhpMapper::untagged_data_enum_names` carries
/// (every caller already has a `PhpMapper` or derives this set the identical way
/// `rust_bindings.rs`/`type_stubs.rs` do) — passing a narrower or stale set would let an
/// untagged-data-enum field slip through this arm and render `&serde_json::Value` as a
/// constructor parameter type, which does not compile. ~keep
pub fn php_field_can_be_constructor_param(
    ty: &TypeRef,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
    untagged_data_enum_names: &AHashSet<String>,
) -> bool {
    match ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            // An untagged data enum element has no `#[php_class]` mirror to decode a
            // `ZendHashTable` entry into (`PhpMapper::named` maps it to `serde_json::Value`,
            // which has no ext-php-rs `FromZval` impl) -- exclude it here for the SAME reason
            // the bare-`Named` arm below excludes it, or `gen_php_function_params` would render
            // `&ZendHashTable` for a Vec this backend cannot actually decode.
            TypeRef::Named(name) => !untagged_data_enum_names.contains(name.as_str()),
            TypeRef::Json => false,
            _ => true,
        },
        TypeRef::Bytes => true,
        // `serde_json::Value` has no ext-php-rs `FromZval` impl -- symmetric with how
        // `ty_is_or_wraps_json` already makes a Json field's GETTER return `Option<String>`
        // (serialized JSON) rather than `Value` (`gen_bindings/types/structs.rs`'s getter loop
        // below), the constructor takes a JSON `String` param and decodes it with
        // `serde_json::from_str`. That decode is fallible on malformed input, exactly like the
        // `Vec<Named>` per-element decode above -- see `param_conversion_is_fallible`, which this
        // arm must stay in sync with, or a constructor could type-check-mismatch a bare `Self`
        // return against the `?`-propagated decode error `representable_field_init` emits.
        TypeRef::Json => true,
        TypeRef::Optional(inner) => {
            php_field_can_be_constructor_param(inner, enum_names, opaque_types, untagged_data_enum_names)
        }
        TypeRef::Named(name)
            if !opaque_types.contains(name.as_str())
                && !enum_names.contains(name.as_str())
                && !untagged_data_enum_names.contains(name.as_str()) =>
        {
            true
        }
        _ => is_php_prop_scalar_with_enums(ty, enum_names),
    }
}
