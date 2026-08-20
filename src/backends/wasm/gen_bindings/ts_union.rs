//! Structural TypeScript type generation for `#[serde(untagged)]` data enums.
//!
//! `enums::is_untagged_data_enum` fields are bridged at runtime as `JsValue` via
//! `serde_wasm_bindgen` (see `enums.rs`'s `~keep` note) — that mechanism is correct and this
//! module does not touch it. What it fixes is the `.d.ts` surface: without a real TypeScript
//! type, wasm-bindgen has nothing but `JsValue` to describe the field, which it renders as
//! `any`. wasm-bindgen's `#[wasm_bindgen(typescript_type = "...")]` extern-type attribute lets a
//! `JsValue`-shaped value carry an arbitrary hand-written TypeScript type instead — confirmed
//! against the pinned wasm-bindgen release by building a throwaway crate and reading the emitted
//! `.d.ts`. The extern type is a zero-cost `JsValue` wrapper (`Into<JsValue>` / `unchecked_into`
//! convert both ways), so it can sit at the getter/setter boundary without changing the
//! underlying field's storage type or the serde bridging that reads/writes it. ~keep
//!
//! The runtime value is a plain JS object/array/primitive produced by `serde_wasm_bindgen`, not
//! a `#[wasm_bindgen]` class instance — so every type this module emits is *structural*
//! (interfaces, type aliases, literal unions), never a reference to a generated wrapper class.

use ahash::{AHashMap, AHashSet};

use crate::codegen::naming::{to_node_name, wire_variant_value};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::enums::is_untagged_data_enum;

/// One named auxiliary TS declaration a variant/field recursively depended on: a struct's
/// interface, a fieldless enum's string-literal union, a nested untagged union's own alias, or
/// (via `build_untagged_enum_ts_plans`) a top-level union's own alias.
enum TsAuxDecl {
    Interface { name: String, fields: Vec<TsField> },
    Alias { name: String, members: Vec<String> },
}

struct TsField {
    name: String,
    ts_type: String,
}

/// The per-enum half of an untagged data enum's TS plan: the Rust-only extern wrapper type a
/// field's getter/setter is typed as, and the small `extern "C" { type ...; }` block that
/// declares it. The TS text itself (the union alias plus every auxiliary interface/alias it
/// depends on) is NOT per-enum — see `AllUntaggedEnumsTsPlan::custom_section`.
pub(super) struct UntaggedEnumTsPlan {
    /// wasm-bindgen resolves this to the exported TS type name (declared in
    /// `AllUntaggedEnumsTsPlan::custom_section`) in the emitted `.d.ts`.
    pub(super) value_type_name: String,
    /// Rendered `extern "C" { type ...; }` block, ready to append to the generated file.
    pub(super) extern_type_declaration: String,
}

/// The complete TS plan for every untagged data enum in one crate's API surface.
pub(super) struct AllUntaggedEnumsTsPlan {
    pub(super) plans: AHashMap<String, UntaggedEnumTsPlan>,
    /// One shared `typescript_custom_section`, or empty if there were no untagged data enums.
    /// Built once, across every enum, so a struct or fieldless enum reachable from more than one
    /// union (e.g. two different unions both carry a `ContentPart`-shaped variant) is declared
    /// exactly once — TypeScript `interface` declarations merge when duplicated, but `type`
    /// aliases do not (confirmed with `tsc`: redeclaring a `type` twice, even identically, is
    /// `TS2300: Duplicate identifier`), so this cannot be built per-enum and concatenated. ~keep
    pub(super) custom_section: String,
}

