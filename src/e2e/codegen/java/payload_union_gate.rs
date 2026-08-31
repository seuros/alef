//! The Java assertion generator's refusal for a leaf field the Java binding lowered to a
//! payload-union wrapper class.
//!
//! ~keep `assertions.rs`'s `field_is_enum` already withholds `.getValue()` for this shape, per
//! `backends::java::gen_bindings::emits_get_value`: the binding renders `gen_java_tagged_union`
//! / `gen_java_untagged_wrapper` for a `serde(tag)` / `serde(untagged)` enum with data variants,
//! and neither class declares that accessor. Withholding it is only half a fix, and the
//! dangerous half alone — exactly as `codegen::payload_union_skip`'s module doc records for
//! dart/kotlin/swift. The field then falls through to the same generic pipeline a `String`
//! field takes, and `java/assertion.jinja` reaches it holding the bare wrapper instance:
//!
//! - **regex**: `matches_regex` renders `{expr}.matches(...)`, and the wrapper declares no
//!   `matches`.
//! - **length/count**: `min_length` / `max_length` render `{expr}.length()`, `count_min` /
//!   `count_equals` render `{expr}.size()`; the wrapper declares neither.
//! - **numeric**: `greater_than` and its three siblings render `{expr} > n` on a reference type.
//! - **equality/string**: `contains` / `contains_all` / `contains_any` / `not_contains` /
//!   `starts_with` / `ends_with` call `String` methods the wrapper does not have, and `equals`
//!   renders `assertEquals("wire_literal", wrapperInstance)` — which javac accepts, through
//!   `assertEquals(Object, Object)`, and which is false for every fixture that ever runs. That
//!   compiling-but-always-false arm is why the refusal has to cover the equality family too and
//!   not only the arms that fail to compile.
//! - **invalid boolean**: `is_true` / `is_false` render `assertTrue(wrapperInstance, ...)` when
//!   the field is not optional, and `assertTrue` takes a `boolean`.
//!
//! Three shapes are NOT refused, because the expression the generator actually emits for them is
//! real Java that asserts what it claims. See [`leaf_assertion_is_substantiated`].
//!
//! ~keep A NON-optional payload-union leaf is refused even for `not_empty` / `is_empty`. The
//! template has an object arm for those (`assertNotNull` / `assertNull`), but it is gated on
//! `field_shape::classify`'s `field_is_object`, which reads `FieldResolver::is_display_unsafe`
//! → `ir_result_fields::leaf_is_named_type` → the `field_types` map — and
//! `record_ir_result_field_kind` enters a field there only when its named type is a STRUCT,
//! routing every enum-typed field to `unresolvable_named_fields` instead. So `field_is_object`
//! is false for every payload-union leaf, the template takes its `{expr}.isEmpty()` arm, and
//! that does not compile on the wrapper. Taking `field_is_object` as a parameter here would
//! read as shape-awareness while being a disjunct that can never be true.

