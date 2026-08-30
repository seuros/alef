//! Which PHP shape `backends::php` lowers each IR enum to, and therefore whether a fixture
//! field path that steps *into* an enum's variant payload names anything the PHP binding
//! actually exposes.
//!
//! ~keep This replaces a literal `f.contains("metadata.format.")` guard that carried the reason
//! "PHP bindings serialize FormatMetadata to JSON, so variants are unavailable in PHP". Both
//! halves were wrong. The path was one consumer crate's own type names hard-coded into a
//! project-agnostic generator, so the identical shape in any other crate was not recognised at
//! all and rendered a property chain against a PHP `string`. And the reason is false for the
//! very shape it named: an internally tagged data enum is lowered to a flat `#[php_class]`
//! whose variant payloads are real `#[php(getter)]`-backed properties, so those variants ARE
//! reachable and the assertion should run.
//!
//! The real condition is the enum's PHP *lowering*, which is a property of the enum's own IR
//! shape and nothing else.

use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant};
use crate::codegen::serde_enum_repr::tagged_object_tag_key;
use crate::e2e::field_access::PhpGetterMap;

/// How `backends::php` lowers each IR enum, partitioned exactly as
/// `backends::php::gen_bindings::rust_bindings::generate_bindings` partitions `api.enums`.
///
/// ~keep The predicates behind the partition (`is_tagged_data_enum`, `is_untagged_data_enum`,
/// `flat_field_name` in `backends::php::gen_bindings::types::enums`) are the source of truth for
/// this file, but `backends::php`'s `gen_bindings` module is private so they cannot be called
/// from here. They are restated below rather than approximated — each is a direct read of IR
/// fields — and `should_match_the_binding_backends_partition` pins the resulting shapes.
/// Re-exporting the three from `backends::php` and deleting the copies is the right follow-up;
/// `is_php_prop_scalar` is already re-exported and IS called directly, which is why this type
/// exposes the enum-name set that predicate takes rather than restating the predicate too.
#[derive(Debug, Default)]
pub(super) struct PhpEnumLowering {
    /// Lowered to a flat `#[php_class]` struct: an `Option<T>` field per variant payload plus a
    /// discriminator, each payload exposed through `#[php(getter)] pub fn get_<flat>()`, which
    /// ext-php-rs registers as the read-only PHP property `<flat>`.
    flat_class: HashSet<String>,
    /// Lowered to a PHP `string` and therefore accepted as a `#[php(prop)]` scalar on any struct
    /// field that names one. This is the exact set `backends::php` passes to
    /// [`crate::backends::php::is_php_prop_scalar`] as its `enum_names` argument.
    string_valued: HashSet<String>,
    /// `#[serde(untagged)]` with data-carrying variants: the binding holds a
    /// `serde_json::Value` and reads it back through a getter returning a JSON `String`. Neither
    /// a `#[php(prop)]` scalar nor a class with members, so it is in its own set.
    json_bridged: HashSet<String>,
}

impl PhpEnumLowering {
    pub(super) fn from_enums(enums: &[EnumDef]) -> Self {
        let mut lowering = Self::default();
        for enum_def in enums {
            if is_tagged_data_enum(enum_def) {
                lowering.flat_class.insert(enum_def.name.clone());
            } else if is_untagged_data_enum(enum_def) {
                lowering.json_bridged.insert(enum_def.name.clone());
            } else {
                lowering.string_valued.insert(enum_def.name.clone());
            }
        }
        lowering
    }

    /// The `enum_names` set the PHP binding backend's own scalar predicate is called with.
    pub(super) fn php_prop_scalar_enum_names(&self) -> ahash::AHashSet<String> {
        self.string_valued.iter().cloned().collect()
    }

