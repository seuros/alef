//! `swift_build_accessor`'s per-segment walk, split out of `accessors.rs` (which sits close to
//! the file-size-ratchet cap) into `AccessorWalk` plus its per-segment helper functions.
//!
//! ~keep Mechanical extraction of the original single ~120-line function: every statement,
//! emitted string, and mutation order is unchanged — only the accumulator's storage (bundled
//! into a struct, threaded by `&mut`, instead of several bare `let mut` locals) and the split
//! points (already-existing branches in the control flow: per-segment dispatch, then the
//! subscripted-vs-plain segment shapes) are new.

use crate::e2e::field_access::FieldResolver;
use heck::ToLowerCamelCase;

/// ~keep Mutable state threaded across `swift_build_accessor`'s per-segment walk. Bundled
/// into a struct — passed by `&mut` to each per-segment helper — rather than threaded as
/// several separate `&mut` parameters, so each helper reads as one self-contained step over
/// one accumulator instead of a long, easy-to-misorder parameter list.
struct AccessorWalk {
    /// Track the current IR type as we walk segments so each segment can be
    /// emitted with property syntax (first-class Codable struct) or method-call
    /// syntax (typealias-to-`RustBridge.X`). Mirrors the per-segment dispatch in
    /// `render_swift_with_first_class_map`.
    current_type: Option<String>,
    /// Once a chain crosses a `[N]` subscript, we are operating on a RustVec
    /// element, which is always the OPAQUE `RustBridge.T` (swift-bridge does not
    /// convert RustVec elements into the first-class Codable struct). Pin
    /// opaque method-call syntax after the first index step.
    via_rust_vec: bool,
    /// Once a chain crosses an opaque (typealias-to-`RustBridge.X`) segment, every
    /// subsequent accessor must also be opaque (method-call syntax). Calling a
    /// method on `RustBridge.X` returns the OPAQUE wrapper of the next type, even
    /// when that next type is independently eligible for first-class emission.
    /// See `field_access::render_swift_with_first_class_map` for the matching
    /// invariant. Without this, `metrics.total_lines` on an opaque parent emits
    /// `.metrics().totalLines` instead of `.metrics().totalLines()`.
    via_opaque: bool,
    out: String,
    has_optional: bool,
    /// Once we emit a `?` to unwrap an Optional, subsequent segments should NOT
    /// emit additional `?` operators. In Swift, `.summary()?.strategy()` unwraps
    /// to a concrete `SummaryResult`, so `.strategy()` is called on the unwrapped
    /// value and does not need another `?` even if the full path `summary.strategy`
    /// is marked optional in the fixture config.
    already_unwrapped: bool,
    path_so_far: String,
}

/// Returns `(accessor_expr, has_optional)` where `has_optional` is true when
/// at least one `?.` was inserted.
///
/// Note: Once we emit a `?` to unwrap an Optional, Swift's type system treats
/// the result as non-Optional for the remainder of the chain, even if the Rust
/// IR type annotation says the next field is Optional. We track whether the chain
/// is already in an "unwrapped" state via `already_unwrapped` — after the first
/// `?`, subsequent optional fields should NOT emit another `?` because the Swift
/// expression is already concrete.
pub(super) fn swift_build_accessor(field: &str, result_var: &str, field_resolver: &FieldResolver) -> (String, bool) {
    let resolved = field_resolver.resolve(field);
    let parts: Vec<&str> = resolved.split('.').collect();
    let mut walk = AccessorWalk {
        current_type: field_resolver.swift_root_type().cloned(),
        via_rust_vec: false,
        via_opaque: false,
        out: result_var.to_string(),
        has_optional: false,
        already_unwrapped: false,
        path_so_far: String::new(),
    };
    let total = parts.len();
    for (i, part) in parts.iter().enumerate() {
        render_accessor_segment(&mut walk, part, i == total - 1, field_resolver);
    }
    (walk.out, walk.has_optional)
}