/// `mod.rs` entry point: filters `api.enums` down to the non-excluded untagged data enums and
/// builds their combined plan. Kept separate from `build_untagged_enum_ts_plans` so the filtering
/// (which `mod.rs` would otherwise have to inline) lives next to the code it's built for.
pub(super) fn build_untagged_enum_ts_plan_for_api(
    api: &ApiSurface,
    exclude_types: &[String],
    opaque_type_names: &AHashSet<String>,
    text_field_enum_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let exclude_types_set: AHashSet<String> = exclude_types.iter().cloned().collect();
    // `untagged_union_text_types` pins these to `String` on the field, the getter and the
    // setter alike. Handing one an extern wrapper type here would retype only the accessors
    // and reintroduce the E0308 that 3678c3e8a fixed, so the text opt-in wins first. ~keep
    let untagged_enum_defs: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| {
            !exclude_types_set.contains(&e.name) && !text_field_enum_names.contains(&e.name) && is_untagged_data_enum(e)
        })
        .collect();
    build_untagged_enum_ts_plans(&untagged_enum_defs, api, &exclude_types_set, opaque_type_names, prefix)
}

/// The per-enum extern wrapper type name, by Rust enum name — what `gen_struct_methods` needs to
/// type an untagged-enum-typed field's getter/setter.
pub(super) fn value_type_names(plan: &AllUntaggedEnumsTsPlan) -> AHashMap<String, String> {
    plan.plans
        .iter()
        .map(|(name, enum_plan)| (name.clone(), enum_plan.value_type_name.clone()))
        .collect()
}

/// Build the full TS plan for every untagged data enum, recursively expanding every variant's
/// payload type.
///
/// `exclude_types` / `opaque_type_names` mark types this backend cannot describe structurally
/// (consumer-excluded, or an opaque handle type with no public fields) — any variant reaching
/// one of those falls back to `any` for that variant's slot only, never for the whole union.
pub(super) fn build_untagged_enum_ts_plans(
    untagged_enums: &[&EnumDef],
    api: &ApiSurface,
    exclude_types: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let mut ctx = TsMapContext {
        api,
        exclude_types,
        opaque_type_names,
        prefix,
        in_progress: AHashSet::default(),
        resolved_names: AHashMap::default(),
        decls: Vec::new(),
    };
    let mut plans = AHashMap::default();

    for enum_def in untagged_enums {
        // A sibling union processed earlier may already have reached this enum as a nested
        // variant payload (see `map_named_enum`'s untagged-enum branch) and queued its alias —
        // in that case its declaration is already pending in `ctx.decls`; only expand it here
        // if that has not happened yet. ~keep
        if !ctx.resolved_names.contains_key(&enum_def.name) {
            ctx.in_progress.insert(enum_def.name.clone());
            let members: Vec<String> = enum_def.variants.iter().map(|v| ctx.map_variant(v)).collect();
            ctx.in_progress.remove(&enum_def.name);
            let ts_type_name = format!("{prefix}{}", enum_def.name);
            ctx.resolved_names.insert(enum_def.name.clone(), ts_type_name.clone());
            ctx.decls.push(TsAuxDecl::Alias {
                name: ts_type_name,
                members,
            });
        }

        let ts_type_name = format!("{prefix}{}", enum_def.name);
        let value_type_name = format!("{ts_type_name}Value");
        let extern_type_declaration = crate::backends::wasm::template_env::render(
            "ts_extern_value_type",
            minijinja::context! {
                ts_type_name => ts_type_name,
                value_type_name => value_type_name.clone(),
            },
        );
        plans.insert(
            enum_def.name.clone(),
            UntaggedEnumTsPlan {
                value_type_name,
                extern_type_declaration,
            },
        );
    }

    let custom_section = if ctx.decls.is_empty() {
        String::new()
    } else {
        let ts_body = ctx.decls.iter().map(render_aux_decl).collect::<Vec<_>>().join("\n\n");
        crate::backends::wasm::template_env::render(
            "ts_custom_section",
            minijinja::context! {
                const_name => "ALEF_UNTAGGED_UNIONS_TS",
                ts_body => ts_body,
            },
        )
    };

    AllUntaggedEnumsTsPlan { plans, custom_section }
}

fn render_alias(name: &str, members: &[String]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_type_alias",
        minijinja::context! { name => name, members => members },
    )
    .trim_end()
    .to_string()
}

fn render_aux_decl(decl: &TsAuxDecl) -> String {
    match decl {
        TsAuxDecl::Interface { name, fields } => render_interface(name, fields),
        TsAuxDecl::Alias { name, members } => render_alias(name, members),
    }
}

