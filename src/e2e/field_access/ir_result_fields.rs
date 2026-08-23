//! Answers "is this field optional?" and "does the result declare this field at all?" against
//! the *exact type the call returns*, instead of by bare field name across the whole crate IR.
//!
//! `FieldResolver::ir_field_sets` has to answer both questions from flat name sets, because it
//! is handed nothing that identifies which type the call under generation actually returns. That
//! forces two compromises it documents honestly and this module removes:
//!
//! * optionality is decided by unanimity — a name counts as optional only when EVERY declaration
//!   of it in the crate is `Option<T>` — so one required twin on an unrelated struct silences the
//!   guard for the declaration that matters;
//! * reachability is decided by existence-anywhere, so a name declared on any type at all reads
//!   as a member of every result.
//!
//! Both are the safe default for a set that cannot tell types apart. Once the call's declared
//! return type is resolved (`codegen::call_ir::resolve_declared_result_type`), neither
//! compromise is needed: [`build_ir_result_field_map`] keys its answers by `(owner_type,
//! field_name)` and the two walkers below advance a type cursor from the root through the IR's
//! own struct graph before answering at the leaf — the same shape `ir_enum` and `ir_collection`
//! already use, and for the same reason.
//!
//! ~keep The optional set is *binding* optionality, not core-crate optionality. A NAPI binding
//! widens every field of a `Default`-implementing type to `Option<T>`, so a field declared
//! `metadata: PageMetadata` in Rust still reaches TypeScript as `readonly metadata?:
//! PageMetadata`; a snippet that renders `result.metadata.title` against it is a `TS18048`.
//! `OptionalityRule` carries which of those rules the target binding applies, and the NAPI arm
//! calls the binding backend's own predicate so the two can never drift.

use std::collections::{HashMap, HashSet};

use crate::codegen::shared::binding_fields;
use crate::core::ir::{FieldDef, TypeDef};
use crate::e2e::codegen::call_ir::named_type;

use super::parse::{parse_path, segment_name};
use super::types::IrResultFieldMap;

/// Which "this field may be absent" rule the target language's binding applies.
///
/// A per-language choice rather than one shared answer because the bindings genuinely disagree,
/// and picking either one for everybody breaks the other half: guarding a wasm-bindgen getter
/// that always returns a value adds dead `?.` noise, while not guarding a NAPI `has_default`
/// field is a compile error in the generated snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionalityRule {
    /// Only the field's own declared type decides. Every binding except NAPI.
    DeclaredType,
    /// The NAPI rule, per `backends::napi::gen_bindings::types::napi_field_is_optional`: the
    /// field's own type, OR its owner implementing `Default`.
    Napi,
}

impl OptionalityRule {
    /// The rule the binding generated for `language` applies to its struct fields.
    pub(crate) fn for_language(language: &str) -> Self {
        match language {
            "node" | "typescript" => Self::Napi,
            _ => Self::DeclaredType,
        }
    }

    fn applies_to(self, field: &FieldDef, owner: &TypeDef) -> bool {
        match self {
            Self::DeclaredType => field.optional,
            Self::Napi => crate::backends::napi::napi_field_is_optional(field, owner),
        }
    }
}

/// Build the per-owner-type field facts [`IrResultFieldMap`] answers from.
///
/// `declared_fields` records only fields the binding actually attaches an accessor to
/// ([`binding_fields`], the same predicate every backend emits from), so a `#[serde(skip)]`
/// field is absent here exactly as it is absent from the generated class — a derived accessor
/// for it would not compile.
pub(super) fn build_ir_result_field_map(type_defs: &[TypeDef], rule: OptionalityRule) -> IrResultFieldMap {
    let struct_names: HashSet<&str> = type_defs.iter().map(|type_def| type_def.name.as_str()).collect();

    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut optional_fields: HashMap<String, HashSet<String>> = HashMap::new();
    let mut declared_fields: HashMap<String, HashSet<String>> = HashMap::new();

    for type_def in type_defs {
        for field in binding_fields(&type_def.fields) {
            declared_fields
                .entry(type_def.name.clone())
                .or_default()
                .insert(field.name.clone());
            if rule.applies_to(field, type_def) {
                optional_fields
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone());
            }
            let Some(named) = named_type(&field.ty) else {
                continue;
            };
            if struct_names.contains(named) {
                field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            }
        }
    }

    IrResultFieldMap {
        field_types,
        optional_fields,
        declared_fields,
        root_type: None,
    }
}

/// Walk `path` from `map.root_type` through the IR struct graph and answer whether the leaf
/// segment is optional on the exact type that owns it.
///
/// `false` — never "unknown" — for an unresolved root, an unrecognized segment, or an unpopulated
/// map. Every one of those is the pre-anchoring answer for a field with no `fields_optional`
/// entry, so this is purely additive: it can only turn a `false` into a `true` when the IR
/// positively confirms the leaf is optional on the type the path reaches. Mirrors
/// `ir_collection::is_collection_path`.
pub(super) fn is_optional_path(map: &IrResultFieldMap, path: &str) -> bool {
    let Some((owner, leaf)) = walk_to_owner(map, path) else {
        return false;
    };
    map.optional_fields
        .get(owner)
        .is_some_and(|fields| fields.contains(&leaf))
}

/// Whether the call's result type declares `path`'s FIRST segment as a binding-visible field.
///
/// `None` when nothing was anchored — no resolved root type, or a root type this map has no
/// fields for (an opaque handle, an enum, a type from outside the extracted surface). Callers
/// must treat `None` as "no answer" and fall back, exactly as `TargetParams::IrAbsent` does;
/// reading it as rejection would empty out every snippet whose result type is not a plain struct.
///
/// Only the first segment is judged. A deeper segment can legitimately walk into a type this map
/// does not carry (a map value, a `serde_json::Value`, a foreign type), and rejecting those would
/// discard real, compiling accessors to close a hole that only ever opened at the root. ~keep
pub(super) fn root_declares_first_segment(map: &IrResultFieldMap, first_segment: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let declared = map.declared_fields.get(root)?;
    Some(declared.contains(first_segment))
}

/// The `(owner_type, leaf_field_name)` a path resolves to, walking every prefix segment through
/// `field_types`. `None` when the root is unresolved or any segment names something the IR does
/// not recognize as a field on the type reached so far.
fn walk_to_owner<'a>(map: &'a IrResultFieldMap, path: &str) -> Option<(&'a str, String)> {
    let root = map.root_type.as_deref()?;
    let segments = parse_path(path);
    let (last, prefix) = segments.split_last()?;

    let mut owner = root;
    for segment in prefix {
        let name = segment_name(segment)?;
        owner = map.field_types.get(owner)?.get(name)?.as_str();
    }
    Some((owner, segment_name(last)?.to_string()))
}
