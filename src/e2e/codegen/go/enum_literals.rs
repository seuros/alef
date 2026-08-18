//! Constructing a Go expression whose type is the one the Go binding declares for an IR enum.
//!
//! The Go backend emits an IR enum as one of six declarations (see
//! [`crate::backends::go::go_enum_representation`]). Three of them — `type X string`,
//! `type X string` with a partial const block, and `type X json.RawMessage` — have an
//! underlying type an untyped string constant converts to, so a fixture value reaches them as
//! the conversion `alias.X(<value>)` and this module deliberately declines to touch them.
//!
//! The other three declare a `struct` or a sealed `interface`, and no conversion reaches
//! either: the value has to be *constructed*. Every identifier used to construct one — the
//! variant's struct field, its JSON key, the discriminator field, the adjacent-tagged
//! constructor, the concrete variant struct of a sealed interface — is asked of the binding
//! backend rather than re-derived here, because a name this module invented would name nothing
//! the binding declares and would fail to compile just as loudly as the conversion it
//! replaced. ~keep

use crate::backends::go::{
    GoEnumRepresentation, GoStructEnumVariantField, go_adjacent_tagged_constructor,
    go_data_enum_untagged_variant_matches, go_data_enum_variant_field, go_data_enum_variant_scalar_tuple_field,
    go_data_enum_variant_struct, go_enum_representation, go_struct_enum_tag_field, go_struct_enum_variant_fields,
};
use crate::codegen::naming::{go_type_name, wire_variant_value};
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use crate::e2e::escape::go_string_literal;

use super::json_values::json_to_go;
use super::setup::{GoFieldSite, GoValueContext, go_named_field_expression, go_struct_field_expression};

/// Build an expression of the Go type the binding declares for `enum_def`, or `Ok(None)` when
/// the fixture value identifies no variant of it and the caller must refuse.
///
/// `Err` is reserved for a value that *did* identify a variant whose payload then had no valid
/// expression — that diagnostic names the offending inner field and is strictly more useful
/// than the outer one, so it propagates instead of collapsing into "no variant". ~keep
pub(super) fn go_enum_value_expression(
    value: &serde_json::Value,
    enum_def: &EnumDef,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    match go_enum_representation(enum_def) {
        // These three already have a legal conversion, which the caller rendered before asking.
        // Constructing something else here would rewrite snippets that compile today. ~keep
        GoEnumRepresentation::UnitString
        | GoEnumRepresentation::NewtypeTupleString
        | GoEnumRepresentation::RawMessage => Ok(None),
        GoEnumRepresentation::AdjacentTaggedStruct => adjacent_tagged_expression(value, enum_def, context, site),
        GoEnumRepresentation::TupleTaggedStruct | GoEnumRepresentation::ExternallyTaggedStruct => {
            struct_union_expression(value, enum_def, context, site)
        }
        GoEnumRepresentation::DataInterface => data_interface_expression(value, enum_def, context, site),
    }
}

/// Build a composite literal for an enum the backend emits as `type X struct { .. }` with one
/// `omitempty` pointer field per variant.
fn struct_union_expression(
    value: &serde_json::Value,
    enum_def: &EnumDef,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    let variant_fields = go_struct_enum_variant_fields(enum_def);
    let Some((selected, payload)) = select_struct_union_variant(value, enum_def, &variant_fields, context) else {
        return Ok(None);
    };
    let TypeRef::Named(payload_type) = &selected.payload.ty else {
        return Ok(None);
    };
    let payload_expression = go_named_field_expression(payload, payload_type, context, site, true)?;
    let mut assignments = Vec::new();
    if let Some((tag_field, _)) = go_struct_enum_tag_field(enum_def) {
        // `tagged_union_marshal_json_header.jinja` switches on this field, so a literal that
        // sets the variant pointer and leaves the tag empty falls through to the footer's
        // tag-only fallback and serialises without the payload at all. ~keep
        let wire_value = variant_wire_value(enum_def, selected.variant);
        assignments.push(format!("{tag_field}: {}", go_string_literal(&wire_value)));
    }
    assignments.push(format!("{}: {payload_expression}", selected.field_name));
    Ok(Some(format!(
        "{}.{}{{{}}}",
        context.import_alias,
        go_type_name(&enum_def.name),
        assignments.join(", ")
    )))
}

