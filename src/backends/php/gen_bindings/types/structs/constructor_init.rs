//! Field initialisers for the `#[php(constructor)] pub fn new(...)` a PHP-mirrored struct gets.
//!
//! Split out of `structs.rs` so the rule deciding what a field the constructor *cannot accept as
//! a parameter* is initialised with lives in one place, next to the tests that pin it.

use ahash::AHashSet;

use super::php_field_can_be_constructor_param;
use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};

/// Local the generated constructor binds the core type's `Default` to, so a constructor that
/// omits several fields reads them all off one delegating call instead of one call per field.
/// Prefixed to stay out of the way of any real field or parameter name. ~keep
pub(crate) const CORE_DEFAULTS_BINDING: &str = "__alef_core_defaults";

/// The `Self { .. }` initialiser list for the named constructor, plus the statement it needs
/// when at least one omitted field is recovered from the core type's `Default`.
///
/// `Debug` is required, not decorative: the refusal tests assert with `expect_err`, whose Ok arm
/// formats the success value into the panic message. Both fields are plain generated-source
/// text, so there is nothing sensitive or large to leak into a failure report. ~keep
#[derive(Debug)]
pub(crate) struct ConstructorInit {
    /// Emitted immediately before `Self { .. }`. Empty when no field needed the recovery, so a
    /// constructor that does not use the local never binds it (and never trips `unused_variables`).
    pub(crate) prelude: String,
    /// Comma-joined `a, b: b_php, c: __alef_core_defaults.c`.
    pub(crate) field_inits: String,
}

/// Where a constructor-omitted field's value comes from.
///
/// Every variant is a value alef can defend. `Default::default()` — the unconditional answer this
/// replaced — is defensible only under [`OmittedInit::TypeZero`], where the IR states the default
/// *is* the field type's own zero. Everywhere else it invents a value the source crate never
/// wrote, and the invention is not cosmetic: for an allow-list field the invented value is an
/// empty allow-list, which is the fail-*open* direction (nothing is on the list, so nothing is
/// checked against it). A deny-list fabricated empty fails open too, by denying nothing. Neither
/// is recoverable at runtime, because the caller has no way to see that a value was invented.
/// Anything this enum cannot account for fails generation instead. ~keep
#[derive(Debug, PartialEq, Eq)]
enum OmittedInit {
    /// `DefaultValue::Empty` / `DefaultValue::None`: the IR asserts the default is exactly this
    /// type's zero, so the target-language zero is exact rather than a guess.
    TypeZero,
    /// Read back off `<Self as Default>::default()`, which for a type with a core `Default` is
    /// the delegating impl (see [`classify_omitted_field`]).
    FromCoreDefault,
    /// An `Option`-typed field on a type with no `Default` at all: `None`.
    ///
    /// This is the one omitted shape where the target-language zero is not an invention. `None`
    /// does not claim a value — it encodes *absent*, which is exactly true of a field the caller
    /// was given no way to pass. The dangerous case is the opposite one: an empty `Vec` claims
    /// "the list is empty", a statement about content that nothing in the source crate made.
    /// And no core `Default` can contradict `None` here, because reaching this variant requires
    /// the owning type to have no `Default` for a `Some(..)` to have been written in. ~keep
    Absent,
}

/// How a diagnostic names the type that owns an unrenderable field: the full Rust path when the
/// IR carries one, so the reader is sent to the definition rather than to a bare short name.
fn owning_type_path(typ: &TypeDef) -> &str {
    if typ.rust_path.is_empty() {
        &typ.name
    } else {
        &typ.rust_path
    }
}

