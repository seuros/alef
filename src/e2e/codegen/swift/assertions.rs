use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::accessors::{
    materialise_vec_temporaries, swift_array_contains_expr, swift_array_count_expr, swift_array_is_empty_expr,
    swift_array_not_empty_predicate, swift_count_target, swift_stringy_aggregator_contains_assert,
    swift_traversal_contains_assert,
};
use super::values::{escape_swift, json_to_swift, swift_numeric_literal_cast};

/// ~keep The token a skip marker names when the assertion has no field path at all (a bare-result
/// assertion). Every registered wording quotes a token, and a marker that quotes nothing matches
/// no shape — which is how `// skipped: field is a scalar String without meaningful .count` stayed
/// invisible to both funnels and to a grep census.
const BARE_RESULT_TOKEN: &str = "<bare result>";

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    result_is_option: bool,
    result_element_is_string: bool,
    result_field_accessor: &HashMap<String, String>,
    is_streaming: bool,
    returns_void: bool,
) {
    // When the bare result is `Optional<T>` (no field path) the opaque class
    // exposed by swift-bridge has no `.toString()` method, so the usual
    // `.toString().isEmpty` pattern produces compile errors. Detect the
    // "bare result" case and prefer `XCTAssertNil` / `XCTAssertNotNil`.
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    // Streaming virtual fields resolve against the `chunks` collected-array variable.
    // Intercept before is_valid_for_result so they are never skipped.
    // Also intercept `usage.*` deep-paths in streaming tests: `AsyncThrowingStream` does
    // not have a `usage()` method, so we must route them through the chunks accessor.
    if let Some(f) = &assertion.field {
        let is_streaming_usage_path =
            is_streaming && (f == "usage" || (f.starts_with("usage.") || f.starts_with("usage[")));
        // Only route through the streaming-virtual `chunks` accessor when this is
        // actually a streaming fixture. Non-streaming fixtures (e.g. `process()`
        // with `chunkMaxSize`) expose `chunks` as a real `ProcessResult` field, so
        // emit `result.chunks()` via the regular field-accessor path below.
        if is_streaming
            && !f.is_empty()
            && (crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) || is_streaming_usage_path)
        {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "swift", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThanOrEqual(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertEqual(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!("        XCTAssertEqual({expr}, \"{escaped}\")\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        XCTAssertEqual({expr}, {b})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "not_empty" => {
                        format!("        XCTAssertFalse({expr}.isEmpty, \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        XCTAssertTrue({expr}.isEmpty, \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        XCTAssertTrue({expr})\n")
                    }
                    "is_false" => {
                        format!("        XCTAssertFalse({expr})\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThan(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!(
                                "        XCTAssertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    _ => format!(
                        "{}\n",
                        streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type)
                    ),
                };
                out.push_str(&line);
            } else {
                // ~keep The accessor returns `None` for reachable inputs — a `stream.has_*_event`
                // predicate never resolves here, since `accessor` supplies no item type — and this
                // branch used to be absent, so the assertion vanished with no line for
                // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the
                // gap, so it is counted, never fatal.
                let _ = writeln!(
                    out,
                    "        // skipped: {}",
                    FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
                );
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }

    // Skip length/count assertions whose collection leaf is bridged to a scalar
    // `RustString` rather than a countable `RustVec`. swift-bridge JSON-bridges
    // `Option<Vec<T>>`, `Vec<Vec<_>>`, and `Map` getters to a single `RustString`,
    // which has no `.count` — so the naive `<collection>().count` the renderer
    // emits for a trailing `.length`/`.count`/`.size` segment does not compile.
    // The renderer cannot see the leaf's swift-bridge kind, so guard here and
    // skip, matching the go/csharp/java backends (which also skip these).
    //
    // ~keep This guard runs only after `is_valid_for_result` above accepted the path, so the field
    // IS resolvable, and `NotAvailableOnResultType` — an `AuthoringGap`, therefore fatal under the
    // strict gate — was the wrong wording for it: the backend dropped the assertion as an honest
    // ABI limit while the gate demanded the consumer repair a field path that was never wrong, two
    // verdicts about one fact with nothing comparing them. `CountOnJsonBridgedLeafInSwift` states
    // the real reason and carries the classification that reason implies.
    if let Some(f) = &assertion.field
        && let Some(collection) = ["length", "count", "size"]
            .iter()
            .find_map(|suffix| f.strip_suffix(&format!(".{suffix}")))
        && !collection.is_empty()
        && !field_resolver.leaf_is_vec_via_swift_map(field_resolver.resolve(collection))
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::CountOnJsonBridgedLeafInSwift.message(f)
        );
        return;
    }

    // Skip assertions that traverse a tagged-union variant boundary.
    // In Swift, FormatMetadata and similar enum-backed opaque types are exposed as
    // plain classes by swift-bridge — variant accessor methods (e.g., `.excel()`)
    // are not generated, so such assertions cannot be expressed.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && field_resolver.tagged_union_split(f).is_some()
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::CrossesTaggedUnionBoundaryInSwift.message(f)
        );
        return;
    }

    // A `foo[].bar` fixture path names EVERY element of `foo`, not element 0. The shared
    // accessor has no wildcard concept: `parse_path` lowers `foo[]` to
    // `PathSegment::ArrayField { index: 0 }`, so any arm reaching the generic accessor emits
    // `result.foo()[0].bar()` — an assertion about one element wearing the fixture's "some
    // element" wording. Route every wildcard path here first so an assertion type this
    // backend cannot traverse leaves a visible skip instead of a silent index-0 assertion,
    // matching the pre-dispatch every other backend already performs. ~keep
    if let Some(field) = assertion.field.as_deref()
        && let Some(dot) = field.find("[].")
    {
        render_wildcard_assertion(out, assertion, field, dot, result_var, field_resolver);
        return;
    }

    // Determine if this field is an enum type. `field_resolver.is_enum` consults the
    // hand-maintained `fields_enum`/`enum_fields` config first and only then the IR-derived
    // classification (`with_ir_enum_map`), so an explicit config entry still wins — this only
    // rescues fields a consumer's `alef.toml` never mentions at all. A config-only check here
    // used to answer `false` for those, emitting `XCTAssertEqual(result.kind().toString(),
    // "key_value")` against a field whose Swift type is the generated enum `DataNodeKind`,
    // which is not compile-comparable to a `String`. ~keep
    let field_is_enum = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_enum(f));

    // Determine if this field is a display-as-text content union (e.g. `AssistantContent`).
    // Such fields are emitted as Swift enums (not `String`) and expose a `.text()` method
    // that concatenates the plain-text representation. The assertion must call `.text()` to
    // compare against the fixture's expected string, mirroring the Kotlin/Go/Java backends.
    let field_is_display_as_text = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_display_as_text(f));

    let field_is_optional = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty() && (field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f)))
    });
    let field_is_array = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty()
            && (field_resolver.is_array(f)
                || field_resolver.is_array(field_resolver.resolve(f))
                || field_resolver.is_collection_root(f)
                || field_resolver.is_collection_root(field_resolver.resolve(f)))
    });

    let field_expr_raw = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "swift", result_var),
            _ => result_var.to_string(),
        }
    };

    // swift-bridge `RustVec<T>` exposes its elements as `T.SelfRef`, which holds
    // a raw pointer into the parent Vec's storage. When the Vec is a temporary
    // (e.g. `result.json_ld()` called inline), Swift ARC may release it before
    // the ref is used, leaving the ref's pointer dangling. Materialise the
    // temporary into a local so it survives the full expression chain.
    //
    // The local name is suffixed with the assertion type plus a hash of the
    // assertion's discriminating fields so multiple assertions on the same
    // collection don't redeclare the same name.
    let local_suffix = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        assertion.field.hash(&mut hasher);
        assertion
            .value
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut hasher);
        format!(
            "{}_{:x}",
            assertion.assertion_type.replace(['-', '.'], "_"),
            hasher.finish() & 0xffff_ffff,
        )
    };
    let (vec_setup, field_expr, is_map_subscript) = materialise_vec_temporaries(&field_expr_raw, &local_suffix);
    // Wildcard paths never reach here — they return via `render_wildcard_assertion` above —
    // so `field_expr` is always the expression the arms below assert on and its setup lines
    // are never dead. The previous suppression list named `is_empty`, which had no traversal
    // branch to suppress for: it dropped the `let _vec_… = …` binding while still emitting an
    // expression referencing that local, so `is_empty` on a wildcard path emitted Swift
    // naming an undeclared variable. ~keep
    for line in &vec_setup {
        let _ = writeln!(out, "        {line}");
    }

    // In Swift, optional chaining with `?.` makes the result optional even if the
    // called method's return type isn't marked optional. For example:
    // `result.markdown()?.content()` returns `Optional<RustString>` because
    // `markdown()` is optional and the `?.` operator wraps the result.
    // Detect this by checking if the accessor contains `?.`.
    let accessor_is_optional = field_expr.contains("?.");
    // First-class Codable Swift struct property access leaves no trailing `()`
    // on the leaf segment — e.g. `result.text` (Swift `String`) vs
    // `result.text()` (RustBridge.RustString). When the leaf is property
    // access, we already have a Swift `String` (or `String?`) and must NOT
    // re-wrap with `.toString()`. Detect this by looking at the final segment
    // after the last `.` — property access ends in a bare identifier (no
    // trailing `()` or `()?`).
    let leaf_is_property_access = {
        let trimmed = field_expr.trim_end_matches('?');
        // Skip subscripts: `name?[0]` should still see `name` as the field.
        let last_segment = trimmed.rsplit_once('.').map(|(_, s)| s).unwrap_or(trimmed);
        let last_segment = last_segment.split('[').next().unwrap_or(last_segment);
        !last_segment.ends_with(')') && !last_segment.is_empty()
    };

    // Bare-result Option<T> case: the call returns `Optional<String>` (or
    // similar) so the field_expr is `result` typed as `String?`. String
    // assertions like `XCTAssertEqual(result.trimmingCharacters(...), …)` will
    // not compile against an optional — coalesce to `""` so the macro sees a
    // concrete Swift `String`.
    let bare_result_is_simple_option =
        result_is_simple && result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();

    // For enum fields, need to handle the string representation differently in Swift.
    // Swift enums don't have `.rawValue` unless they're explicitly RawRepresentable.
    // Check if this is an enum type and handle accordingly.
    // For optional fields (Optional<RustString>), use optional chaining before toString().
    // For other fields: swift-bridge returns all Rust `String` fields as `RustString`.
    // We add .toString() here so string assertions (contains, hasPrefix, etc.) work.
    // Non-string opaque fields (DocumentStructure, etc.) should not appear in string
    // assertions — the fixture schema controls which assertions apply to which fields.
    let string_expr = if field_is_display_as_text {
        // Display-as-text content union (e.g. `AssistantContent`): the leaf is a Swift
        // enum exposing `.text()` returning a non-optional `String`. For optional content
        // (`AssistantContent?`) or an optional ancestor chain, unwrap with `?.text()` and
        // coalesce to "" so XCTAssert receives a concrete Swift `String`.
        if field_is_optional || accessor_is_optional {
            format!("({field_expr}?.text() ?? \"\")")
        } else {
            format!("{field_expr}.text()")
        }
    } else if is_map_subscript {
        // The field_expr already evaluates to `String?` (from a JSON-decoded
        // `[String: String]` subscript). No `.toString()` chain needed —
        // coalesce the optional to "" and use the Swift String directly.
        format!("({field_expr} ?? \"\")")
    } else if leaf_is_property_access {
        // First-class Codable struct field access: leaf is already a Swift
        // `String` (or `String?`/enum type) — never a `RustString` requiring
        // `.toString()`. For optional leaves, coalesce to "" so XCTAssert
        // receives a non-optional Swift `String`.
        if field_is_enum && (field_is_optional || accessor_is_optional) {
            // Optional first-class Codable enum (e.g. `FinishReason?` where
            // `FinishReason: String, Codable`). `.rawValue` gives the serde
            // wire value (e.g. "tool_calls") so assertions match fixture JSON.
            format!("(({field_expr})?.rawValue ?? \"\")")
        } else if field_is_enum {
            format!("{field_expr}.rawValue")
        } else if field_is_optional || accessor_is_optional || bare_result_is_simple_option {
            format!("({field_expr} ?? \"\")")
        } else {
            field_expr.to_string()
        }
    } else if field_is_enum && accessor_is_optional {
        // Enum-typed leaf reached through an ancestor optional chain. The chain's `?`
        // already propagated, so `field_expr` is `Optional<RustString>` even though
        // the leaf accessor itself is non-Optional. Use `.toString()` (no extra `?`)
        // to avoid Swift's "cannot use optional chaining on non-optional value" error.
        format!("({field_expr}.toString() ?? \"\")")
    } else if field_is_enum && field_is_optional {
        // Enum-typed field that is itself Optional<RustString> (e.g. `finish_reason()`
        // returning `Optional<RustString>` at the binding surface) — unwrap with `?`.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if field_is_enum {
        // Enum-typed fields are now bridged as `String` (RustString in Swift) rather than
        // as opaque enum handles. The getter on the Rust side calls `to_string()` internally
        // and returns a `String` across the FFI. In Swift this arrives as `RustString`, so
        // `.toString()` converts it to a Swift `String` — one call, not two.
        format!("{field_expr}.toString()")
    } else if accessor_is_optional {
        // Ancestor optional chain already propagated `?` (e.g. `result.summary()?.strategy()`),
        // so the whole `field_expr` is Optional<RustString> regardless of whether the leaf
        // field itself is also marked optional. Adding another `?` before `.toString()` here
        // would emit `result.summary()?.strategy()?.toString()` which Swift rejects:
        // "cannot use optional chaining on non-optional value of type 'RustString'".
        // The earlier `?` from the accessor's chain already unwraps; use `.toString()` here.
        format!("({field_expr}.toString() ?? \"\")")
    } else if field_is_optional {
        // Leaf field itself is Optional<RustString> with no ancestor chain — need
        // ?.toString() to unwrap before stringifying.
        format!("({field_expr}?.toString() ?? \"\")")
    } else {
        format!("{field_expr}.toString()")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "        XCTAssertEqual({string_expr}, {swift_val})");
                } else {
                    // For numeric fields, cast the expected value to match the field's type (e.g., UInt).
                    let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}, {cast_swift_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // When the root result IS the array (result_is_simple + result_is_array) and
                // there is no field path, check array membership via map+contains.
                let no_field = assertion.field.as_deref().is_none_or(|f| f.is_empty());
                if result_is_simple && result_is_array && no_field {
                    if result_element_is_string {
                        // The Swift binding exposes the result as a native
                        // `[String]` (e.g. `manifestLanguages() -> [String]`),
                        // not the opaque `RustVec<RustString>`. Iterating
                        // elements yields plain Swift `String`, which has no
                        // `asStr()` — emit a direct `.contains(...)` instead.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        // RustVec<RustString> iteration yields RustStringRef (no `toString()`);
                        // use `.asStr().toString()` to convert each element to a Swift String.
                        // swift-bridge renames `as_str` → `asStr` automatically.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.map {{ $0.asStr().toString() }}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                } else {
                    // For array fields (RustVec<RustString>), check membership via map+contains.
                    let field_is_array = assertion
                        .field
                        .as_deref()
                        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
                    if field_is_array {
                        // First try the "stringy aggregator" path: when the array element
                        // is an opaque DTO with several text-bearing accessors (e.g.
                        // ImportInfo with source/items/alias, or StructureItem with
                        // kind/name/signature/...), emit a `contains(where: { ... })`
                        // closure that walks every accessor and does substring matching,
                        // mirroring python's `_alef_e2e_item_texts`. This avoids the
                        // brittle "primary accessor" guess (e.g. ImportInfo → source
                        // misses imports whose name lives in `items`).
                        let aggregator = swift_stringy_aggregator_contains_assert(
                            assertion.field.as_deref(),
                            result_var,
                            field_resolver,
                            &swift_val,
                        );
                        if let Some(line) = aggregator {
                            let _ = writeln!(out, "{line}");
                        } else {
                            let (contains_expr, is_optional) = swift_array_contains_expr(
                                assertion.field.as_deref(),
                                result_var,
                                field_resolver,
                                result_field_accessor,
                                Some(&field_expr),
                            );
                            let wrapped = if is_optional {
                                format!("({contains_expr} ?? [])")
                            } else {
                                contains_expr
                            };
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else if field_is_enum {
                        // Enum fields: use `toString().toString()` (via string_expr) to get the
                        // serde variant name as a Swift String, then check substring containment.
                        // Swift's `String.contains("")` returns false; guard with `.isEmpty` so
                        // fixtures that assert containment of an empty string still pass.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        // Same `isEmpty` guard as the enum branch — every string trivially
                        // "contains" the empty string, but Swift's `String.contains` does not.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                if let Some(f) = assertion.field.as_deref() {
                    // For array fields (RustVec<RustString>), check membership via map+contains.
                    let field_is_array = field_resolver.is_array(field_resolver.resolve(f));
                    if field_is_array {
                        let (contains_expr, is_optional) = swift_array_contains_expr(
                            assertion.field.as_deref(),
                            result_var,
                            field_resolver,
                            result_field_accessor,
                            Some(&field_expr),
                        );
                        let wrapped = if is_optional {
                            format!("({contains_expr} ?? [])")
                        } else {
                            contains_expr
                        };
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else if field_is_enum {
                        // Enum fields: use `toString().toString()` (via string_expr) to get the
                        // serde variant name as a Swift String, then check substring containment.
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else {
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    }
                } else {
                    // No field — fall back to existing string_expr path.
                    for val in values {
                        let swift_val = json_to_swift(val);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                );
            }
        }
        "not_empty" => {
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For array fields (RustVec<T>), check .isEmpty on the vec directly.
            // For result_is_simple (e.g. Data, String), use .isEmpty directly on
            // the result — avoids calling .toString() on non-RustString types.
            // For string fields, convert to Swift String and check .isEmpty.
            if bare_result_is_option {
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.isEmpty, \"expected non-empty value\")"
                );
            } else if field_is_array && field_is_optional {
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => format!("{field_expr}?.isEmpty == false") },
                ));
            } else if field_is_optional {
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => format!("{field_expr} != nil") },
                ));
            } else if field_is_array {
                let predicate = swift_array_not_empty_predicate(&field_expr, accessor_is_optional);
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => predicate },
                ));
            } else if result_is_simple {
                // result_is_simple: result is a primitive (Data, String, etc.) — use .isEmpty directly.
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({result_var}.isEmpty, \"expected non-empty value\")"
                );
            } else {
                // First-class Swift struct fields are properties typed as native Swift
                // `String` / `[T]` / `Data` etc — all of which expose `.count` (and
                // `String`/`Array` also expose `.isEmpty`). Use `.count > 0` so the same
                // path works whether the field is a String or an Array.
                //
                // When the accessor contains a `?.` optional chain, `.count` returns an
                // Optional which Swift cannot compare directly to `0`; coalesce via `?? 0`
                // so the assertion typechecks.
                //
                // For opaque method-call accessors (`result.id()`), the returned type is
                // `RustString`, which lacks `.count`. Convert to Swift `String` first via
                // `.toString()`. Array fields short-circuit above via `field_is_array`, so
                // method-call accessors landing here are guaranteed to be the scalar /
                // string flavour; vec accessors return `RustVec` (whose `.count` is fine).
                if let Some(count_target) = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref())
                {
                    let len_expr = if accessor_is_optional {
                        format!("({count_target}.count ?? 0)")
                    } else {
                        format!("{count_target}.count")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertGreaterThan({len_expr}, 0, \"expected non-empty value\")"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "        // skipped: {}",
                        FieldSkip::CountOnJsonBridgedLeafInSwift
                            .message(assertion.field.as_deref().unwrap_or(BARE_RESULT_TOKEN))
                    );
                }
            }
        }
        "is_empty" => {
            if bare_result_is_option {
                let _ = writeln!(out, "        XCTAssertNil({result_var}, \"expected nil value\")");
            } else if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else if field_is_array {
                let is_empty_expr = swift_array_is_empty_expr(&field_expr, accessor_is_optional);
                let _ = writeln!(out, "        XCTAssertTrue({is_empty_expr}, \"expected empty value\")");
            } else {
                // Symmetric with not_empty: use .count == 0 on first-class Swift types.
                // Wrap opaque method-call accessors (`result.id()`) with `.toString()` so
                // `.count` lands on Swift `String`, not `RustString` (which lacks `.count`).
                if let Some(count_target) = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref())
                {
                    let len_expr = if accessor_is_optional {
                        format!("({count_target}.count ?? 0)")
                    } else {
                        format!("{count_target}.count")
                    };
                    let _ = writeln!(out, "        XCTAssertEqual({len_expr}, 0, \"expected empty value\")");
                } else {
                    let _ = writeln!(
                        out,
                        "        // skipped: {}",
                        FieldSkip::CountOnJsonBridgedLeafInSwift
                            .message(assertion.field.as_deref().unwrap_or(BARE_RESULT_TOKEN))
                    );
                }
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let swift_val = json_to_swift(v);
                        format!("{string_expr}.contains({swift_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertGreaterThan({compare_expr}, {cast_swift_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertLessThan({compare_expr}, {cast_swift_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertGreaterThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertLessThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasPrefix({swift_val}), \"expected to start with: \\({swift_val})\")"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasSuffix({swift_val}), \"expected to end with: \\({swift_val})\")"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                // Use string_expr.count: for RustString fields string_expr already has
                // .toString() appended, giving a Swift String whose .count is character count.
                let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({string_expr}.count, {n})");
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "        XCTAssertLessThanOrEqual({string_expr}.count, {n})");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                // For fields nested inside an optional parent (e.g. document.nodes where
                // document is Optional), the accessor generates `result.document().nodes()`
                // which doesn't compile in Swift without optional chaining.
                if let Some(count_expr) = swift_array_count_expr(
                    assertion.field.as_deref(),
                    result_var,
                    field_resolver,
                    Some(&field_expr),
                ) {
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                } else {
                    // swift_array_count_expr returns None when the field is a scalar String
                    // marked (incorrectly) as an array in fields_array. Such fields don't
                    // support .count and would produce invalid code.
                    let f = assertion.field.as_deref().unwrap_or(BARE_RESULT_TOKEN);
                    let _ = writeln!(
                        out,
                        "        // skipped: field '{f}' is a scalar String without meaningful .count"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                if let Some(count_expr) = swift_array_count_expr(
                    assertion.field.as_deref(),
                    result_var,
                    field_resolver,
                    Some(&field_expr),
                ) {
                    let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
                } else {
                    // swift_array_count_expr returns None when the field is a scalar String
                    // marked (incorrectly) as an array in fields_array. Such fields don't
                    // support .count and would produce invalid code.
                    let f = assertion.field.as_deref().unwrap_or(BARE_RESULT_TOKEN);
                    let _ = writeln!(
                        out,
                        "        // skipped: field '{f}' is a scalar String without meaningful .count"
                    );
                }
            }
        }
        "is_true" | "is_false" => {
            // `accessor_is_optional` only catches an intermediate `?.` in the chain -- a
            // field that is ITSELF the optional leaf (e.g. `data` in `data.kind`, with no
            // further segment to safe-navigate past) leaves `field_expr` as `result.data()`
            // with no `?.` anywhere, so that check alone misses it. Consult the resolver
            // directly for the leaf's own optionality too. ~keep
            let leaf_is_optional = assertion
                .field
                .as_deref()
                .is_some_and(|f| field_resolver.is_optional(field_resolver.resolve(f)));
            if accessor_is_optional || leaf_is_optional {
                // `T?`: "is_true"/"is_false" mean "present"/"absent" -- `?? false` only
                // type-checks when T is `Bool` and for any other T (e.g. `DataNode?`) it is a
                // compile error. `!= nil` is the interpretation that holds for any T,
                // matching the Rust `.is_some()` convention for this assertion type.
                if assertion.assertion_type == "is_true" {
                    let _ = writeln!(out, "        XCTAssertNotNil({field_expr})");
                } else {
                    let _ = writeln!(out, "        XCTAssertNil({field_expr})");
                }
            } else if assertion.assertion_type == "is_true" {
                let _ = writeln!(out, "        XCTAssertTrue({field_expr})");
            } else {
                let _ = writeln!(out, "        XCTAssertFalse({field_expr})");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil({string_expr}.range(of: {swift_val}, options: .regularExpression), \"expected value to match regex: \\({swift_val})\")"
                );
            }
        }
        "not_error" => {
            super::not_error_assertion::render_not_error_assertion(
                out,
                result_var,
                bare_result_is_option,
                is_streaming,
                returns_void,
            );
        }
        "error" => {
            // ~keep Handled at the test method level, via `render_error_catch_block`
            // in `test_method.rs` (plain success catch or a declared-value check).
        }
        "method_result" => {
            let _ = writeln!(out, "        // method_result assertions not yet implemented for Swift");
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Render an assertion whose field path traverses an array with `[].` (e.g. `links[].url`).
///
/// `dot` is the byte offset of the `[].` separator in `field`, so `field[..dot]` is the array
/// path and `field[dot + 3..]` the per-element sub-path.
///
/// The wildcard means "some element of the array satisfies this", which Swift expresses as
/// `array.contains(where: { ... })`. Only the assertion types with a predicate form get that
/// treatment; every other type is refused with a visible skip.
///
/// Refusing is deliberate. The alternative — falling through to the generic accessor — is not
/// "no traversal", it is a *different assertion*: the shared resolver lowers `foo[]` to
/// `PathSegment::ArrayField { index: 0 }`, so the emitted Swift reads `result.foo()[0].bar()`
/// and passes whenever element zero happens to match, while claiming to cover the whole array.
/// That is a false green, which is strictly worse than a gap you can see. `zig` refuses the
/// nested-wildcard case for exactly this reason (`zig/assertions.rs`), and the skip wording
/// here is the one the other twelve backends already emit, so a wildcard `equals` is a
/// recorded gap in every language rather than a Swift-only accidental pass. ~keep
fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    dot: usize,
    result_var: &str,
    field_resolver: &FieldResolver,
) {
    let array_part = &field[..dot];
    let elem_part = &field[dot + 3..];

    // The split above consumes the FIRST `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part`. The `not_empty` arm builds its element accessor inline instead of
    // going through `swift_traversal_contains_assert`, so without this guard `accessor` lowers that
    // surviving wildcard to index 0 and the `contains(where:)` closure ranges over `pages` while
    // reading `links[0]`. Guarding here rather than per-arm also gives the refused path the wording
    // the strict field-availability gate counts, which the generic `unsupported traversal
    // assertion` fallback deliberately is not. ~keep
    if let Some(line) = nested_wildcard_skip_line("        ", "//", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }

    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(expected) = &assertion.value {
                emit_wildcard_contains(
                    out,
                    expected,
                    false,
                    array_part,
                    elem_part,
                    field,
                    result_var,
                    field_resolver,
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for value in values {
                    emit_wildcard_contains(
                        out,
                        value,
                        false,
                        array_part,
                        elem_part,
                        field,
                        result_var,
                        field_resolver,
                    );
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                emit_wildcard_contains(
                    out,
                    expected,
                    true,
                    array_part,
                    elem_part,
                    field,
                    result_var,
                    field_resolver,
                );
            }
        }
        "not_empty" => {
            let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
            let resolved_full = field_resolver.resolve(field);
            let resolved_elem_part = resolved_full
                .find("[].")
                .map(|d| &resolved_full[d + 3..])
                .unwrap_or(elem_part);
            let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
            let elem_is_enum = field_resolver.is_enum(field);
            let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
                || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
            let elem_str = if elem_is_enum {
                format!("{elem_accessor}.to_string().toString()")
            } else if elem_is_optional {
                format!("({elem_accessor}?.toString() ?? \"\")")
            } else {
                format!("{elem_accessor}.toString()")
            };
            let _ = writeln!(
                out,
                "        XCTAssertTrue({array_accessor}.contains(where: {{ !{elem_str}.isEmpty }}), \"expected non-empty value\")"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

/// Emit one `XCTAssert{True,False}(array.contains(where: { … }), …)` line for a wildcard path.
#[allow(clippy::too_many_arguments)]
fn emit_wildcard_contains(
    out: &mut String,
    value: &serde_json::Value,
    negate: bool,
    array_part: &str,
    elem_part: &str,
    field: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
) {
    let swift_val = json_to_swift(value);
    let msg = if negate {
        format!("expected NOT to contain: \\({swift_val})")
    } else {
        format!("expected to contain: \\({swift_val})")
    };
    let line = swift_traversal_contains_assert(
        array_part,
        elem_part,
        field,
        &swift_val,
        result_var,
        negate,
        &msg,
        field_resolver,
    );
    let _ = writeln!(out, "{line}");
}

#[cfg(test)]
mod nested_wildcard_tests {
    use super::render_wildcard_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn array_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &names, &names, &HashSet::new())
    }

    fn render_not_empty(field: &str, resolver: &FieldResolver) -> String {
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        };
        let dot = field.find("[].").expect("test field must carry a wildcard");
        let mut out = String::new();
        render_wildcard_assertion(&mut out, &assertion, field, dot, "result", resolver);
        out
    }

    /// The control, and the one that matters: a single wildcard must still quantify over every
    /// element, so the refusal below cannot have been implemented by disabling wildcards. ~keep
    #[test]
    fn single_wildcard_not_empty_still_quantifies_over_every_element() {
        let out = render_not_empty("links[].url", &array_resolver("links"));
        assert!(out.contains(".contains(where: {"), "got: {out}");
        assert!(!out.contains("skipped:"), "got: {out}");
        assert!(!out.contains("[0]"), "got: {out}");
    }

    /// `not_empty` was the arm the shared `swift_traversal_contains_assert` guard never covered:
    /// it builds its element accessor inline, so `pages[].links[].url` emitted a closure over
    /// `pages` whose body read `links[0]` — a whole-array claim inspecting one inner element.
    /// Pre-guard this test fails on the emitted `XCTAssertTrue(...contains(where:...))` line. ~keep
    #[test]
    fn nested_wildcard_not_empty_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_not_empty("pages[].links[].url", &array_resolver("pages"));
        assert_eq!(
            out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }
}

#[cfg(test)]
mod skip_marker_tests {
    use super::render_assertion;
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
    use crate::e2e::codegen::field_skip::FieldSkip;
    use crate::e2e::codegen::{SkipVerdict, fail_on_unavailable_field_markers, take_skip_records};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(assertion: &Assertion, is_streaming: bool) -> String {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            &resolver,
            false,
            false,
            false,
            false,
            &HashMap::new(),
            is_streaming,
            false,
        );
        out
    }

    fn assertion_on(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value,
            ..Assertion::default()
        }
    }

    /// Run the shared field funnel over a rendered body and return its verdicts, so a test can
    /// assert what the gate DECIDED rather than only what the text says. ~keep
    fn field_verdicts(body: &str) -> Vec<SkipVerdict> {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(body, "swift", "swift_smoke", &[]);
        take_skip_records().into_iter().map(|record| record.verdict).collect()
    }

    /// Non-vacuity control for every test below: the same harness on an ordinary field must render
    /// a real `XCTAssert`, or "no marker" / "one marker" would be a fact about the harness rather
    /// than about markers. ~keep
    #[test]
    fn the_harness_renders_a_real_assertion_for_an_ordinary_field() {
        let out = render(
            &assertion_on("equals", "title", Some(serde_json::json!("hello"))),
            false,
        );
        assert!(out.contains("XCTAssert"), "got: {out}");
        assert!(
            field_verdicts(&out).is_empty(),
            "a live assertion records no skip: {out}"
        );
    }

    /// The collision the consumer hit: `headings.length` RESOLVES (the guard runs only after
    /// `is_valid_for_result` accepted it) but swift-bridge JSON-bridges the leaf to a `RustString`
    /// with no `.count`, so the backend honestly refuses it. It used to refuse in
    /// `NotAvailableOnResultType`'s words — an `AuthoringGap`, and therefore FATAL — so the
    /// backend called it an ABI limit and the strict gate called it a broken field path, about the
    /// same field, at the same moment. Asserting the VERDICT is what stops the two disagreeing
    /// again: reclassifying this variant fails here rather than silently failing consumers. ~keep
    #[test]
    fn a_json_bridged_count_is_a_limitation_never_an_unacknowledged_gap() {
        let out = render(
            &assertion_on("count_equals", "headings.length", Some(serde_json::json!(3))),
            false,
        );
        assert_eq!(
            FieldSkip::extract_classified(out.trim_end()),
            Some(("headings.length", FieldSkip::CountOnJsonBridgedLeafInSwift)),
            "got: {out}"
        );
        assert_eq!(
            field_verdicts(&out),
            vec![SkipVerdict::Limitation],
            "a resolvable field refused for an ABI reason must never be an unacknowledged gap: {out}"
        );
    }

    /// `stream.has_page_event` has no accessor without a resolved item type, and swift's call site
    /// supplies none. The pre-fix renderer emitted NOTHING and returned, so the assertion vanished
    /// and the emitted test body could end up with no assertion call at all — permanently green.
    #[test]
    fn a_streaming_field_with_no_accessor_emits_a_counted_marker() {
        let out = render(&assertion_on("is_true", "stream.has_page_event", None), true);
        assert!(!out.is_empty(), "the assertion must not vanish");
        assert_eq!(
            FieldSkip::extract_classified(out.trim_end()),
            Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
            "got: {out}"
        );
        assert_eq!(
            field_verdicts(&out),
            vec![SkipVerdict::AwaitingGeneratorSupport],
            "alef's own generator debt is counted, never fatal: {out}"
        );
    }

    /// A streaming assertion type the renderer does not implement used to emit
    /// `// streaming field '<f>': assertion type '<t>' not rendered`, which matched no registered
    /// shape and carried no `skipped:` prefix — invisible to both funnels and to a grep census.
    #[test]
    fn an_unrenderable_streaming_assertion_type_emits_a_registered_marker() {
        let out = render(&assertion_on("matches_regex", "chunks", None), true);
        assert_eq!(
            AssertionTypeSkip::extract_classified(out.trim_end()),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "got: {out}"
        );
    }

    /// A value the renderer cannot narrow used to leave the arm silent.
    #[test]
    fn an_unrenderable_streaming_value_emits_a_registered_marker() {
        let out = render(
            &assertion_on("count_min", "chunks", Some(serde_json::json!("three"))),
            true,
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(out.trim_end()),
            Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
            "got: {out}"
        );
    }
}