/// Which variant of a struct-shaped enum a fixture value selects, and the JSON its payload is
/// built from.
///
/// The three struct representations answer this from three different places, exactly as the
/// emitted decoders do: the internally tagged form reads its discriminator and hands the whole
/// object to the payload, because serde writes the payload's fields alongside the tag; the
/// externally tagged form is a single-key object whose key names the variant and whose value is
/// the payload; and the untagged form has no discriminator at all, so it takes the first variant
/// whose payload type can represent the value — the static reading of
/// `untagged_union_marshalers.jinja`'s try-each-variant-in-declaration-order decode. ~keep
fn select_struct_union_variant<'a, 'v>(
    value: &'v serde_json::Value,
    enum_def: &EnumDef,
    variant_fields: &'a [GoStructEnumVariantField<'a>],
    context: GoValueContext<'_>,
) -> Option<(&'a GoStructEnumVariantField<'a>, &'v serde_json::Value)> {
    if let Some((_, tag_key)) = go_struct_enum_tag_field(enum_def) {
        let tag = value.get(tag_key)?.as_str()?;
        let selected = variant_fields
            .iter()
            .find(|candidate| variant_wire_value(enum_def, candidate.variant) == tag)?;
        return Some((selected, value));
    }
    if go_enum_representation(enum_def) == GoEnumRepresentation::ExternallyTaggedStruct {
        let object = value.as_object()?;
        return variant_fields
            .iter()
            .find_map(|candidate| Some((candidate, object.get(&candidate.json_key)?)));
    }
    variant_fields
        .iter()
        .find(|candidate| payload_type_can_represent(value, &candidate.payload.ty, context))
        .map(|candidate| (candidate, value))
}

/// Whether a JSON value has an expression of the Go type declared for `type_ref` — the question
/// an untagged union's generated `UnmarshalJSON` answers at run time by attempting each variant
/// in turn.
///
/// Answered conservatively: a name that resolves to neither a struct nor an enum proves nothing
/// about what its Go declaration accepts, so it never wins the selection and the caller refuses
/// rather than emitting a guess. ~keep
fn payload_type_can_represent(value: &serde_json::Value, type_ref: &TypeRef, context: GoValueContext<'_>) -> bool {
    let TypeRef::Named(name) = type_ref else {
        return false;
    };
    if context
        .type_defs
        .iter()
        .any(|definition| definition.name == *name && !definition.is_opaque)
    {
        return value.is_object();
    }
    let Some(enum_def) = context.enums.iter().find(|candidate| candidate.name == *name) else {
        return false;
    };
    match go_enum_representation(enum_def) {
        GoEnumRepresentation::UnitString | GoEnumRepresentation::NewtypeTupleString => value.is_string(),
        GoEnumRepresentation::RawMessage => !value.is_null(),
        _ => value.is_object(),
    }
}

/// Build a call to the constructor `adjacent_tagged_enum.jinja` declares for the tagged variant.
///
/// The constructor is preferred over a bare composite literal because it is the only spelling
/// that knows whether the emitted content field is a pointer (homogeneous payloads) or `any`
/// (heterogeneous ones); a literal would have to re-derive that and could get it wrong. ~keep
fn adjacent_tagged_expression(
    value: &serde_json::Value,
    enum_def: &EnumDef,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    let Some((_, tag_key)) = go_struct_enum_tag_field(enum_def) else {
        return Ok(None);
    };
    let Some(tag) = value.get(tag_key).and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    let Some(variant) = enum_def
        .variants
        .iter()
        .find(|candidate| variant_wire_value(enum_def, candidate) == tag)
    else {
        return Ok(None);
    };
    let constructor = format!(
        "{}.{}",
        context.import_alias,
        go_adjacent_tagged_constructor(enum_def, variant)
    );
    let Some(payload_field) = variant.fields.first() else {
        return Ok(Some(format!("{constructor}()")));
    };
    let Some(content_key) = enum_def.serde_content.as_deref() else {
        return Ok(None);
    };
    let Some(content) = value.get(content_key) else {
        return Ok(None);
    };
    let Some(payload) = unaddressed_payload_expression(content, &payload_field.ty, context, site)? else {
        return Ok(None);
    };
    Ok(Some(format!("{constructor}({payload})")))
}

