use super::super::optional_renderers::{
    render_csharp_with_optionals, render_dart_with_optionals, render_java_with_optionals,
    render_kotlin_android_with_optionals, render_kotlin_with_optionals, render_php_with_getters,
    render_rust_with_optionals, render_typescript_with_optionals, render_zig_with_optionals,
};
use super::super::parse::parse_path;
use super::super::renderers::{render_accessor, render_swift_with_first_class_map};
use super::super::types::{FieldResolver, PathSegment};

impl FieldResolver {
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
            "zig" => render_zig_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
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
