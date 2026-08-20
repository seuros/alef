use super::super::ir_enum::build_ir_enum_map;
use super::super::types::{DartFirstClassMap, FieldResolver, IrEnumMap, PhpGetterMap, SwiftFirstClassMap};
use std::collections::{HashMap, HashSet};

impl FieldResolver {
    /// Create a new resolver from the e2e config's `fields` aliases,
    /// `fields_optional` set, `result_fields` set, `fields_array` set,
    /// and `fields_method_calls` set.
    pub fn new(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: HashMap::new(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
        }
    }

    /// Create a new resolver that also includes error-path field aliases.
    ///
    /// `error_field_aliases` maps fixture sub-field names (the part after `"error."`)
    /// to the actual field names on the error type, enabling `accessor_for_error` to
    /// resolve fields like `"status_code"` against the error value.
    pub fn new_with_error_aliases(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
        }
    }

    /// Create a new resolver that also knows which PHP fields need getter-method syntax.
    ///
    /// `php_getter_map` carries a per-`(type_name, field_name)` classification: the PHP
    /// accessor renderer emits `->getCamelCase()` when `(owner_type, field)` is
    /// recorded as needing a getter, and `->camelCase` property syntax otherwise.
    /// This matches the ext-php-rs 0.15.x behaviour where `#[php(getter)]` is used for
    /// non-scalar fields (Named structs, `Vec<Named>`, Map, etc.) while `#[php(prop)]` is
    /// used for scalar-compatible fields.
    ///
    /// Keying by (type, field) — not bare field name — is essential because the same
    /// field name can have different scalarness on different types. The map also carries
    /// per-type field→nested-type mappings so the renderer can walk a path like
    /// `outer.inner.content` through the IR, advancing the current-type cursor at each
    /// segment.
    pub fn new_with_php_getters(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        php_getter_map: PhpGetterMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map,
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
        }
    }

    /// Return a clone of this resolver with the Swift first-class map's
    /// `root_type` replaced.
    ///
    /// Used by Swift e2e codegen to thread a per-fixture (per-call) root type
    /// into the `render_swift_with_first_class_map` dispatcher. Each fixture's
    /// call returns a different IR type (e.g. `ChatCompletionResponse` vs
    /// `FileObject`), and the first-class/opaque classification of the root
    /// drives whether path segments are emitted with property access or
    /// method-call access. Setting it per-fixture avoids picking a single
    /// workspace-wide default that breaks half the fixtures.
    pub fn with_swift_root_type(&self, root_type: Option<String>) -> Self {
        let mut clone = self.clone();
        clone.swift_first_class_map.root_type = root_type;
        clone
    }

    /// Create a new resolver that also knows the Swift first-class/opaque
    /// classification per IR type. Mirrors `new_with_php_getters` but for the
    /// Swift `render_swift_with_first_class_map` path.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_swift_first_class(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        swift_first_class_map: SwiftFirstClassMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map,
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
        }
    }

    /// Create a new resolver that also knows the Dart stringy field
    /// classification per IR type (for aggregating text accessors in contains
    /// assertions on `Vec<T>` fields).
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_dart_first_class(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        dart_first_class_map: DartFirstClassMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map,
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
        }
    }

    /// Return a clone of this resolver with the Dart first-class map's
    /// `root_type` replaced.
    pub fn with_dart_root_type(&self, root_type: Option<String>) -> Self {
        let mut clone = self.clone();
        clone.dart_first_class_map.root_type = root_type;
        clone
    }

    /// Return a clone of this resolver with `display_as_text_fields` set.
    ///
    /// Fields in this set have an `Option<T>` inner type (e.g. `RichTextContent`)
    /// that is NOT a plain `String`. Language generators will call the language-idiomatic
    /// text accessor (`.Text()` in Go/Java/C#, `.text()` in PHP) instead of generic
    /// object stringification (`string(*ptr)`, `Objects::toString()`, `.ToString()`).
    pub fn with_display_as_text_fields(mut self, fields: HashSet<String>) -> Self {
        self.display_as_text_fields = fields;
        self
    }

    pub fn with_enum_fields(mut self, fields: HashSet<String>) -> Self {
        self.enum_fields = fields;
        self
    }

    /// Compute the IR-derived enum-field classification for [`Self::with_ir_enum_map`],
    /// mirroring [`Self::ir_field_sets`]'s "compute once from the crate's IR" shape. The
    /// returned map has no `root_type` set yet — `with_ir_enum_map` anchors it to the
    /// specific call being rendered.
    pub fn ir_enum_fields(type_defs: &[crate::core::ir::TypeDef], enums: &[crate::core::ir::EnumDef]) -> IrEnumMap {
        build_ir_enum_map(type_defs, enums)
    }

    /// Attach IR-derived enum classification to this resolver, anchored at `root_type` — the
    /// IR type name backing the current call's result variable, if resolved (e.g. via the
    /// call's declared Rust return type, unwrapped through `Option`/`Vec`).
    ///
    /// `map` should come from [`Self::ir_enum_fields`], computed once per crate IR and reused
    /// across calls; only `root_type` varies per call. `is_enum` consults this AFTER the
    /// hand-maintained `fields_enum` config, so an explicit config entry always wins and this
    /// only rescues fields the config never mentioned — the same precedence `with_ir_fields`
    /// already established for `result_fields`. ~keep
    pub fn with_ir_enum_map(mut self, mut map: IrEnumMap, root_type: Option<String>) -> Self {
        map.root_type = root_type;
        self.ir_enum_map = map;
        self
    }

    /// Return a clone of this resolver with IR-derived field-reachability data set.
    ///
    /// `reachable`/`excluded` come from [`Self::ir_field_sets`]. Once set, they become
    /// the primary source of truth for [`Self::is_valid_for_result`]: the hand-maintained
    /// `result_fields` config only gets the final say on field names the IR has never
    /// heard of (virtual namespace prefixes, synthetic/derived assertion fields, and the
    /// like) — see that method's doc comment for why config alone cannot be trusted.
    ///
    /// `optional` (also from [`Self::ir_field_sets`]) is merged into the config-declared
    /// `fields_optional` set rather than replacing it, so an `Option<T>` field is detected
    /// even when a consumer's `alef.toml` never lists it under `fields_optional` at all —
    /// see [`Self::ir_field_sets`] for why this merge is safe to do unconditionally. ~keep
    pub fn with_ir_fields(
        mut self,
        reachable: HashSet<String>,
        excluded: HashSet<String>,
        optional: HashSet<String>,
    ) -> Self {
        self.ir_reachable_fields = reachable;
        self.ir_known_excluded_fields = excluded;
        self.optional_fields.extend(optional);
        self.warn_on_result_fields_contradicting_ir();
        self
    }

    /// Emit a `WARN` event for every `result_fields` entry the IR marks
    /// `binding_excluded` — every case where `is_valid_for_result` now rejects the field
    /// despite the config claiming it's available.
    ///
    /// `result_fields` is meant to *select* which available fields a call asserts on, not
    /// to *declare* availability (the IR does that); an entry landing here is always a
    /// config bug, not a legitimate declaration. This must be loud, not silent — a
    /// shipped config was found with exactly this shape (a `#[serde(skip)]`, no-getter
    /// field still listed in `result_fields`) sitting undetected because nothing surfaced
    /// the contradiction. ~keep
    fn warn_on_result_fields_contradicting_ir(&self) {
        for field in &self.result_fields {
            if self.ir_known_excluded_fields.contains(field) {
                tracing::warn!(
                    field = %field,
                    "e2e config result_fields lists a field the IR marks binding_excluded (no \
                     accessor is emitted in any generated binding); the IR now overrides \
                     result_fields for this field and it will be treated as unavailable — fix \
                     or remove this result_fields entry"
                );
            }
        }
    }

    /// Compute the reachable/excluded/optional field-name sets from a crate's IR type
    /// definitions, for use with [`Self::with_ir_fields`].
    ///
    /// A field name is "reachable" if it is present, and not `binding_excluded`, on ANY
    /// type in `type_defs` — the exact predicate `crate::codegen::shared::binding_fields`
    /// uses to decide which struct fields a backend (pyo3, napi, go, …) actually attaches
    /// a real accessor to (e.g. `#[pyo3(get)]`). A field name is "known excluded" if it
    /// appears on some type but IS `binding_excluded` there, and is not reachable on any
    /// other type — reachable-on-any-type wins, since a bare field name can't be pinned to
    /// one exact result type here (callers only reach for this data when they can't
    /// already do that resolution themselves). ~keep
    ///
    /// A field name is "optional" only when EVERY declaration of it across `type_defs` is
    /// `Option<T>` (unanimous, not "any type wins" like `reachable`/`excluded` above). The
    /// direction has to flip here: `optional_fields` membership changes what code an
    /// accessor emits (`.as_ref().unwrap()` in Rust, `!` in C#, …), so a false positive is a
    /// compile error in a caller's generated test, while a false negative merely reproduces
    /// today's behavior (the field falls back to requiring an explicit `fields_optional`
    /// entry, exactly as before this method existed). ~keep
    pub fn ir_field_sets(
        type_defs: &[crate::core::ir::TypeDef],
    ) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
        let mut reachable = HashSet::new();
        let mut excluded = HashSet::new();
        // Name -> (seen as Option<T> somewhere, seen as non-Option somewhere).
        let mut optionality: HashMap<String, (bool, bool)> = HashMap::new();
        for type_def in type_defs {
            for field in &type_def.fields {
                if field.binding_excluded {
                    excluded.insert(field.name.clone());
                } else {
                    reachable.insert(field.name.clone());
                }
                let seen = optionality.entry(field.name.clone()).or_insert((false, false));
                if field.optional {
                    seen.0 = true;
                } else {
                    seen.1 = true;
                }
            }
        }
        excluded.retain(|f| !reachable.contains(f));
        let optional = optionality
            .into_iter()
            .filter_map(|(name, (seen_optional, seen_required))| (seen_optional && !seen_required).then_some(name))
            .collect();
        (reachable, excluded, optional)
    }
}
