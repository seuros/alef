use crate::backends::swift::gen_bindings::dto::needs_json_bridge_for_swift;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::e2e::escape::escape_java as escape_swift_str;
use crate::e2e::field_access::SwiftFirstClassMap;
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

/// Returns true when `element_type` names a scalar Rust/Swift element type.
///
/// Scalar element types describe `Vec<T>` Rust parameters that the swift-bridge
/// surface exposes as native Swift `[T]` arrays — these can be constructed from
/// a Swift array literal without any opaque-type intermediate. Object element
/// types (everything else) require an `options_via` configuration to construct.
pub(super) fn is_scalar_element_type(element_type: Option<&str>) -> bool {
    matches!(
        element_type.map(str::trim),
        Some(
            "String"
                | "str"
                | "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "usize"
                | "f32"
                | "f64",
        )
    )
}

pub(super) fn from_json_helper_for_arg(arg: &crate::e2e::config::ArgMapping, options_type: Option<&str>) -> String {
    let type_name = arg
        .element_type
        .as_deref()
        .or(options_type)
        .unwrap_or(arg.name.as_str());
    format!("{}FromJson", type_name.to_lower_camel_case())
}

pub(super) fn json_to_swift(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_swift(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_swift).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_swift(&json_str))
        }
    }
}

/// When comparing numeric values in Swift, integer and floating-point literals
/// should not be wrapped in type constructors. Swift's type inference will infer
/// the correct type based on the field expression's return type.
///
/// Booleans ("true"/"false") are never wrapped — they are Swift `Bool` literals
/// and should never be cast to numeric types.
///
/// Floating-point literals should never be wrapped, as they may compare against
/// fields that return `Double` or other floating-point types.
pub(super) fn swift_numeric_literal_cast(_field_expr: &str, numeric_literal: &str) -> String {
    // Never wrap booleans.
    if numeric_literal == "true" || numeric_literal == "false" {
        return numeric_literal.to_string();
    }

    // Don't wrap any numeric literals — Swift's type inference will handle it.
    // This avoids type mismatches when fields return specific types like UInt16,
    // UInt32, Int, etc. The comparison operator and field type will guide inference.
    numeric_literal.to_string()
}

/// Escape a string for embedding in a Swift double-quoted string literal.
pub(super) fn escape_swift(s: &str) -> String {
    escape_swift_str(s)
}

/// Resolve the IR type name backing this call's result.
///
/// Lookup order mirrors PHP's `derive_root_type` for `[crates.e2e.calls.*]`
/// configs: any of `c, csharp, java, kotlin, go, php` overrides may carry a
/// `result_type = "ChatCompletionResponse"` field. The first non-empty value
/// wins. These overrides are language-agnostic IR type names — they were
/// originally added for the C/C# backends and other backends piggy-back on them
/// because the IR names are shared across every binding.
///
/// Returns `None` when no override sets `result_type`; the renderer then falls
/// back to the workspace-default heuristic in `SwiftFirstClassMap` (which
/// defaults to property access — the right call for first-class result types
/// like `FileObject` but wrong for opaque types like `ChatCompletionResponse`).
pub(super) fn swift_call_result_type(call_config: &crate::core::config::e2e::CallConfig) -> Option<String> {
    const LOOKUP_LANGS: &[&str] = &["c", "csharp", "java", "kotlin", "go", "php"];
    for lang in LOOKUP_LANGS {
        if let Some(o) = call_config.overrides.get(*lang)
            && let Some(rt) = o.result_type.as_deref()
            && !rt.is_empty()
        {
            return Some(rt.to_string());
        }
    }
    None
}

pub(super) fn swift_client_factory_call(factory: &str, api_key: &str, base_url: &str) -> String {
    format!("let _client = try {factory}(apiKey: {api_key}, baseUrl: {base_url})")
}