    /// Every readable PHP property on the flat class for `enum_def`, or `None` when the enum is
    /// not lowered to a flat class.
    pub(super) fn flat_class_properties(&self, enum_def: &EnumDef) -> Option<Vec<FlatProperty>> {
        if !self.flat_class.contains(&enum_def.name) {
            return None;
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut properties = Vec::new();
        for variant in &enum_def.variants {
            for (index, field) in variant.fields.iter().enumerate() {
                let name = flat_field_name(variant, index);
                if seen.insert(name.clone()) {
                    properties.push(FlatProperty {
                        payload_type: super::types::inner_named(&field.ty),
                        name,
                    });
                }
            }
        }
        properties.push(FlatProperty {
            name: format!("{}_tag", tagged_object_tag_key(enum_def)),
            payload_type: None,
        });
        Some(properties)
    }

    /// True when this crate declares `name` as an enum the PHP binding renders with no readable
    /// members, so a field path may not traverse into it.
    fn is_memberless(&self, name: &str) -> bool {
        self.string_valued.contains(name) || self.json_bridged.contains(name)
    }

    fn is_flat_class(&self, name: &str) -> bool {
        self.flat_class.contains(name)
    }
}

/// Whether a flat data-enum property's PHP name is one the e2e accessor renderer can spell.
///
/// ~keep `#[php(getter)] pub fn get_<flat>` does NOT register a method. ext-php-rs strips the
/// literal `get_` prefix off the RAW Rust ident with NO case conversion and registers a read-only
/// property under that snake_case name, so `get_fiction_book` is `$format->fiction_book` — the
/// exact opposite of the struct path, whose `#[php(prop, name = to_php_name(..))]` really is
/// lowerCamelCase (see `type_stubs.rs::gen_data_enum_property_declarations`).
///
/// `field_access::optional_renderers::render_php_with_getters` lowerCamelCases every path segment
/// unconditionally, which is correct for struct props and wrong for these. Single-word flat names
/// are unaffected (`spreadsheet` casts to itself) and render correctly today; multi-word ones would
/// emit `->fictionBook`, a property that does not exist. Teaching the renderer the difference means
/// editing `field_access`, so until then a multi-word flat property is refused as alef's own
/// generator gap rather than emitted wrong.
fn renderer_can_spell(flat_property: &str) -> bool {
    flat_property == heck::ToLowerCamelCase::to_lower_camel_case(flat_property)
}

/// What the PHP binding offers for the enum-variant segment of a field path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VariantAccess {
    /// Nothing to refuse: the path traverses no enum, or every enum on it is a flat class whose
    /// property the renderer can spell.
    Available,
    /// The path steps into an enum the binding lowered to a plain `string` (unit variants) or a
    /// JSON `string` (`#[serde(untagged)]` data). There is no variant accessor to call, and no
    /// consumer config change creates one.
    NoAccessor,
    /// The flat class DOES expose the property, but its snake_case PHP name is one the shared
    /// accessor renderer cannot currently produce. See [`renderer_can_spell`].
    UnspellableFlatProperty,
}

/// One readable property on a flat data-enum class.
pub(super) struct FlatProperty {
    pub(super) name: String,
    /// The IR type the property's payload traverses into, when the path can keep walking.
    pub(super) payload_type: Option<String>,
}

/// Answers, for one call, whether a fixture field path names an enum-variant accessor the PHP
/// binding never emitted.
pub(super) struct PhpVariantAccess<'a> {
    getter_map: &'a PhpGetterMap,
    lowering: &'a PhpEnumLowering,
}

impl PhpVariantAccess<'static> {
    /// An oracle that never refuses, for tests whose field paths traverse no enum at all.
    #[cfg(test)]
    pub(super) fn none() -> Self {
        static GETTER_MAP: std::sync::OnceLock<PhpGetterMap> = std::sync::OnceLock::new();
        static LOWERING: std::sync::OnceLock<PhpEnumLowering> = std::sync::OnceLock::new();
        Self {
            getter_map: GETTER_MAP.get_or_init(PhpGetterMap::default),
            lowering: LOWERING.get_or_init(PhpEnumLowering::default),
        }
    }
}

impl<'a> PhpVariantAccess<'a> {
    pub(super) fn new(getter_map: &'a PhpGetterMap, lowering: &'a PhpEnumLowering) -> Self {
        Self { getter_map, lowering }
    }

    /// What the PHP binding offers for `field`'s enum-variant segment, if it has one.
    ///
    /// ~keep The walk is anchored the way `PhpGetterMap` anchors accessor rendering: from the
    /// call's resolved `root_type`, one declared field at a time. Every "I do not know" answer (no
    /// root type, a segment the map has never seen, a name this crate declares no enum for) yields
    /// [`VariantAccess::Available`] and leaves the assertion rendering exactly as it does today, so
    /// this can only refuse paths the IR positively confirms are unreachable.
    pub(super) fn classify(&self, field: &str) -> VariantAccess {
        let Some(root) = self.getter_map.root_type.as_deref() else {
            return VariantAccess::Available;
        };
        let mut owner = root.to_string();
        let mut remaining = field.split('.').peekable();
        while let Some(segment) = remaining.next() {
            let Some(next_segment) = remaining.peek().copied() else {
                return VariantAccess::Available;
            };
            let name = segment.split('[').next().unwrap_or(segment);
            if !self
                .getter_map
                .all_fields
                .get(&owner)
                .is_some_and(|fields| fields.contains(name))
            {
                return VariantAccess::Available;
            }
            let Some(next) = self.getter_map.advance(Some(&owner), name) else {
                return VariantAccess::Available;
            };
            if self.lowering.is_memberless(&next) {
                return VariantAccess::NoAccessor;
            }
            let property = next_segment.split('[').next().unwrap_or(next_segment);
            if self.lowering.is_flat_class(&next) && !renderer_can_spell(property) {
                return VariantAccess::UnspellableFlatProperty;
            }
            owner = next;
        }
        VariantAccess::Available
    }