/// A payload rendered as a value rather than an address: what an adjacent-tagged constructor
/// parameter (declared `go_type(&field.ty)`) and a sealed-interface variant's `Value` field
/// (declared the same way) both take. ~keep
fn unaddressed_payload_expression(
    content: &serde_json::Value,
    type_ref: &TypeRef,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    match type_ref {
        TypeRef::Named(name) => go_named_field_expression(content, name, context, site, false).map(Some),
        TypeRef::String | TypeRef::Char | TypeRef::Path => Ok(content.as_str().map(go_string_literal)),
        TypeRef::Primitive(_) => Ok((content.is_number() || content.is_boolean()).then(|| json_to_go(content))),
        _ => Ok(None),
    }
}

/// Build a literal of the concrete variant struct a sealed-interface enum declares.
///
/// An interface has no constructor and no conversion, so the concrete type is the only thing a
/// snippet can write; it satisfies the interface through the unexported marker method the
/// binding attaches to it. ~keep
fn data_interface_expression(
    value: &serde_json::Value,
    enum_def: &EnumDef,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    let Some(variant) = select_data_enum_variant(value, enum_def) else {
        return Ok(None);
    };
    let struct_name = format!(
        "{}.{}",
        context.import_alias,
        go_data_enum_variant_struct(enum_def, variant)
    );
    if let Some(scalar) = go_data_enum_variant_scalar_tuple_field(enum_def, variant) {
        let Some(literal) = unaddressed_payload_expression(value, &scalar.ty, context, site)? else {
            return Ok(None);
        };
        return Ok(Some(format!("{struct_name}{{Value: {literal}}}")));
    }
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let mut assignments: Vec<String> = Vec::new();
    for field in &variant.fields {
        let Some((field_name, json_key)) = go_data_enum_variant_field(enum_def, field) else {
            continue;
        };
        let Some(field_value) = object.get(&json_key).or_else(|| object.get(&field.name)) else {
            continue;
        };
        let field_pointer = format!("{}/{}", site.pointer, field.name);
        let field_site = GoFieldSite {
            owner_type: &enum_def.name,
            field_name: &field.name,
            pointer: &field_pointer,
        };
        // The variant struct declares `go_optional_type` for an optional field and `go_type`
        // otherwise. `needs_omitempty_pointer` is a struct-emitter rule and is not consulted
        // here, so pointer-ness follows `field.optional` alone. ~keep
        let Some(expression) = go_struct_field_expression(field, field_value, context, field_site, field.optional)?
        else {
            continue;
        };
        assignments.push(format!("{field_name}: {expression}"));
    }
    Ok(Some(format!("{struct_name}{{{}}}", assignments.join(", "))))
}

/// Which variant of a sealed-interface enum a fixture value selects.
///
/// Mirrors `gen_data_enum_type`'s own decoder: an internally tagged enum switches on the tag,
/// an untagged one takes the first variant whose declared JSON shape matches. serde's default
/// external tagging is deliberately absent — `data_enum_unmarshal_wire_header.jinja` reads a
/// discriminator that form does not carry, and the variant marshalers write no external tag
/// either, so there is no expression this can prove round-trips and refusing leaves the operator
/// the two honest options the diagnostic names. ~keep
fn select_data_enum_variant<'a>(value: &serde_json::Value, enum_def: &'a EnumDef) -> Option<&'a EnumVariant> {
    if let Some(tag_key) = enum_def.serde_tag.as_deref() {
        let tag = value.get(tag_key)?.as_str()?;
        return enum_def
            .variants
            .iter()
            .find(|variant| variant_wire_value(enum_def, variant) == tag);
    }
    if !enum_def.serde_untagged {
        return None;
    }
    enum_def
        .variants
        .iter()
        .find(|variant| go_data_enum_untagged_variant_matches(variant, value))
}

/// The wire string serde writes for a variant, which is the discriminator value fixture JSON
/// carries and the key an externally tagged object is keyed by.
fn variant_wire_value(enum_def: &EnumDef, variant: &EnumVariant) -> String {
    wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    )
}