pub(super) fn resolve_streaming_adapter<'a>(
    config: &'a ResolvedCrateConfig,
    call_config: &crate::core::config::e2e::CallConfig,
    function_name: &str,
    client_factory: Option<&str>,
) -> Option<&'a crate::core::config::AdapterConfig> {
    let owner_type = client_factory.filter(|value| value.chars().next().is_some_and(char::is_uppercase));
    config
        .adapters
        .iter()
        .find(|adapter| {
            matches!(adapter.pattern, AdapterPattern::Streaming)
                && adapter.name.to_lower_camel_case() == function_name
                && owner_type.is_none_or(|owner| adapter.owner_type.as_deref() == Some(owner))
        })
        .or_else(|| {
            call_config.overrides.values().find_map(|override_config| {
                override_config.result_type.as_deref().and_then(|result_type| {
                    config.adapters.iter().find(|adapter| {
                        matches!(adapter.pattern, AdapterPattern::Streaming)
                            && adapter.name.to_lower_camel_case() == function_name
                            && adapter.item_type.as_deref() == Some(result_type)
                    })
                })
            })
        })
}

/// Returns true when the field type would be emitted as a Swift primitive value
/// or a known first-class Codable struct/unit-enum, so it can appear on a
/// first-class Codable Swift struct without forcing the host type into a
/// typealias. Mirrors `first_class_field_supported` in alef-backend-swift.
///
/// Accepts:
/// - `Primitive` and `String`
/// - `Named(S)` when `S` is in `known_dto_names` (seeded with unit-serde enums and
///   grown via fixed-point iteration over candidate struct DTOs)
/// - `Vec<T>` and `Optional<T>` recursively
///
/// Rejects `Map`, `Path`, `Bytes`, `Duration`, `Char`, `Json`, and unknown
/// `Named(_)` references (the backend treats those as typealias-to-opaque).
pub(super) fn swift_first_class_field_supported(
    ty: &crate::core::ir::TypeRef,
    known_dto_names: &HashSet<String>,
) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(_) | TypeRef::String => true,
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => swift_first_class_field_supported(inner, known_dto_names),
        _ => false,
    }
}