/// Decide where a constructor-omitted field's value comes from, or fail generation.
///
/// `<Self as Default>::default()` is safe to read here because PHP emits a *delegating* `Default`
/// impl for every serde-mirrored type whose core type has one (`gen_php_struct` sets
/// `emit_delegating_default_impl` from `typ.has_default`, and PHP passes no
/// `emit_delegating_default_for_types` allow-list, so the impl is always the delegating one).
/// That impl is `<core::T as Default>::default().into()`, so reading a field off it needs no
/// per-field core-to-binding conversion — the whole struct has already been converted.
///
/// `DefaultValue::Unresolved` normally never reaches here: `unreadable_field_default_diagnostics`
/// (`cli::pipeline::generate::validation`) already fails the whole run for it. It does reach here
/// for a crate that suppresses that code, and the delegating read is the right answer then too —
/// alef could not *spell* the value, but the compiled `Default` impl still produces it. ~keep
fn classify_omitted_field(typ: &TypeDef, field: &FieldDef) -> anyhow::Result<OmittedInit> {
    if matches!(field.typed_default, Some(DefaultValue::Empty | DefaultValue::None)) {
        return Ok(OmittedInit::TypeZero);
    }
    if typ.has_default {
        return Ok(OmittedInit::FromCoreDefault);
    }
    if matches!(field.ty, TypeRef::Optional(_)) {
        return Ok(OmittedInit::Absent);
    }
    anyhow::bail!(
        "php backend: cannot initialise `{type_path}.{field_name}` in the generated \
         `#[php(constructor)] new(...)`.\n\
         The field's type is not representable as a PHP constructor parameter, so the constructor \
         omits it; `{type_name}` has no `Default` impl for alef to read the field's real value \
         back from; and the field is not an `Option`, whose absence `None` would honestly encode. \
         Alef will not fall back to `Default::default()`: that invents a value the source crate \
         never specifies — an empty allow-list, an empty deny-list, a zero limit — and for a \
         security control the invented empty value is the fail-open direction.\n\
         Fix one of:\n  \
         - add `#[derive(Default)]` or `impl Default for {type_name}` so alef delegates to it;\n  \
         - mark the field `#[alef(skip)]` if PHP callers must never set it;\n  \
         - give the field a PHP-representable type so it becomes a real constructor parameter.",
        type_path = owning_type_path(typ),
        type_name = typ.name,
        field_name = field.name,
    )
}

/// The initialiser for a field the constructor *does* accept as a parameter, keyed off the same
/// param-name convention `gen_php_function_params` establishes for the parameter list.
///
/// There is deliberately no `Vec<Named>` -> `{param}_core` arm here, unlike the sibling
/// constructor in `structs.rs` that takes every field as a parameter. That arm needs the inner
/// name to be neither opaque nor an enum, while reaching this function at all needs
/// `php_field_can_be_constructor_param`, which for `Vec<Named>` requires the inner name to be
/// opaque or an enum — the two conditions are mutually exclusive. ~keep
fn representable_field_init(field: &FieldDef, php_param_name: &str) -> String {
    let is_bytes = matches!(&field.ty, TypeRef::Bytes)
        || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes));
    if is_bytes {
        if field.optional {
            return format!("{}: {php_param_name}.map(|b| b.0)", field.name);
        }
        return format!("{}: {php_param_name}.0", field.name);
    }
    if field.name == php_param_name {
        field.name.clone()
    } else {
        format!("{}: {php_param_name}", field.name)
    }
}

/// Build the `Self { .. }` initialiser list for the named constructor.
///
/// Fails when any omitted field's value is unknowable — see [`classify_omitted_field`].
pub(crate) fn gen_constructor_field_inits(
    typ: &TypeDef,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> anyhow::Result<ConstructorInit> {
    let mut field_inits: Vec<String> = Vec::new();
    let mut needs_core_defaults = false;

    for field in typ.fields.iter().filter(|f| !f.binding_excluded) {
        // A `#[cfg]`-gated field is kept in the binding struct with `#[serde(skip)]` and has no
        // parameter under either cfg state, so it is not part of the omitted-default question
        // this module answers. ~keep
        if field.cfg.is_some() {
            field_inits.push(format!("{}: Default::default()", field.name));
            continue;
        }
        if php_field_can_be_constructor_param(&field.ty, enum_names, opaque_types) {
            let php_param_name = crate::codegen::naming::to_php_name(&field.name);
            field_inits.push(representable_field_init(field, &php_param_name));
            continue;
        }
        match classify_omitted_field(typ, field)? {
            OmittedInit::TypeZero => field_inits.push(format!("{}: Default::default()", field.name)),
            OmittedInit::FromCoreDefault => {
                needs_core_defaults = true;
                field_inits.push(format!("{}: {CORE_DEFAULTS_BINDING}.{}", field.name, field.name));
            }
            OmittedInit::Absent => field_inits.push(format!("{}: None", field.name)),
        }
    }

    let prelude = if needs_core_defaults {
        crate::backends::php::template_env::render(
            "php_core_defaults_let_binding.jinja",
            minijinja::context! { binding => CORE_DEFAULTS_BINDING },
        )
    } else {
        String::new()
    };

    Ok(ConstructorInit {
        prelude,
        field_inits: field_inits.join(", "),
    })
}

#[cfg(test)]
#[path = "constructor_init/tests.rs"]
mod tests;
