//! Every reason the TypeScript e2e generator refuses an assertion FIELD outright, in the order
//! it applies them.
//!
//! ~keep Split out of `assertions.rs` (already over the repo's 1,000-line cap) rather than grown
//! there, matching the precedent set by `is_true_tests.rs` and `node_enum_import_tests.rs`.
//!
//! The refusals are ordered, not independent: the result-type miss is checked first because it is
//! the coarse "does this path exist at all" question, and the tagged-union crossing second
//! because it only makes sense for a path that does exist. Keeping them behind one entry point
//! means a caller cannot add a third refusal by writing another `writeln!` somewhere in the
//! render path and quietly bypassing the `FieldSkip` funnel the strict gate counts.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// The rendered `// skipped: ...` line refusing `field`, or `None` when the field survives every
/// refusal and the caller should render a real assertion for it.
///
/// The returned line carries its own trailing newline, so callers `push_str` it directly.
pub(super) fn refusal_line(field: &str, field_resolver: &FieldResolver) -> Option<String> {
    if !field_resolver.is_valid_for_result(field) {
        return Some(skip_line(FieldSkip::NotAvailableOnResultType, field));
    }
    // Ask the same authority gleam, dart, kotlin and swift ask -- the consumer's own
    // `fields_method_calls`, read through `FieldResolver::tagged_union_split` -- rather than
    // re-deriving "is this a union crossing" from the path's shape here. This generator used to
    // ask nobody: `test_case.rs` built its resolver with an EMPTY method-call set, so the split
    // answered `None` for every path and the boundary was rendered verbatim by an accessor
    // renderer whose only per-segment decision is `.` vs `?.`. Neither binding this generator
    // serves has a member at that segment to spell -- NAPI flattens a data enum into a single
    // object (discriminant plus every variant's fields as optional siblings) and wasm-bindgen
    // emits a structural union that a straight-line assertion cannot narrow -- so the generated
    // suite failed to compile on `TS2339` rather than merely asserting the wrong thing. ~keep
    if field_resolver.tagged_union_split(field).is_some() {
        return Some(skip_line(FieldSkip::CrossesTaggedUnionBoundaryInTypescript, field));
    }
    None
}

fn skip_line(skip: FieldSkip, field: &str) -> String {
    format!("    // skipped: {}\n", skip.message(field))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::refusal_line;
    use crate::e2e::field_access::FieldResolver;

    /// A resolver holding nothing but the crossing declaration, mirroring what
    /// `test_file/test_case.rs` builds from `[e2e].fields_method_calls`. Empty `result_fields`
    /// makes `is_valid_for_result` accept every path, so the first refusal cannot mask the
    /// second and each test isolates the refusal it names.
    fn resolver_declaring(method_calls: &[&str]) -> FieldResolver {
        let declared: HashSet<String> = method_calls.iter().map(|entry| (*entry).to_string()).collect();
        let empty = HashSet::new();
        FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &declared)
    }

    /// A `fields_method_calls` entry names `<enum field path>.<variant>` -- `shape.circle` for
    /// the crossing `shape.circle.radius` walks -- exactly as `kotlin/assertions/tests.rs`
    /// declares it. ~keep
    #[test]
    fn a_declared_tagged_union_crossing_is_refused_rather_than_spelled() {
        let resolver = resolver_declaring(&["shape.circle"]);

        let line = refusal_line("shape.circle.radius", &resolver)
            .expect("a declared union crossing must be refused, not rendered as an accessor");

        assert_eq!(
            line,
            "    // skipped: field 'shape.circle.radius' crosses a tagged-union variant boundary \
             (no variant member on the generated TypeScript type)\n"
        );
    }

    /// The control that stops "refuse everything" from passing: ordinary struct paths through the
    /// very same resolver must survive so the caller renders their normal accessors -- including
    /// the union field itself, which is a real member and only its VARIANT segment is not.
    #[test]
    fn an_ordinary_struct_field_is_not_refused() {
        let resolver = resolver_declaring(&["shape.circle"]);

        assert_eq!(refusal_line("summary.title", &resolver), None);
        assert_eq!(refusal_line("shape", &resolver), None);
    }

    /// The refusal is keyed on the consumer's declaration, not on "the path has three segments".
    /// With nothing declared, the same path is spelled exactly as before this refusal existed.
    #[test]
    fn an_undeclared_deep_path_is_not_refused() {
        let resolver = resolver_declaring(&[]);

        assert_eq!(refusal_line("shape.circle.radius", &resolver), None);
    }
}