/// Build the per-type Swift first-class/opaque classification map used by
/// `render_swift_with_first_class_map`.
///
/// A TypeDef is treated as first-class (Codable Swift struct → property access)
/// when it is not opaque, has serde derives, has at least one field, and every
/// binding field is supported by `swift_first_class_field_supported` against the
/// current first-class set. All other public types end up as typealiases to
/// opaque `RustBridge.X` classes whose fields are swift-bridge methods
/// (`.id()`, `.status()`).
///
/// Mirrors the fixed-point iteration in `alef-backend-swift::gen_bindings.rs`
/// (lines 100-130). Without the fixed point, a type like `TranscriptionResponse`
/// that holds `Option<Vec<TranscriptionSegment>>` would be wrongly classified
/// opaque, causing the renderer to emit `.text()` against a first-class struct
/// whose `text` is a `public let` property.
///
/// `field_types` records the next-type that each Named field traverses into,
/// so the renderer can advance its current-type cursor through nested
/// `data[0].id` style paths.
///
/// `call_config` is used to resolve the explicit `result_type` override via
/// `swift_call_result_type()`. When available, this override takes precedence
/// over the fallback heuristic of finding a TypeDef that contains all
/// `result_fields` (which fails when result_fields is workspace-global across
/// many call sites with different result types like ChatCompletionResponse,
/// EmbeddingResponse, ModelsListResponse, etc.).
pub(super) fn build_swift_first_class_map(
    type_defs: &[crate::core::ir::TypeDef],
    enum_defs: &[crate::core::ir::EnumDef],
    e2e_config: &crate::e2e::config::E2eConfig,
    call_config: &crate::core::config::e2e::CallConfig,
) -> SwiftFirstClassMap {
    use crate::core::ir::TypeRef;
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut vec_field_names: HashSet<String> = HashSet::new();
    // Field names that appear as a JSON-bridged vec (`Option<Vec<T>>`,
    // `Vec<Vec<..>>`, etc.) on some type — these get a `RustString` getter with
    // no `.count`. `vec_field_names` keys on bare names, so any name recorded
    // here is later removed from the countable set to avoid emitting `.count`
    // on the RustString flavour (a compile error). Skipping the count is safe.
    let mut json_bridged_vec_names: HashSet<String> = HashSet::new();
    fn inner_named(ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::Named(n) => Some(n.clone()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    fn is_vec_ty(ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Vec(_) => true,
            TypeRef::Optional(inner) => is_vec_ty(inner),
            _ => false,
        }
    }
    /// Returns true when `ty` is `Vec<Named(X)>` (or `Option<Vec<Named(X)>>`) and
    /// `field_optional` is set, for an `X` whose real Swift getter collapses the
    /// *whole* field to a single JSON-encoded `String` instead of a per-element
    /// `Vec<String>`/`Vec<Wrapper>`.
    ///
    /// Mirrors `emit_vec_enum_string_getter`/`emit_vec_struct_serde_getter`
    /// (`gen_rust_crate::wrappers::getters`), which special-case `field.optional`
    /// on top of `needs_json_bridge_for_swift`:
    /// - `Vec<Named(enum)>`: always serialized via `emit_vec_enum_string_getter`,
    ///   which emits `getter_vec_enum_string_optional.jinja` (whole-field
    ///   `serde_json::to_string(&self.0.<field>) -> String`) when optional, vs.
    ///   `getter_vec_enum_string.jinja` (`Vec<String>`, one element per call) when not.
    /// - `Vec<Named(struct)>` on a first-class parent DTO with serde: routed through
    ///   `emit_vec_struct_serde_getter`, which shares the same optional/non-optional
    ///   template split (whole-field `String` vs. per-element `Vec<String>`).
    ///
    /// `needs_json_bridge_for_swift` alone can't see this: it only looks at the
    /// field's own type shape (`Vec<Named(_)>` is always a leaf-inner Vec, so it
    /// never reports these as bridged), so the caller must OR this predicate in.
    fn swift_optional_vec_of_named_is_string_getter(
        ty: &TypeRef,
        field_optional: bool,
        enum_names: &HashSet<&str>,
        has_serde_names: &HashSet<&str>,
        parent_first_class: bool,
    ) -> bool {
        if !field_optional {
            return false;
        }
        // Field extraction (`unwrap_optional`) always strips a top-level `Optional` off
        // `field.ty` into the `optional` bool, so `ty` is normally `Vec(Named(_))` already.
        // Some test fixtures (and any future caller) may still pass the un-stripped
        // `Optional(Vec(Named(_)))` shape, so unwrap defensively to stay in sync with
        // `is_vec_ty`/`needs_json_bridge_for_swift`, which both do the same.
        let vec_ty = match ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        let TypeRef::Vec(inner) = vec_ty else {
            return false;
        };
        let TypeRef::Named(name) = inner.as_ref() else {
            return false;
        };
        if enum_names.contains(name.as_str()) {
            return true;
        }
        has_serde_names.contains(name.as_str()) && parent_first_class
    }
    let has_serde_names: HashSet<&str> = type_defs
        .iter()
        .filter(|td| td.has_serde)
        .map(|td| td.name.as_str())
        .collect();
    // Seed with unit serde enum names — Codable on the Swift side and can appear
    // as leaf fields on struct DTOs. Also seed data-variant enums (tagged + untagged)
    // that have any fields, matching gen_bindings.rs which seeds both unit + data enums.
    // This ensures containing structs (like ChatCompletionResponse holding Choice holding
    // AssistantContent) are classified as first-class when all their fields are supported.
    let unit_serde_enum_names: HashSet<String> = enum_defs
        .iter()
        .filter(|e| e.has_serde && e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.clone())
        .collect();

    let data_variant_enum_names: HashSet<String> = enum_defs
        .iter()
        .filter(|e| e.has_serde && e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.clone())
        .collect();

    let mut known_dto_names: HashSet<String> = unit_serde_enum_names.clone();
    known_dto_names.extend(data_variant_enum_names.iter().cloned());

    // Candidate struct DTOs: non-opaque, has_serde, non-empty fields.
    // Trait types and binding-excluded types are skipped (matches backend semantics
    // — note backend further filters via `exclude_types`, which we don't have here,
    // but accepting a superset is safe: types not actually emitted simply never
    // appear in path-access chains).
    let candidates: Vec<&crate::core::ir::TypeDef> = type_defs
        .iter()
        .filter(|td| !td.is_trait && !td.is_opaque && td.has_serde && !td.fields.is_empty())
        .collect();

    loop {
        let prev = known_dto_names.len();
        for td in &candidates {
            if known_dto_names.contains(&td.name) {
                continue;
            }
            let all_supported = td
                .fields
                .iter()
                .filter(|f| !f.binding_excluded)
                .all(|f| swift_first_class_field_supported(&f.ty, &known_dto_names));
            if all_supported {
                known_dto_names.insert(td.name.clone());
            }
        }
        if known_dto_names.len() == prev {
            break;
        }
    }

    // The first-class set on SwiftFirstClassMap conceptually represents structs
    // accessed via property syntax. Unit enums never appear as the *owner* of a
    // chain segment (they are leaves), but including them is harmless since
    // `advance()` never returns them as a current_type for further traversal.
    let first_class_types: HashSet<String> = candidates
        .iter()
        .filter(|td| known_dto_names.contains(&td.name))
        .map(|td| td.name.clone())
        .collect();

    use crate::e2e::field_access::{StringyField, StringyFieldKind};
    // Enums are bridged as `String` on the swift-bridge surface (the binding
    // emits `fn kind(&self) -> String` for `kind: SomeEnum`), so they must
    // also count as text-bearing accessors when aggregating contains-matchers.
    let enum_names: HashSet<&str> = enum_defs.iter().map(|e| e.name.as_str()).collect();
    let classify_stringy = |ty: &TypeRef, field_optional: bool| -> Option<StringyFieldKind> {
        match ty {
            TypeRef::String => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Optional),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Optional),
                _ => None,
            },
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Vec),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Vec),
                _ => None,
            },
            _ => None,
        }
    };
    let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
    for td in type_defs {
        let mut td_field_types: HashMap<String, String> = HashMap::new();
        let mut td_stringy: Vec<StringyField> = Vec::new();
        for f in &td.fields {
            if let Some(named) = inner_named(&f.ty) {
                td_field_types.insert(f.name.clone(), named);
            }
            // Record a Vec field as countable only when its getter is a real
            // `RustVec` (which has `.count`, or `Optional<RustVec<T>>` which
            // has `?.count`). A field that structurally JSON-bridges —
            // `Vec<Vec<..>>`, `Map<..>`, etc. — returns a plain `RustString`
            // instead, which has no `.count`; recording such a field as
            // countable would make the e2e emit `.count` on a `RustString`
            // and fail to compile.
            //
            // `needs_json_bridge_for_swift` is the exact predicate the Swift
            // binding generator (`gen_bindings::dto`) uses to decide a getter's
            // return type, so classification here must match the real getter
            // shape. In particular `Option<Vec<T>>` (e.g. `elements: Option<Vec<Element>>`,
            // `extracted_keywords: Option<Vec<Keyword>>`) is NOT JSON-bridged when `T`
            // is itself a leaf (opaque swift-bridge type, primitive, or String):
            // swift-bridge natively exposes it as `Optional<RustVec<T>>`, which has no
            // `.toString()` but IS countable via `?.count`. Only genuinely JSON-bridged
            // shapes — `Vec<Vec<..>>`, `Map<..>`, `Option<Vec<Vec<..>>>`, etc. — return a
            // plain `RustString` with no `.count`. Do NOT blanket-gate on `f.optional`: an
            // earlier version did, which misclassified every optional Vec (including the
            // natively-bridged `Optional<RustVec<T>>` case) as JSON-bridged, and made the
            // e2e emit `<accessor>().toString().count` against `RustVec<T>?` — a compile
            // error ("value of type 'RustVec<Element>?' has no member 'toString'").
            //
            // `f.optional` DOES matter for one narrower shape, though:
            // `swift_optional_vec_of_named_is_string_getter` below catches
            // `Option<Vec<Named(enum)>>` and `Option<Vec<Named(struct-with-serde)>>` on a
            // first-class parent DTO. `needs_json_bridge_for_swift` alone reports these as
            // countable (a `Vec<Named(_)>` inner is always leaf-inner), but
            // `emit_vec_enum_string_getter`/`emit_vec_struct_serde_getter`
            // (`gen_rust_crate::wrappers::getters`) collapse the *whole optional field* to
            // a single `serde_json::to_string(&self.0.<field>) -> String` when the field is
            // optional — e.g. `headings: Option<Vec<HeadingInfo>>` emits
            // `fn headings(&self) -> String`, not a countable `RustVec`. Only the
            // non-optional variant of those getters returns a per-element `Vec<String>`
            // (still countable). Optionality on genuinely-countable shapes is handled
            // separately by `swift_array_count_expr`/`swift_count_target` in
            // `accessors.rs`, which already emit `(expr?.count ?? 0)` for optional Vec leaves.
            if is_vec_ty(&f.ty) {
                let is_string_getter = needs_json_bridge_for_swift(&f.ty)
                    || swift_optional_vec_of_named_is_string_getter(
                        &f.ty,
                        f.optional,
                        &enum_names,
                        &has_serde_names,
                        first_class_types.contains(&td.name),
                    );
                if is_string_getter {
                    json_bridged_vec_names.insert(f.name.clone());
                } else {
                    vec_field_names.insert(f.name.clone());
                }
            }
            if f.binding_excluded {
                continue;
            }
            if let Some(kind) = classify_stringy(&f.ty, f.optional) {
                td_stringy.push(StringyField {
                    name: f.name.clone(),
                    kind,
                });
            }
        }
        if !td_field_types.is_empty() {
            field_types.insert(td.name.clone(), td_field_types);
        }
        if !td_stringy.is_empty() {
            stringy_fields_by_type.insert(td.name.clone(), td_stringy);
        }
    }
    // Drop any field name that is JSON-bridged (RustString getter) on some
    // type from the countable set. Because the map keys on bare names, a name
    // that is a countable `RustVec` on one type but a `RustString` on another
    // (e.g. `elements`: `Vec<InternalElement>` vs `Option<Vec<Element>>`) is
    // ambiguous; treating it as non-countable and skipping the `.count`
    // assertion is always safe, whereas emitting `.count` on a `RustString`
    // fails to compile.
    for name in &json_bridged_vec_names {
        vec_field_names.remove(name);
    }

    // Root-type detection: first check for an explicit `result_type` override
    // in the call config. If present, use that directly. Otherwise fall back to
    // picking a unique TypeDef that contains all `result_fields`.
    let root_type = swift_call_result_type(call_config).or_else(|| {
        if e2e_config.result_fields.is_empty() {
            None
        } else {
            let matches: Vec<&crate::core::ir::TypeDef> = type_defs
                .iter()
                .filter(|td| {
                    let names: HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
                    e2e_config.result_fields.iter().all(|rf| names.contains(rf.as_str()))
                })
                .collect();
            if matches.len() == 1 {
                Some(matches[0].name.clone())
            } else {
                None
            }
        }
    });
    SwiftFirstClassMap {
        first_class_types,
        field_types,
        vec_field_names,
        root_type,
        stringy_fields_by_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{CallConfig, E2eConfig};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    fn named_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// Regression test for the Swift e2e `count_min`/`min_length` assertion emitter
    /// crash: `Option<Vec<Named>>` fields (e.g. `elements: Option<Vec<Element>>`,
    /// `extracted_keywords: Option<Vec<Keyword>>`) are natively bridged by swift-bridge
    /// as `Optional<RustVec<T>>` — never JSON-bridged to a `RustString` — so they must
    /// be classified as countable (`vec_field_names`), NOT as `json_bridged_vec_names`.
    /// Previously an `f.optional ||` disjunct misclassified every optional Vec as
    /// JSON-bridged regardless of its element type, which made the e2e generator emit
    /// `<accessor>().toString().count` against `RustVec<Element>?` — a compile error
    /// ("value of type 'RustVec<Element>?' has no member 'toString'").
    #[test]
    fn optional_vec_of_named_leaf_is_countable_not_json_bridged() {
        let element_dto = TypeDef {
            name: "Element".to_string(),
            fields: vec![named_field("kind", TypeRef::String, false)],
            ..Default::default()
        };
        let result_dto = TypeDef {
            name: "ExtractionResult".to_string(),
            fields: vec![named_field(
                "elements",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Element".to_string()))))),
                true,
            )],
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[element_dto, result_dto],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            map.is_vec_field_name("elements"),
            "Option<Vec<Named>> field must be classified as a countable RustVec, not JSON-bridged"
        );
    }

    /// Genuinely JSON-bridged shapes — `Vec<Vec<T>>` (and `Option<Vec<Vec<T>>>`) —
    /// return a plain `RustString` getter with no `.count`, so they must stay excluded
    /// from `vec_field_names` (and thus never get a `.count` assertion emitted).
    #[test]
    fn optional_vec_of_vec_stays_json_bridged() {
        let result_dto = TypeDef {
            name: "TableResult".to_string(),
            fields: vec![named_field(
                "rows",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(
                    TypeRef::String,
                )))))),
                true,
            )],
            ..Default::default()
        };

        let map = build_swift_first_class_map(&[result_dto], &[], &E2eConfig::default(), &CallConfig::default());

        assert!(
            !map.is_vec_field_name("rows"),
            "Vec<Vec<T>> field is JSON-bridged to RustString and must not be marked countable"
        );
    }

    /// Regression test for the `headings()?.count` swift e2e compile failure:
    /// `headings: Option<Vec<HeadingInfo>>` where `HeadingInfo` is a serde struct.
    /// Because the parent type (`Metadata`) is first-class and `HeadingInfo` has serde,
    /// `emit_vec_struct_serde_getter` (`gen_rust_crate::wrappers::getters`) collapses the
    /// whole optional field to `fn headings(&self) -> String` (whole-field
    /// `serde_json::to_string`), not a countable `RustVec`/`Vec<String>`. Emitting
    /// `headings()?.count` against that `String` fails with "cannot use optional
    /// chaining on non-optional value of type 'RustString'" / "value of type
    /// 'RustString' has no member 'count'". `headings` must be classified as
    /// JSON-bridged (non-countable) so the e2e generator never emits `.count` on it.
    #[test]
    fn optional_vec_of_serde_struct_on_first_class_parent_is_json_bridged() {
        let heading_info = TypeDef {
            name: "HeadingInfo".to_string(),
            fields: vec![named_field("text", TypeRef::String, false)],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "headings",
                TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                true,
            )],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[heading_info, metadata],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            !map.is_vec_field_name("headings"),
            "Option<Vec<Named(struct)>> on a first-class parent is a whole-field String getter \
             and must not be marked countable"
        );
    }

    /// Regression test for the non-optional sibling of the `headings` shape above:
    /// `Vec<Named(struct)>` (not optional) on a first-class serde parent routes through
    /// `emit_vec_struct_serde_getter`'s non-optional template, which returns a per-element
    /// `Vec<String>` — real, countable via `.count`. Only the `Option<...>` variant
    /// collapses to a whole-field `String`.
    #[test]
    fn non_optional_vec_of_serde_struct_on_first_class_parent_is_countable() {
        let heading_info = TypeDef {
            name: "HeadingInfo".to_string(),
            fields: vec![named_field("text", TypeRef::String, false)],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![named_field(
                "headings",
                TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                false,
            )],
            has_serde: true,
            ..Default::default()
        };

        let map = build_swift_first_class_map(
            &[heading_info, metadata],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );

        assert!(
            map.is_vec_field_name("headings"),
            "non-optional Vec<Named(struct)> on a first-class parent returns a countable Vec<String>"
        );
    }
}