    pub(super) fn is_unavailable(&self, field: &str) -> bool {
        self.classify(field) != VariantAccess::Available
    }
}

/// Return true if an enum is a "tagged data enum" — has a serde tag AND at least one variant
/// carrying data. Restated from `backends::php::gen_bindings::types::enums::is_tagged_data_enum`;
/// see [`PhpEnumLowering`]'s note. ~keep
fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|variant| !variant.fields.is_empty())
}

/// Return true if an enum is an "untagged data enum" — `#[serde(untagged)]` AND at least one
/// variant carrying data. Restated from
/// `backends::php::gen_bindings::types::enums::is_untagged_data_enum`. ~keep
fn is_untagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_untagged && enum_def.variants.iter().any(|variant| !variant.fields.is_empty())
}

/// The flat struct field name for one field of one variant. Restated from
/// `backends::php::gen_bindings::types::enums::flat_field_name`. ~keep
fn flat_field_name(variant: &EnumVariant, field_index: usize) -> String {
    if crate::codegen::conversions::is_tuple_variant(&variant.fields) {
        let base = crate::codegen::naming::pascal_to_snake(&variant.name);
        if variant.fields.len() == 1 {
            base
        } else {
            format!("{base}_{field_index}")
        }
    } else {
        variant.fields[field_index].name.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use super::PhpEnumLowering;
    use crate::core::config::e2e::CallConfig;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
    use crate::e2e::codegen::field_skip::{FieldSkip, SkipClass};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn named(name: &str) -> TypeRef {
        TypeRef::Named(name.to_string())
    }

    /// A neutral fixture crate carrying the three enum lowerings the PHP backend distinguishes:
    ///
    /// * `EncodingDetails` — internally tagged with data-carrying variants, lowered to a flat
    ///   `#[php_class]` whose variant payloads are readable PHP properties (`->spreadsheet`).
    /// * `DocumentKind` — unit variants only, lowered to a PHP `string`.
    /// * `Payload` — `#[serde(untagged)]` with data, bridged as a JSON `string`.
    fn ir() -> (Vec<TypeDef>, Vec<EnumDef>) {
        let type_defs = vec![
            TypeDef {
                name: "DocumentResult".to_string(),
                fields: vec![field("metadata", named("DocumentMetadata"))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DocumentMetadata".to_string(),
                fields: vec![
                    field("encoding", named("EncodingDetails")),
                    field("kind", named("DocumentKind")),
                    field("payload", named("Payload")),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "SpreadsheetDetails".to_string(),
                fields: vec![field("sheet_count", TypeRef::Primitive(PrimitiveType::U32))],
                ..TypeDef::default()
            },
        ];
        let enums = vec![
            EnumDef {
                name: "EncodingDetails".to_string(),
                serde_tag: Some("type".to_string()),
                variants: vec![
                    EnumVariant {
                        name: "Spreadsheet".to_string(),
                        is_tuple: true,
                        fields: vec![field("_0", named("SpreadsheetDetails"))],
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "FictionBook".to_string(),
                        is_tuple: true,
                        fields: vec![field("_0", named("SpreadsheetDetails"))],
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DocumentKind".to_string(),
                variants: vec![EnumVariant {
                    name: "Report".to_string(),
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "Payload".to_string(),
                serde_untagged: true,
                variants: vec![EnumVariant {
                    name: "Text".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", TypeRef::String)],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
        ];
        (type_defs, enums)
    }

    fn render(field_path: &str) -> String {
        let (type_defs, enums) = ir();
        let result_fields: HashSet<String> = ["metadata".to_string()].into_iter().collect();
        let lowering = PhpEnumLowering::from_enums(&enums);
        let getter_map =
            super::super::types::build_php_getter_map(&type_defs, &enums, &CallConfig::default(), &result_fields);
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let resolver = FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            getter_map.clone(),
        )
        .with_ir_fields(reachable, excluded, optional);
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field_path.to_string()),
            value: Some(serde_json::json!(3)),
            ..Assertion::default()
        };
        let mut out = String::new();
        super::super::assertions::render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            false,
            &BTreeMap::new(),
            false,
            &super::PhpVariantAccess::new(&getter_map, &lowering),
        );
        out
    }

    /// The `project-agnostic-codegen` defect: the guard keyed off the literal consumer path
    /// `metadata.format.`, so an identically shaped path in any other crate was not recognised as
    /// an enum traversal at all and rendered a property chain against a PHP `string`.
    ///
    /// It must be a `LanguageLimitation`, not the generic `not available on result type` marker:
    /// that wording is an `AuthoringGap`, which the strict gate treats as consumer-fixable drift
    /// and fails the run on — but no `alef.toml` edit grows a variant accessor on a PHP `string`.
    #[test]
    fn a_variant_path_through_a_string_lowered_enum_is_skipped_whatever_the_field_is_named() {
        let out = render("metadata.kind.report");
        assert_eq!(
            FieldSkip::extract_classified(&out).map(|(field, skip)| (field, skip.class())),
            Some(("metadata.kind.report", SkipClass::LanguageLimitation)),
            "got: {out}"
        );
    }

    /// The untagged-data sibling: bridged to a JSON `string` getter, so there is no per-variant
    /// accessor either.
    #[test]
    fn a_variant_path_through_a_json_lowered_enum_is_skipped() {
        let out = render("metadata.payload.text");
        assert_eq!(
            FieldSkip::extract_classified(&out).map(|(field, skip)| (field, skip.class())),
            Some(("metadata.payload.text", SkipClass::LanguageLimitation)),
            "got: {out}"
        );
    }

    /// For an internally tagged data enum the PHP backend emits a flat `#[php_class]` with
    /// `#[php(getter)] pub fn get_spreadsheet(&self) -> Option<SpreadsheetDetails>`, which
    /// ext-php-rs registers as the read-only PHP property `->spreadsheet`. The variant payload is
    /// reachable, so no skip is defensible.
    ///
    /// The emitted chain must call `getEncoding()`: a data enum is NOT `#[php(prop)]`-scalar in
    /// the binding, so the field is exposed through a getter method, not a property. The e2e
    /// getter map used to be built from ALL enum names — including the data enums the binding
    /// excludes — and so emitted `->encoding`, a property that does not exist.
    #[test]
    fn a_variant_path_through_a_flat_class_enum_asserts_instead_of_skipping() {
        let out = render("metadata.encoding.spreadsheet.sheet_count");
        assert_eq!(FieldSkip::extract_classified(&out), None, "got: {out}");
        assert!(
            out.contains("$result->getMetadata()->getEncoding()->spreadsheet->sheetCount"),
            "got: {out}"
        );
    }

    /// The flat class really does expose `fiction_book` — but ext-php-rs registers it under that
    /// RAW snake_case ident while the shared accessor renderer lowerCamelCases every path segment,
    /// so the only chain it can emit is `->fictionBook`, a property that does not exist. Refusing
    /// is the honest answer until `field_access` learns the difference; emitting it would be a
    /// green assertion against nothing, which is the exact defect this funnel exists to stop.
    ///
    /// It is alef's own gap, not PHP's and not the fixture's, so it must be a `GeneratorGap` —
    /// a `LanguageLimitation` would misattribute it and an `AuthoringGap` would fail a consumer's
    /// build for something no config of theirs can change.
    #[test]
    fn a_multi_word_flat_property_is_refused_as_a_generator_gap_not_emitted_camel_cased() {
        let out = render("metadata.encoding.fiction_book.sheet_count");
        assert_eq!(
            FieldSkip::extract_classified(&out).map(|(field, skip)| (field, skip.class())),
            Some(("metadata.encoding.fiction_book.sheet_count", SkipClass::GeneratorGap)),
            "got: {out}"
        );
        assert!(!out.contains("fictionBook"), "got: {out}");
    }

    /// A path that stops AT the enum is not a variant accessor and must keep rendering: a
    /// string-lowered enum is a real `#[php(prop)]` property holding the variant's wire name.
    #[test]
    fn a_path_ending_on_the_enum_itself_is_not_skipped() {
        let out = render("metadata.kind");
        assert_eq!(FieldSkip::extract_classified(&out), None, "got: {out}");
        assert!(out.contains("$result->getMetadata()->kind"), "got: {out}");
    }

    /// The partition must reproduce the binding backend's three-way split, since it is what
    /// decides both the skip verdict and whether a field is a `#[php(prop)]` scalar.
    #[test]
    fn should_match_the_binding_backends_partition() {
        let (_, enums) = ir();
        let lowering = PhpEnumLowering::from_enums(&enums);
        let scalars = lowering.php_prop_scalar_enum_names();
        assert!(
            scalars.contains("DocumentKind"),
            "unit-variant enums map to a PHP string"
        );
        assert!(
            !scalars.contains("EncodingDetails"),
            "a tagged data enum is a flat class, not a #[php(prop)] scalar"
        );
        assert!(
            !scalars.contains("Payload"),
            "an untagged data enum is bridged as JSON, not a #[php(prop)] scalar"
        );
        assert!(lowering.flat_class_properties(&enums[1]).is_none());
        let properties = lowering.flat_class_properties(&enums[0]).expect("flat class");
        let names: Vec<&str> = properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["spreadsheet", "fiction_book", "type_tag"]);
    }
}