use crate::e2e::codegen::payload_union_skip::{UnionLoweringTarget, payload_union_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Indentation and comment opener of a rendered Java assertion line, matching every other
/// `// skipped:` line `assertions.rs` emits.
const JAVA_ASSERTION_INDENT: &str = "        ";
const JAVA_COMMENT_OPEN: &str = "//";

/// Register a skip and return `true` when `assertion`'s family cannot be lowered onto this
/// leaf's payload-union wrapper class; `false` when the leaf is not a payload union at all, or
/// when the family is one the leaf's own lowering substantiates.
///
/// Callers must have already excluded a sealed-interface field (`is_sealed_display_field`),
/// whose generated `{TypeName}Display.toDisplayString` helper hands every string-shaped family a
/// genuine `String`, and a `result_is_simple` call, whose `field_expr` ignores the leaf entirely.
///
/// ~keep This must run BEFORE `render_assertion` dispatches to a lowering, not after. A
/// bracket-wildcard path is lowered by `assertion_wildcard::render_wildcard_assertion`, which
/// returns without ever consulting a later gate, so a check placed after it decides nothing for
/// the wildcard case.
pub(super) fn try_skip_unsupported_family(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
) -> bool {
    let Some(field) = assertion.field.as_deref().filter(|f| !f.is_empty()) else {
        return false;
    };
    let Some(line) = payload_union_skip_line(
        JAVA_ASSERTION_INDENT,
        JAVA_COMMENT_OPEN,
        field_resolver,
        Some(field),
        UnionLoweringTarget::Java,
    ) else {
        return false;
    };
    if leaf_assertion_is_substantiated(assertion, field_resolver, field) {
        return false;
    }
    out.push_str(&line);
    out.push('\n');
    true
}

/// Whether the expression `render_assertion` emits for this family on a payload-union leaf is
/// real Java that asserts what it claims.
///
/// Exactly three shapes qualify, in the order tested below.
///
/// ~keep A bracket-wildcard path qualifies for NONE of them, and is checked first. Its lowering
/// is `assertion_wildcard::render_wildcard_assertion`, not `assertion.jinja`, and that renderer
/// stringifies each element with `String.valueOf({elem_accessor})` — on a wrapper leaf that is
/// `toString()`, i.e. Jackson's JSON rendering, quotes and object keys included. Its `contains`
/// arms then match against that diagnostic form rather than the value (a fixture asserting a
/// key name would match a JSON key), and its `not_empty` arm cannot fail at all, because
/// `String.valueOf` of an absent payload is the four-character `"null"`. Neither is a compile
/// error, so nothing else would ever catch them.
fn leaf_assertion_is_substantiated(assertion: &Assertion, field_resolver: &FieldResolver, field: &str) -> bool {
    if field_resolver.wildcard_split(field).is_some() {
        return false;
    }
    if !lowers_through_optional(field_resolver, field) {
        return false;
    }
    if field_resolver.is_display_as_text(field) && text_surface_supports(assertion) {
        return true;
    }
    matches!(
        assertion.assertion_type.as_str(),
        "not_empty" | "is_empty" | "is_true" | "is_false"
    )
}

/// Whether the `.text()` surface a `fields_display_as_text` field lowers through genuinely
/// supports `assertion`'s family.
///
/// ~keep `.text()` is a real accessor — `gen_enum_class` passes `emit_text` to
/// `gen_java_untagged_wrapper` for exactly the types that config names — but it yields a
/// `String`, so only the families a `String` answers are substantiated by it:
///
/// - **length** (`min_length` / `max_length`) — `{expr}.length()` on a `String`. Supported.
/// - **string/equality** (`equals`, `contains`, `contains_all`, `contains_any`, `not_contains`,
///   `starts_with`, `ends_with`) — supported, but ONLY for string-valued fixtures. A numeric
///   fixture value routes `equals` through the template's
///   `.map(Number::longValue)` arm, which does not compile on a `String`, and the containment
///   arms would pass a non-`String` argument to `String.contains`.
/// - **numeric** (`greater_than` and siblings) — renders `{String} > n`. Does not compile.
/// - **count** (`count_min` / `count_equals`) — renders `{String}.size()`. Does not compile.
/// - **regex** (`matches_regex`) — this one DOES compile, since `String.matches` exists. It is
///   refused on soundness rather than compilability: a regex fixture is written against the
///   union's wire form, while `.text()` is a lossy display projection that returns `""` for
///   object- and array-shaped values and concatenates content parts. Matching a structural
///   pattern against that projection asserts something other than what the fixture says.
/// - **boolean** (`is_true` / `is_false`) — the text surface plays no part: those types never
///   reach `.text()` at all, because `render_assertion`'s display-as-text branch returns the raw
///   `Optional` for them. They stay substantiated through the presence rule in
///   [`leaf_assertion_is_substantiated`], which is where that decision belongs.
fn text_surface_supports(assertion: &Assertion) -> bool {
    match assertion.assertion_type.as_str() {
        "min_length" | "max_length" => true,
        "equals" | "contains" | "contains_all" | "contains_any" | "not_contains" | "starts_with" | "ends_with" => {
            every_value_is_a_string(assertion)
        }
        _ => false,
    }
}

/// Whether every fixture value this assertion carries is a JSON string, so the `String` arms the
/// text surface supports receive `String` arguments.
fn every_value_is_a_string(assertion: &Assertion) -> bool {
    let scalar_is_string = assertion.value.as_ref().is_none_or(serde_json::Value::is_string);
    let list_is_string = assertion
        .values
        .as_ref()
        .is_none_or(|values| values.iter().all(serde_json::Value::is_string));
    scalar_is_string && list_is_string
}

/// Whether `render_assertion` wraps this field's accessor in `Optional.ofNullable(...)`.
///
/// ~keep Mirrors that function's own `is_optional(resolved) && !has_map_access(field)` guard
/// exactly. A map-access path keeps the bare accessor even when the field is declared optional,
/// so asking `is_optional` alone would exempt presence checks that render `{expr}.isEmpty()` on
/// a wrapper instance and do not compile.
fn lowers_through_optional(field_resolver: &FieldResolver, field: &str) -> bool {
    field_resolver.is_optional(field_resolver.resolve(field)) && !field_resolver.has_map_access(field)
}