fn render_interface(name: &str, fields: &[TsField]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_interface",
        minijinja::context! {
            name => name,
            fields => fields.iter().map(|f| minijinja::context! {
                name => f.name,
                ts_type => f.ts_type,
            }).collect::<Vec<_>>(),
        },
    )
    .trim_end()
    .to_string()
}

fn render_inline_object(fields: &[TsField]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_inline_object",
        minijinja::context! {
            fields => fields.iter().map(|f| minijinja::context! {
                name => f.name,
                ts_type => f.ts_type,
            }).collect::<Vec<_>>(),
        },
    )
    .trim_end()
    .to_string()
}

struct TsMapContext<'a> {
    api: &'a ApiSurface,
    exclude_types: &'a AHashSet<String>,
    opaque_type_names: &'a AHashSet<String>,
    prefix: &'a str,
    /// Names currently being expanded — breaks cycles (self- or mutually-recursive types) by
    /// resolving a re-entrant reference to a name instead of recursing again. Every name in here
    /// resolves to the plain `{prefix}{name}` form (only a fully-resolved fieldless enum gets a
    /// suffixed name — see `resolved_names` — and a fieldless enum has no fields to recurse
    /// through, so it can never appear here mid-expansion).
    in_progress: AHashSet<String>,
    /// Rust source name -> the TS name it was actually declared under, once expansion finishes.
    /// A struct or a nested untagged union keeps the bare `{prefix}{name}`; a fieldless enum gets
    /// a `Wire`-suffixed name instead (see `map_named_enum`) — so a *second* reference to an
    /// already-resolved name must look up the name actually used here rather than recomputing
    /// `{prefix}{name}` blind, or it would point at the wrong (unsuffixed, colliding) name. ~keep
    resolved_names: AHashMap<String, String>,
    decls: Vec<TsAuxDecl>,
}