/// ~keep Render one dot-separated segment of the field path into `walk.out`, then dispatch
/// to [`render_indexed_segment`] or [`render_plain_segment`] depending on whether this
/// segment carries an array/map subscript (`data[0]`) or not.
fn render_accessor_segment(walk: &mut AccessorWalk, part: &str, is_leaf: bool, field_resolver: &FieldResolver) {
    // Handle array index subscripts within a segment, e.g. `data[0]`.
    // `data[0]` must become `.data()[0]` (opaque) or `.data[0]` (first-class).
    // Split at the first `[` if present.
    let (field_name, subscript): (&str, Option<&str>) = if let Some(bracket_pos) = part.find('[') {
        (&part[..bracket_pos], Some(&part[bracket_pos..]))
    } else {
        (part, None)
    };

    if !walk.path_so_far.is_empty() {
        walk.path_so_far.push('.');
    }
    // Build the base path (without subscript) for the optional check. When the
    // segment is e.g. `tool_calls[0]`, we want to check `is_optional` against
    // "choices[0].message.tool_calls" not "choices[0].message.tool_calls[0]".
    let base_path = {
        let mut p = walk.path_so_far.clone();
        p.push_str(field_name);
        p
    };
    // Now push the full part (with subscript if any) so walk.path_so_far is correct
    // for subsequent segment checks.
    walk.path_so_far.push_str(part);

    // First-class struct fields → property access (no `()`); typealias-to-
    // opaque fields → method-call access (`()`). Once we've indexed through
    // a RustVec, every subsequent segment is on an opaque element.
    // When walk.current_type is None (opaque parent that doesn't appear in field_types),
    // treat it as opaque and use method-call syntax.
    let is_first_class = walk
        .current_type
        .as_ref()
        .is_some_and(|t| field_resolver.swift_is_first_class(Some(t)));
    let property_syntax = !walk.via_rust_vec && !walk.via_opaque && is_first_class;
    if !property_syntax {
        walk.via_opaque = true;
    }
    walk.out.push('.');
    // Swift bindings (both first-class `public let` props and swift-bridge
    // method names) always use lowerCamelCase — never raw snake_case from IR.
    walk.out.push_str(&field_name.to_lower_camel_case());
    if let Some(sub) = subscript {
        render_indexed_segment(walk, field_name, sub, &base_path, property_syntax, field_resolver);
    } else {
        render_plain_segment(walk, field_name, is_leaf, &base_path, property_syntax, field_resolver);
    }
}

/// ~keep Render the `[...]` subscript half of [`render_accessor_segment`]'s dispatch — a
/// segment like `tool_calls[0]`, already split into `field_name` ("tool_calls") and `sub`
/// ("[0]").
fn render_indexed_segment(
    walk: &mut AccessorWalk,
    field_name: &str,
    sub: &str,
    base_path: &str,
    property_syntax: bool,
    field_resolver: &FieldResolver,
) {
    // When the getter for this subscripted field is itself optional
    // (e.g. tool_calls returns Optional<RustVec<T>>), insert `?` before
    // the subscript so Swift unwraps the Optional before indexing.
    // Only emit `?` if we haven't already unwrapped in this chain.
    let field_is_optional = field_resolver.is_optional(base_path);
    let access = if property_syntax { "" } else { "()" };
    if field_is_optional && !walk.already_unwrapped {
        walk.out.push_str(&format!("{access}?"));
        walk.has_optional = true;
        walk.already_unwrapped = true;
    } else {
        walk.out.push_str(access);
    }
    walk.out.push_str(sub);
    // Do NOT append a trailing `?` after the subscript index: in Swift,
    // `optionalVec?[N]` via `Collection.subscript` returns the element
    // type `T` directly. The parent `walk.has_optional` flag is still set
    // when `field_is_optional` is true, which causes the enclosing
    // expression to be wrapped in `(... ?? fallback)` correctly.
    // Indexing into a Vec<Named> yields a Named element. Only pin opaque
    // syntax when the array itself was opaque (method-call); when the
    // owner is first-class, the array is a Swift `[T]` whose elements
    // are first-class T (property access).
    walk.current_type = field_resolver.swift_advance(walk.current_type.as_deref(), field_name);
    if !property_syntax {
        walk.via_rust_vec = true;
    }
}

/// ~keep Render the plain (non-subscripted) half of [`render_accessor_segment`]'s dispatch —
/// a segment like `message` with no trailing `[...]`.
fn render_plain_segment(
    walk: &mut AccessorWalk,
    field_name: &str,
    is_leaf: bool,
    base_path: &str,
    property_syntax: bool,
    field_resolver: &FieldResolver,
) {
    if !property_syntax {
        walk.out.push_str("()");
    }
    // Insert `?` after the accessor for non-leaf optional fields so the
    // next member access becomes `?.`. Only emit `?` if we haven't already
    // unwrapped in this chain with a previous optional chaining operator.
    if !is_leaf && field_resolver.is_optional(base_path) && !walk.already_unwrapped {
        walk.out.push('?');
        walk.has_optional = true;
        walk.already_unwrapped = true;
    }
    walk.current_type = field_resolver.swift_advance(walk.current_type.as_deref(), field_name);
}
