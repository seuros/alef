use super::optional_renderers::{
    render_csharp_with_optionals, render_dart_with_optionals, render_java_with_optionals,
    render_kotlin_android_with_optionals, render_kotlin_with_optionals, render_php_with_getters,
    render_rust_with_optionals, render_typescript_with_optionals, render_zig_with_optionals,
};
use super::parse::{normalize_indices_to_wildcards, normalize_numeric_indices, parse_path, strip_numeric_indices};
use super::renderers::{render_accessor, render_swift_with_first_class_map};
use super::types::{DartFirstClassMap, FieldResolver, PathSegment, PhpGetterMap, StringyField, SwiftFirstClassMap};
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

    /// Returns `true` when `fixture_field` (or its resolved alias, or a
    /// normalised form) is configured as a display-as-text field.
    ///
    /// Accepts both the raw fixture field path and the alias-resolved path so
    /// callers don't need to resolve first.
    pub fn is_display_as_text(&self, fixture_field: &str) -> bool {
        if self.display_as_text_fields.is_empty() {
            return false;
        }
        if self.display_as_text_fields.contains(fixture_field) {
            return true;
        }
        let resolved = self.resolve(fixture_field);
        self.display_as_text_fields.contains(resolved)
    }

    /// Resolve a fixture field path to the actual struct path.
    /// Falls back to the field itself if no alias exists.
    pub fn resolve<'a>(&'a self, fixture_field: &'a str) -> &'a str {
        self.aliases
            .get(fixture_field)
            .map(String::as_str)
            .unwrap_or(fixture_field)
    }

    /// True when the leaf segment of `field` is a `Vec<T>` field on any IR type.
    ///
    /// Used by swift codegen to keep `.count` straight on method-call accessors
    /// (`result.output()` returns RustVec — `.count` works directly, no
    /// `.toString()` needed). The check is on the bare leaf name, so it is best-
    /// effort when distinct types share a field name with different kinds.
    pub fn leaf_is_vec_via_swift_map(&self, field: &str) -> bool {
        let leaf = field.split('.').next_back().unwrap_or(field);
        let leaf = leaf.split('[').next().unwrap_or(leaf);
        self.swift_first_class_map.is_vec_field_name(leaf)
    }

    /// IR type backing the Swift result variable, if known. Used by
    /// `swift_build_accessor` to seed its per-segment type cursor.
    pub fn swift_root_type(&self) -> Option<&String> {
        self.swift_first_class_map.root_type.as_ref()
    }

    /// Whether fields on `type_name` should be accessed as Swift properties
    /// (first-class Codable struct → `public let`) vs swift-bridge method calls
    /// (typealias-to-opaque RustBridge class). Mirrors `SwiftFirstClassMap::is_first_class`.
    pub fn swift_is_first_class(&self, type_name: Option<&str>) -> bool {
        self.swift_first_class_map.is_first_class(type_name)
    }

    /// Advance the per-segment type cursor by one field name. Mirrors
    /// `SwiftFirstClassMap::advance`.
    pub fn swift_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.swift_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Swift
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn swift_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.swift_first_class_map.stringy_fields(type_name)
    }

    /// IR type backing the Dart result variable, if known.
    pub fn dart_root_type(&self) -> Option<&String> {
        self.dart_first_class_map.root_type.as_ref()
    }

    /// Advance the Dart type cursor through a field, returning the target type name.
    pub fn dart_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.dart_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Dart
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn dart_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.dart_first_class_map.stringy_fields(type_name)
    }

    /// Check if a resolved field path is optional.
    pub fn is_optional(&self, field: &str) -> bool {
        if self.is_optional_direct(field) {
            return true;
        }
        // Namespace-prefix fallback: paths like `interaction.action_results[0].data`
        // strip the virtual `interaction.` prefix before consulting `optional_fields`,
        // matching the same convention used by `is_valid_for_result`.
        if let Some(suffix) = self.namespace_stripped_path(field)
            && self.is_optional_direct(suffix)
        {
            return true;
        }
        false
    }

    fn is_optional_direct(&self, field: &str) -> bool {
        if self.optional_fields.contains(field) {
            return true;
        }
        let index_normalized = normalize_numeric_indices(field);
        if index_normalized != field && self.optional_fields.contains(index_normalized.as_str()) {
            return true;
        }
        // Also check with all numeric indices stripped: "choices[0].message.tool_calls"
        // should match optional_fields entry "choices.message.tool_calls".
        let de_indexed = strip_numeric_indices(field);
        if de_indexed != field && self.optional_fields.contains(de_indexed.as_str()) {
            return true;
        }
        let normalized = field.replace("[].", ".");
        if normalized != field && self.optional_fields.contains(normalized.as_str()) {
            return true;
        }
        for af in &self.array_fields {
            if let Some(rest) = field.strip_prefix(af.as_str())
                && let Some(rest) = rest.strip_prefix('.')
            {
                let with_bracket = format!("{af}[].{rest}");
                if self.optional_fields.contains(with_bracket.as_str()) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a fixture field has an explicit alias mapping.
    pub fn has_alias(&self, fixture_field: &str) -> bool {
        self.aliases.contains_key(fixture_field)
    }

    /// Check whether `field_name` is configured as an explicit result field.
    ///
    /// Returns true only when the caller has populated `result_fields` AND the
    /// field name is present. Empty `result_fields` always returns false — use
    /// `is_valid_for_result` for the default-allow semantics.
    pub fn has_explicit_field(&self, field_name: &str) -> bool {
        if self.result_fields.is_empty() {
            return false;
        }
        self.result_fields.contains(field_name)
    }

    /// Check whether a fixture field path is valid for the configured result type.
    ///
    /// The IR is authoritative whenever it recognizes the resolved path's first segment
    /// as a real struct field name (populated via [`Self::with_ir_fields`]):
    /// reachable-through-the-binding wins regardless of `result_fields`, and
    /// known-excluded-from-the-binding loses regardless of `result_fields`. `result_fields`
    /// is a hand-maintained allowlist with no automatic connection to the real struct, and
    /// it can drift in BOTH directions at once — one shipped config was found with a field
    /// genuinely exposed via a real getter missing from `result_fields` (silently
    /// downgrading every assertion on it to a "not available" comment) *and*, in the same
    /// list, a field that carries `#[serde(skip)]` with no getter still listed as
    /// available (which would generate a passing-looking assertion against an attribute
    /// that doesn't exist at runtime). Neither direction is fixable by trusting
    /// `result_fields` harder or consulting more hand-maintained config — the IR is the
    /// only signal here that isn't itself hand-maintained per fixture. ~keep
    ///
    /// When the IR has never heard of the first segment at all — a virtual namespace
    /// prefix like `"browser."`, a streaming/synthetic pseudo-field, or simply because the
    /// codegen call site hasn't wired IR data in via `with_ir_fields` — this falls back to
    /// the config-only check: the resolved path's first segment is in `result_fields`, or
    /// the path uses a single virtual namespace prefix (e.g. `"browser."`, `"interaction."`)
    /// whose second segment IS in `result_fields`, or (last resort, see
    /// [`Self::is_known_via_sibling_field_config`]) another per-field config map already
    /// references the field even though `result_fields` doesn't.
    pub fn is_valid_for_result(&self, fixture_field: &str) -> bool {
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);

        // IR oracle: only consulted for names the IR actually recognizes. A name it has
        // never seen (namespace prefixes, synthetic fields, or simply no IR data wired up)
        // falls through to the config-only checks below unaffected.
        if self.ir_reachable_fields.contains(first_segment) {
            return true;
        }
        if self.ir_known_excluded_fields.contains(first_segment) {
            return false;
        }

        if self.result_fields.is_empty() {
            return true;
        }
        if self.result_fields.contains(first_segment) {
            return true;
        }
        // Namespace-prefix fallback: if the first segment is NOT a known result field
        // but stripping it yields a path whose own first segment IS a known result
        // field, treat the path as valid.  This supports fixture field paths like
        // `"browser.browser_used"` where `"browser"` is a virtual grouping prefix
        // and the real field is `"browser_used"`.
        if let Some(suffix) = self.namespace_stripped_path(resolved) {
            let suffix_first = suffix.split('.').next().unwrap_or(suffix);
            let suffix_first = suffix_first.split('[').next().unwrap_or(suffix_first);
            if self.result_fields.contains(suffix_first) {
                return true;
            }
        }
        self.is_known_via_sibling_field_config(fixture_field, resolved)
    }

    /// True when `fixture_field` (or its alias-resolved path) is referenced by one of
    /// the other per-field config maps (`fields`, `fields_optional`, `fields_array`,
    /// `fields_method_calls`) even though it is absent from `result_fields`.
    ///
    /// Last-resort fallback for codegen call sites that haven't wired IR data in via
    /// `with_ir_fields` (`is_valid_for_result` only reaches this once the IR has had, and
    /// declined, the chance to answer). These maps only make sense to populate for a field
    /// that genuinely exists on the result type — an alias target, an optionality flag, an
    /// array marker, or a method-call accessor all require the config author to have
    /// looked at the real struct. A field that is truly unavailable (no getter generated
    /// for it at all) has nothing to configure here, so this check does not make
    /// unavailable fields pass — it only rescues fields the config demonstrably already
    /// knows about. ~keep
    fn is_known_via_sibling_field_config(&self, fixture_field: &str, resolved: &str) -> bool {
        self.aliases.contains_key(fixture_field)
            || self.is_optional_direct(resolved)
            || self.is_array(resolved)
            || self.method_calls.contains(resolved)
    }

    /// If `path`'s first dot-separated segment is NOT in `result_fields` and
    /// contains no `[…]` indexing (i.e. it looks like a pure namespace label),
    /// return the remainder of the path after that first segment.  Returns `None`
    /// when the first segment already matches a result field or when stripping it
    /// would leave an empty string.
    pub fn namespace_stripped_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        // When the consumer hasn't configured `result_fields`, there is no way
        // to tell a virtual namespace prefix (e.g. `interaction.action_results`)
        // from a real nested-struct field path (e.g. `metrics.total_lines`).
        // Defaulting to "strip" was lossy — every dotted field path was reduced
        // to its leaf segment, so backends (notably the C e2e codegen) emitted
        // accessors against the wrong parent type. Opt the stripping in only
        // when the consumer explicitly listed the top-level result fields.
        if self.result_fields.is_empty() {
            return None;
        }
        let dot_pos = path.find('.')?;
        let first = &path[..dot_pos];
        // Only strip if the first segment contains no brackets (i.e. is a bare
        // label, not an array access like `pages[0]`).
        if first.contains('[') {
            return None;
        }
        // Only strip if the first segment is NOT itself a known result field —
        // real fields should never be treated as namespace prefixes.
        if self.result_fields.contains(first) {
            return None;
        }
        let suffix = &path[dot_pos + 1..];
        if suffix.is_empty() { None } else { Some(suffix) }
    }

    /// Check if a resolved field is an array/Vec type.
    pub fn is_array(&self, field: &str) -> bool {
        self.array_fields.contains(field)
    }

    /// Check whether `field` (a raw or already-resolved fixture path) is
    /// configured as a `fields_json_scalar` entry — i.e. its Kotlin type is
    /// an untyped JSON scalar (`Any?`, from `Option<serde_json::Value>`)
    /// rather than `Option<String>`, so `.orEmpty()` is undefined on it.
    ///
    /// Consults `json_scalar_fields` (a per-call resolved set, not stored on
    /// the resolver) against every spelling `fields_optional`/`is_optional`
    /// already treats as interchangeable — bracket-wildcard (`a[].b`) and
    /// fully de-indexed (`a.b`) — and, mirroring `is_optional`'s namespace
    /// fallback, against the path with a virtual grouping prefix (e.g.
    /// `interaction.`) stripped. Fixture field paths like
    /// `interaction.action_results[0].data` resolve to the struct path
    /// `action_results[0].data` for accessor generation via
    /// `namespace_stripped_path`; the same stripped path must be consulted
    /// here so `fields_json_scalar` entries configured against the struct
    /// path (not the virtual fixture namespace) are honored.
    pub fn is_json_scalar(&self, field: &str, json_scalar_fields: &HashSet<String>) -> bool {
        if Self::matches_json_scalar_spelling(field, json_scalar_fields) {
            return true;
        }
        let resolved = self.resolve(field);
        if resolved != field && Self::matches_json_scalar_spelling(resolved, json_scalar_fields) {
            return true;
        }
        self.namespace_stripped_path(resolved)
            .is_some_and(|stripped| Self::matches_json_scalar_spelling(stripped, json_scalar_fields))
    }

    fn matches_json_scalar_spelling(path: &str, json_scalar_fields: &HashSet<String>) -> bool {
        if json_scalar_fields.contains(path) {
            return true;
        }
        let normalized = normalize_indices_to_wildcards(path);
        if normalized != path && json_scalar_fields.contains(normalized.as_str()) {
            return true;
        }
        let de_indexed = strip_numeric_indices(path);
        de_indexed != path && json_scalar_fields.contains(de_indexed.as_str())
    }

    pub fn is_enum(&self, field: &str) -> bool {
        self.enum_fields.contains(field) || self.enum_fields.contains(self.resolve(field))
    }

    /// Check if a field name is the root of a collection type (i.e., the field
    /// itself returns a `Vec`/array, even though it is not in `fields_array`
    /// directly).
    ///
    /// `fields_array` tracks traversal paths like `choices[0].message.tool_calls`
    /// — the array element paths — not the bare collection accessor (`choices`).
    /// `fields_optional` may also contain paths like `data[0].url` that reveal
    /// `data` is a collection root.
    ///
    /// Returns `true` when any entry in `array_fields` or `optional_fields`
    /// starts with `{field}[`, indicating that `field` is the top-level
    /// collection getter.
    pub fn is_collection_root(&self, field: &str) -> bool {
        let prefix = format!("{field}[");
        self.array_fields.iter().any(|af| af.starts_with(&prefix))
            || self.optional_fields.iter().any(|of| of.starts_with(&prefix))
    }

    /// Check if a resolved field path traverses a tagged-union variant.
    ///
    /// Returns `Some((prefix, variant, suffix))` where:
    /// - `prefix` is the path up to (but not including) the tagged-union field
    ///   (e.g., `"metadata.format"`)
    /// - `variant` is the tagged-union accessor segment
    ///   (e.g., `"excel"`)
    /// - `suffix` is the remaining path after the variant
    ///   (e.g., `"sheet_count"`)
    ///
    /// Returns `None` if no tagged-union segment exists in the path.
    pub fn tagged_union_split(&self, fixture_field: &str) -> Option<(String, String, String)> {
        let resolved = self.resolve(fixture_field);
        let segments: Vec<&str> = resolved.split('.').collect();
        let mut path_so_far = String::new();
        for (i, seg) in segments.iter().enumerate() {
            if !path_so_far.is_empty() {
                path_so_far.push('.');
            }
            path_so_far.push_str(seg);
            if self.method_calls.contains(&path_so_far) {
                // Everything before the last segment of path_so_far is the prefix.
                let prefix = segments[..i].join(".");
                let variant = (*seg).to_string();
                let suffix = segments[i + 1..].join(".");
                return Some((prefix, variant, suffix));
            }
        }
        None
    }

    /// Split a bracket-wildcard path (`foo[].bar`) into its array-root path and
    /// element sub-path, or `None` when the path has no wildcard.
    ///
    /// A wildcard means "every element", so callers render an any-element
    /// construct over the array root rather than an accessor into one index.
    /// Build the element side with `accessor(&element, lang, "<lambda param>")`
    /// — passing the closure parameter as the result var is what lets a nested
    /// element sub-path resolve against the loop variable instead of the result.
    ///
    /// Alias resolution happens BEFORE the split, so a renamed sub-field lands on
    /// the element side; the raw split is only a fallback for when resolution drops
    /// the marker. Explicit numeric indices (`choices[0].message`) return `None` and
    /// keep their existing index-preserving path through `accessor`. ~keep
    ///
    /// The split is NOT recursive: it consumes the FIRST `[].` only. A doubly-nested path
    /// (`pages[].links[].url`) therefore returns an element sub-path that still carries a
    /// wildcard, and handing that to `accessor` lowers the inner `[]` to index 0 (see
    /// `parse_path`) — the caller's loop covers `pages` while the assertion inside it silently
    /// reads `links[0]`. Gate the element sub-path with
    /// `crate::e2e::codegen::field_skip::nested_wildcard_skip_line` before building an
    /// accessor from it. ~keep
    pub fn wildcard_split(&self, fixture_field: &str) -> Option<(String, String)> {
        let raw_dot = fixture_field.find("[].")?;
        let resolved = self.resolve(fixture_field);
        match resolved.find("[].") {
            Some(dot) => Some((resolved[..dot].to_string(), resolved[dot + 3..].to_string())),
            None => Some((
                fixture_field[..raw_dot].to_string(),
                fixture_field[raw_dot + 3..].to_string(),
            )),
        }
    }

    /// Check if a resolved field path contains a non-numeric map access.
    pub fn has_map_access(&self, fixture_field: &str) -> bool {
        let resolved = self.resolve(fixture_field);
        let segments = parse_path(resolved);
        segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        })
    }

    /// Generate a language-specific accessor expression.
    ///
    /// When `fixture_field` resolves to a path whose first segment is a virtual
    /// namespace prefix (not a real result field), the prefix is stripped before
    /// generating the accessor.  This matches the behaviour of `is_valid_for_result`
    /// so that paths like `"browser.browser_used"` produce `result.browser_used`
    /// (Python) / `result.BrowserUsed` (C#) / etc. rather than the raw
    /// `result.browser.browser_used` which would fail at runtime.
    pub fn accessor(&self, fixture_field: &str, language: &str, result_var: &str) -> String {
        let resolved = self.resolve(fixture_field);
        // Strip a leading namespace prefix when the first segment is not a known
        // result field but the remainder's first segment is.  This handles fixture
        // paths like `"browser.browser_used"` → actual accessor path `"browser_used"`.
        let effective = if !self.result_fields.is_empty() {
            if let Some(stripped) = self.namespace_stripped_path(resolved) {
                let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                if self.result_fields.contains(stripped_first) {
                    stripped
                } else {
                    resolved
                }
            } else {
                resolved
            }
        } else {
            resolved
        };
        let segments = parse_path(effective);
        let segments = self.inject_array_indexing(segments);
        match language {
            "typescript" | "node" => render_typescript_with_optionals(&segments, result_var, &self.optional_fields),
            "java" => render_java_with_optionals(&segments, result_var, &self.optional_fields),
            "kotlin" => render_kotlin_with_optionals(&segments, result_var, &self.optional_fields),
            // kotlin_android data classes expose fields as Kotlin properties (no parens),
            // not as Java-style getter methods. Use the dedicated renderer.
            "kotlin_android" => render_kotlin_android_with_optionals(&segments, result_var, &self.optional_fields),
            "rust" => render_rust_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
            "csharp" => render_csharp_with_optionals(&segments, result_var, &self.optional_fields),
            "zig" => render_zig_with_optionals(&segments, result_var, &self.optional_fields, &self.method_calls),
            // Always use `render_swift_with_first_class_map` for Swift. The map
            // correctly handles both first-class (property syntax) and opaque
            // (method-call syntax) types. When no type info is available (empty map,
            // unknown root type), `is_first_class(None)` returns `false` so
            // method-call syntax is the safe default — opaque swift-bridge types
            // expose fields as methods, not properties.
            "swift" => render_swift_with_first_class_map(
                &segments,
                result_var,
                &self.optional_fields,
                &self.swift_first_class_map,
            ),
            "dart" => render_dart_with_optionals(&segments, result_var, &self.optional_fields),
            "php" if !self.php_getter_map.is_empty() => {
                render_php_with_getters(&segments, result_var, &self.php_getter_map, &self.optional_fields)
            }
            _ => render_accessor(&segments, language, result_var),
        }
    }

    /// Generate a language-specific accessor expression for an error-path field.
    ///
    /// Used when `assertion_type == "error"` and the fixture declares a `field`
    /// like `"error.status_code"`. The caller strips the `"error."` prefix and
    /// passes the sub-field name (e.g. `"status_code"`) here.
    ///
    /// Resolves against `error_field_aliases` (instead of the success-path
    /// `aliases`). Falls back to direct field access (i.e. `err_var.status_code`)
    /// when no alias exists.
    ///
    /// For Rust, uses `render_rust_with_optionals` so that fields in
    /// `method_calls` emit parentheses (e.g. `err.status_code()` when
    /// `"status_code"` is in `fields_method_calls`).
    pub fn accessor_for_error(&self, sub_field: &str, language: &str, err_var: &str) -> String {
        let resolved = self
            .error_field_aliases
            .get(sub_field)
            .map(String::as_str)
            .unwrap_or(sub_field);
        let segments = parse_path(resolved);
        // Error fields are simple scalar fields — no array injection needed.
        // For Rust, delegate to render_rust_with_optionals so method_calls are honoured.
        match language {
            "rust" => render_rust_with_optionals(
                &segments,
                err_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
            _ => render_accessor(&segments, language, err_var),
        }
    }

    /// Check whether a sub-field (the part after `"error."`) has an entry in
    /// `error_field_aliases` or if there are any error aliases at all.
    ///
    /// When there are no error aliases configured, callers fall back to
    /// direct field access, which is the safe default for known public fields
    /// like `status_code` on `SampleLlmError`.
    pub fn has_error_aliases(&self) -> bool {
        !self.error_field_aliases.is_empty()
    }

    fn inject_array_indexing(&self, segments: Vec<PathSegment>) -> Vec<PathSegment> {
        if self.array_fields.is_empty() {
            return segments;
        }
        let len = segments.len();
        let mut result = Vec::with_capacity(len);
        let mut path_so_far = String::new();
        for i in 0..len {
            let seg = &segments[i];
            match seg {
                PathSegment::Field(f) => {
                    if !path_so_far.is_empty() {
                        path_so_far.push('.');
                    }
                    path_so_far.push_str(f);
                    let next_is_length = i + 1 < len && matches!(segments[i + 1], PathSegment::Length);
                    if i + 1 < len && self.array_fields.contains(&path_so_far) && !next_is_length {
                        // Config-registered array field without explicit index — default to 0.
                        result.push(PathSegment::ArrayField {
                            name: f.clone(),
                            index: 0,
                        });
                    } else {
                        result.push(seg.clone());
                    }
                }
                // Explicit ArrayField from parse_path — pass through unchanged; the user's
                // explicit index takes precedence over any config default.
                PathSegment::ArrayField { .. } => {
                    result.push(seg.clone());
                }
                PathSegment::MapAccess { field, key } => {
                    if !path_so_far.is_empty() {
                        path_so_far.push('.');
                    }
                    path_so_far.push_str(field);
                    let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                    if is_numeric && self.array_fields.contains(&path_so_far) {
                        // Numeric map-access on a registered array field — upgrade to ArrayField.
                        let index: usize = key.parse().unwrap_or(0);
                        result.push(PathSegment::ArrayField {
                            name: field.clone(),
                            index,
                        });
                    } else {
                        result.push(seg.clone());
                    }
                }
                _ => {
                    result.push(seg.clone());
                }
            }
        }
        result
    }

    /// Generate a Rust variable binding that unwraps an Optional string field.
    pub fn rust_unwrap_binding(&self, fixture_field: &str, result_var: &str) -> Option<(String, String)> {
        let resolved = self.resolve(fixture_field);
        if !self.is_optional(resolved) {
            return None;
        }
        // Mirror the namespace-prefix stripping done in `accessor()` so paths
        // like `"interaction.action_results[0].data"` resolve against the real
        // result type (`InteractionResult`) rather than the literal namespace.
        let effective = if !self.result_fields.is_empty() {
            if let Some(stripped) = self.namespace_stripped_path(resolved) {
                let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                if self.result_fields.contains(stripped_first) {
                    stripped
                } else {
                    resolved
                }
            } else {
                resolved
            }
        } else {
            resolved
        };
        let segments = parse_path(effective);
        let segments = self.inject_array_indexing(segments);
        // Sanitize the resolved path into a snake_case Rust identifier:
        // 1. `.` and `[` become `_` separators, `]` is dropped.
        // 2. Collapse runs of `_` so `foo[].bar` → `foo__bar` → `foo_bar`
        //    and strip any leading/trailing underscores.
        let local_var = {
            let raw = effective.replace(['.', '['], "_").replace(']', "");
            let mut collapsed = String::with_capacity(raw.len());
            let mut prev_underscore = false;
            for ch in raw.chars() {
                if ch == '_' {
                    if !prev_underscore {
                        collapsed.push('_');
                    }
                    prev_underscore = true;
                } else {
                    collapsed.push(ch);
                    prev_underscore = false;
                }
            }
            // Prefix with `_` so the binding declaration suppresses `-D unused_variables`
            // when no assertion actually references the local.  The variable remains fully
            // accessible under the `_`-prefixed name if an assertion does use it.
            format!("_{}", collapsed.trim_matches('_'))
        };
        // Use the optional-aware Rust renderer so intermediate `Option<T>`
        // segments produce `.as_ref().unwrap()` instead of bare field access.
        // For e.g. `summary.strategy` with `summary` in `optional_fields`, the
        // basic `render_accessor` would emit `result.summary.strategy`, which
        // is a compile error because `Option<Summary>` has no `strategy` field.
        let accessor = render_rust_with_optionals(
            &segments,
            result_var,
            &self.optional_fields,
            &self.method_calls,
            &self.result_fields,
        );
        let has_map_access = segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        });
        let is_array = self.is_array(resolved);
        let binding = if has_map_access {
            format!("let {local_var} = {accessor}.unwrap_or(\"\");")
        } else if is_array {
            format!("let {local_var} = {accessor}.as_deref().unwrap_or(&[]);")
        } else {
            // Use Display (via `.to_string()`) so types that intentionally implement Display
            // with a serde-style representation (e.g. `FinishReason` rendering as
            // `"content_filter"`) match the wire-format strings asserted in fixtures.
            // Types without Display would need to be excluded from string-equals assertions
            // or have a Display impl added to the core library.
            format!("let {local_var} = {accessor}.as_ref().map(|v| v.to_string()).unwrap_or_default();")
        };
        Some((binding, local_var))
    }
}