impl TsMapContext<'_> {
    fn map_variant(&mut self, variant: &EnumVariant) -> String {
        if variant.fields.is_empty() {
            // A fieldless variant of an otherwise data-carrying untagged enum serializes as
            // serde's unit representation: JSON `null`.
            return "null".to_string();
        }
        if variant.is_tuple {
            if variant.fields.len() == 1 {
                return self.map_type(&variant.fields[0].ty);
            }
            let members: Vec<String> = variant.fields.iter().map(|f| self.map_type(&f.ty)).collect();
            return format!("[{}]", members.join(", "));
        }
        let fields = self.map_fields(&variant.fields);
        render_inline_object(&fields)
    }

    fn map_fields(&mut self, fields: &[FieldDef]) -> Vec<TsField> {
        fields
            .iter()
            .map(|f| TsField {
                name: to_node_name(&f.name),
                ts_type: self.map_field_type(f),
            })
            .collect()
    }

    fn map_field_type(&mut self, field: &FieldDef) -> String {
        let base = self.map_type(&field.ty);
        if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
            format!("{base} | undefined")
        } else {
            base
        }
    }

    fn map_type(&mut self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => primitive_ts_type(p).to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
            TypeRef::Bytes => "Uint8Array".to_string(),
            TypeRef::Unit => "null".to_string(),
            // An arbitrary `serde_json::Value` genuinely has no narrower structural type. ~keep
            TypeRef::Json => "any".to_string(),
            TypeRef::Duration => "number".to_string(),
            TypeRef::Optional(inner) => format!("{} | undefined", self.map_type(inner)),
            TypeRef::Vec(inner) => format!("{}[]", self.map_type(inner)),
            TypeRef::Map(key, value) => {
                let key_ts = self.map_type(key);
                if key_ts == "string" {
                    format!("Record<string, {}>", self.map_type(value))
                } else {
                    // A non-string-keyed map doesn't serialize to a plain JSON object, so it
                    // can't be described as a `Record`. ~keep
                    "any".to_string()
                }
            }
            TypeRef::Named(name) => self.map_named(name),
        }
    }

    fn map_named(&mut self, name: &str) -> String {
        if self.exclude_types.contains(name) || self.opaque_type_names.contains(name) {
            return "any".to_string();
        }
        if let Some(resolved) = self.resolved_names.get(name) {
            return resolved.clone();
        }
        let ts_name = format!("{}{name}", self.prefix);
        if self.in_progress.contains(name) {
            return ts_name;
        }
        if let Some(enum_def) = self.api.enums.iter().find(|e| e.name == name) {
            return self.map_named_enum(enum_def, ts_name);
        }
        if let Some(type_def) = self.api.types.iter().find(|t| t.name == name) {
            return self.map_named_struct(type_def, ts_name);
        }
        // Unresolvable: a generic parameter or a type outside this crate's API surface.
        "any".to_string()
    }

    fn map_named_struct(&mut self, type_def: &TypeDef, ts_name: String) -> String {
        if type_def.is_opaque {
            return "any".to_string();
        }
        self.in_progress.insert(type_def.name.clone());
        let fields = self.map_fields(&type_def.fields);
        self.in_progress.remove(&type_def.name);
        self.resolved_names.insert(type_def.name.clone(), ts_name.clone());
        self.decls.push(TsAuxDecl::Interface {
            name: ts_name.clone(),
            fields,
        });
        ts_name
    }

    fn map_named_enum(&mut self, enum_def: &EnumDef, ts_name: String) -> String {
        if enum_def.variants.iter().all(|v| v.fields.is_empty()) {
            // A fieldless enum always gets its own real `Wasm{Enum}` wasm-bindgen TS `enum`
            // elsewhere in the file (see `gen_enum`), unconditionally — regardless of whether
            // any struct field references it by that name. That native enum's runtime value is
            // the numeric ABI discriminant, NOT the serde wire string this union member actually
            // carries, so the two are incompatible representations that happen to share a Rust
            // name — reusing the bare name would either collide (a `type` alias cannot merge
            // with an `enum`, confirmed with `tsc`) or, if it merged, describe the wrong runtime
            // shape. A distinct suffix sidesteps both. ~keep
            let literal_name = format!("{ts_name}Wire");
            let values: Vec<String> = enum_def
                .variants
                .iter()
                .map(|v| {
                    let wire =
                        wire_variant_value(&v.name, v.serde_rename.as_deref(), enum_def.serde_rename_all.as_deref());
                    format!("\"{wire}\"")
                })
                .collect();
            self.resolved_names.insert(enum_def.name.clone(), literal_name.clone());
            self.decls.push(TsAuxDecl::Alias {
                name: literal_name.clone(),
                members: values,
            });
            return literal_name;
        }
        if is_untagged_data_enum(enum_def) {
            // This enum gets its own top-level entry in `untagged_enums`, processed by the same
            // driving loop that reached `self` (before or after — order doesn't matter to TS).
            // That loop's own guard (`!resolved_names.contains_key(name)`) decides whether it
            // still needs expanding, so `resolved_names` must NOT be written here: registering it
            // now — before that loop reaches it — would make the loop skip it, and since a bare
            // reference (unlike the struct/fieldless-enum branches above) pushes no decl of its
            // own, the enum's `type` alias would then never be emitted at all, leaving
            // `typescript_type = "{ts_name}"` dangling. Just return the reference. ~keep
            return ts_name;
        }
        // An internally-tagged data enum (`#[serde(tag = "...")]`) or another shape this module
        // doesn't structurally model — its own struct-with-discriminator wrapper is a different
        // representation than the plain-JSON shape this module describes. ~keep
        "any".to_string()
    }
}

fn primitive_ts_type(prim: &crate::core::ir::PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match prim {
        PrimitiveType::Bool => "boolean",
        PrimitiveType::U64 | PrimitiveType::I64 => "bigint",
        PrimitiveType::U8
        | PrimitiveType::U16
        | PrimitiveType::U32
        | PrimitiveType::I8
        | PrimitiveType::I16
        | PrimitiveType::I32
        | PrimitiveType::F32
        | PrimitiveType::F64
        | PrimitiveType::Usize
        | PrimitiveType::Isize => "number",
    }
}

#[cfg(test)]
mod tests;
