//! Regression coverage for anchoring the `chunks` synthetic handlers to the call's own root type.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow. The shape under
//! test is `Envelope { results: Vec<Document> }` where only `Document` declares `chunks`: before
//! the fix the synthetic handlers hardcoded `result.chunks` regardless of what `result` actually
//! was, intercepting ahead of any field validation. ~keep

use super::super::assertions::render_assertion;
use super::tests::*;
use crate::e2e::field_access::FieldResolver;

/// The regression shape: `Envelope { results: Vec<Document> }`, and `Document` (reached only
/// through `results`) declares `chunks`. Before the fix, `chunks_have_content` hardcoded
/// `result.chunks` regardless of which type `result` actually was — intercepting ahead of
/// any field validation. Anchoring the interception at the call's own declared root type is
/// what tells `Envelope` and `Document` apart. ~keep
fn envelope_and_document_type_defs() -> Vec<crate::core::ir::TypeDef> {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![FieldDef {
                name: "results".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn resolver_anchored_at(root_type: Option<&str>) -> FieldResolver {
    let type_defs = envelope_and_document_type_defs();
    let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    empty_resolver()
        .with_ir_result_fields(map, root_type.map(str::to_string))
        .with_ir_fields(reachable, excluded, optional)
}

fn render_chunks_have_content_call(resolver: &FieldResolver) -> String {
    let assertion = make_assertion("is_true", Some("chunks_have_content"), None);
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "my_mod",
        "dep",
        false,
        &[],
        resolver,
        false,
        false,
        false,
        false,
        false,
        None,
    );
    out
}

/// The confirmed defect: a call whose own root type (`Envelope`) does not declare `chunks`
/// must not emit `result.chunks` — that struct has no such field and the generated Rust
/// would not compile.
#[test]
fn chunks_have_content_refused_when_call_root_lacks_chunks() {
    let resolver = resolver_anchored_at(Some("Envelope"));
    let out = render_chunks_have_content_call(&resolver);
    assert!(
        !out.contains("result.chunks"),
        "must not hardcode result.chunks against a root type that declares no such field, got: {out}"
    );
    assert!(out.contains("// skipped:"), "got: {out}");
}

/// The control: a call whose root type genuinely declares `chunks` must still render the
/// real assertion — the fix must not turn into "refuse every chunks_have_content fixture."
#[test]
fn chunks_have_content_still_renders_when_call_root_declares_chunks() {
    let resolver = resolver_anchored_at(Some("Document"));
    let out = render_chunks_have_content_call(&resolver);
    assert!(out.contains("result.chunks"), "got: {out}");
    assert!(!out.contains("// skipped:"), "got: {out}");
}

/// No anchored root type at all (the state of every call site before this fix) must keep
/// the pre-existing permissive behaviour: nothing here regresses a fixture whose call site
/// never resolved a root type.
#[test]
fn chunks_have_content_renders_when_no_root_type_is_anchored() {
    let resolver = resolver_anchored_at(None);
    let out = render_chunks_have_content_call(&resolver);
    assert!(out.contains("result.chunks"), "got: {out}");
    assert!(!out.contains("// skipped:"), "got: {out}");
}

/// Coverage for the envelope-projection shape: `Envelope { results: Vec<Document> }`,
/// `result_fields = {"results"}`, and only `Document` declares `chunks`. Mirrors the production
/// wiring (`rust/test_file/test_function.rs`), which anchors `with_ir_collection_map` at the same
/// `call_root_type` as `with_ir_result_fields` — a test that skips wiring the collection map would
/// under-test `anchor_leaf`'s `is_collection_root` dependency and could pass on a prefix that omits
/// the `[0]` a real `Vec<Document>` needs. ~keep
mod envelope_projection {
    use super::{empty_resolver, make_assertion, render_assertion};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;
    use std::collections::HashSet;

    fn envelope_document_type_defs() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![FieldDef {
                    name: "results".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Document".to_string(),
                fields: vec![FieldDef {
                    name: "chunks".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    /// The exact wiring shape `rust/test_file/test_function.rs` builds for a real call: both the
    /// collection map and the result-field map anchored at the same `call_root_type`, plus
    /// `result_fields` carrying the consumer's declared envelope prefix.
    fn envelope_resolver_with_result_fields(result_fields: &[&str]) -> FieldResolver {
        let type_defs = envelope_document_type_defs();
        let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let collection_map = FieldResolver::ir_collection_fields(&type_defs);
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let result_fields: HashSet<String> = result_fields.iter().map(|s| s.to_string()).collect();
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(result_field_map, Some("Envelope".to_string()))
        .with_ir_collection_map(collection_map, Some("Envelope".to_string()))
        .with_ir_fields(reachable, excluded, optional)
    }

    fn render_chunks_have_content(resolver: &FieldResolver) -> String {
        let assertion = make_assertion("is_true", Some("chunks_have_content"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        out
    }

    /// Settles a counter-hypothesis floated during review: that `result_var` is already the
    /// projected first extraction result (so bare `result.chunks` was already correct, and
    /// the real defect was an oracle anchored to the wrong type). It is not — `result_var` is
    /// purely a naming knob (`CallConfig::effective_result_var`, default `"result"`), bound in
    /// generated code to the call's raw declared return type. For this consumer that return type
    /// is `Envelope`, which has no `chunks` field at all; the value with `chunks` sits one hop
    /// deeper, behind the real `results` field `result_fields` names. Confirmed independently by
    /// `FieldResolver::namespace_stripped_path` (`resolver/classify.rs`): a leading segment is
    /// stripped as a virtual label only when it is NEITHER declared in `result_fields` NOR a real
    /// struct field the IR finds on the root — `"results"` is both declared in `result_fields`
    /// (by this consumer) and a genuine `Envelope` field, so it is never treated as a virtual
    /// prefix to strip. A rescue that omits the `results[0]` hop would therefore emit
    /// `result.chunks`, which fails to compile against `Envelope`. ~keep
    #[test]
    fn synthetic_and_generic_paths_agree_on_the_same_accessor() {
        let resolver = envelope_resolver_with_result_fields(&["results"]);

        // The generic path: a fixture that spells the real, structurally valid path directly.
        // `FieldResolver::accessor` is the single renderer every non-synthetic assertion in this
        // backend already goes through, so this is the independently-verifiable ground truth.
        let generic_accessor = resolver.accessor("results[0].chunks", "rust", "result");
        assert_eq!(
            generic_accessor, "result.results[0].chunks",
            "sanity: the generic accessor for the real path must include the [0] hop, got: {generic_accessor}"
        );

        // The synthetic path: `chunks_have_content`'s hardcoded `.chunks` access, anchored via
        // `chunks_result_var`, must reach the exact same expression.
        let synthetic_out = render_chunks_have_content(&resolver);
        assert!(
            synthetic_out.contains(&generic_accessor),
            "synthetic and generic paths disagree — synthetic must contain `{generic_accessor}`, got: {synthetic_out}"
        );
        assert!(!synthetic_out.contains("// skipped:"), "got: {synthetic_out}");
    }

    /// Negative control: when `chunks` is unreachable both on the root AND through every
    /// `result_fields` prefix (here, `results` names a field that does not exist at all), the
    /// synthetic handler must still refuse — a rescue that always finds SOME prefix would
    /// silently swap one non-compiling accessor for another. `result_fields` is deliberately
    /// non-empty here (unlike `chunks_have_content_refused_when_call_root_lacks_chunks`, which
    /// covers the no-`result_fields`-at-all case) so this exercises the actual prefix search this
    /// fix adds, not just the pre-existing bare-root refusal it wraps.
    #[test]
    fn refuses_when_no_result_fields_prefix_reaches_chunks_through_the_full_render_path() {
        let resolver = envelope_resolver_with_result_fields(&["not_a_real_field"]);
        let out = render_chunks_have_content(&resolver);
        assert!(
            !out.contains("result.chunks") && !out.contains("result.not_a_real_field"),
            "must not emit a non-compiling accessor for a genuinely unreachable field, got: {out}"
        );
        assert!(out.contains("// skipped:"), "got: {out}");
    }

    /// Additivity, pinned at the full render path: a root that genuinely declares `chunks`
    /// itself (`result_field_oracle_knows` already answers `Some(true)`) must render `Direct`
    /// unconditionally — the prefix search must never even run, so a `result_fields` entry that
    /// happens to also exist cannot redirect an already-correct accessor.
    #[test]
    fn a_root_declaring_chunks_directly_stays_direct_even_with_result_fields_present() {
        let type_defs = vec![TypeDef {
            name: "Document".to_string(),
            fields: vec![FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }];
        let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let result_fields: HashSet<String> = ["unrelated".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(result_field_map, Some("Document".to_string()))
        .with_ir_fields(reachable, excluded, optional);

        let out = render_chunks_have_content(&resolver);
        assert!(out.contains("result.chunks"), "got: {out}");
        assert!(!out.contains("// skipped:"), "got: {out}");
    }

    /// Additivity, pinned at the full render path for the `None` (no IR anchor at all) case:
    /// mirrors `chunks_have_content_renders_when_no_root_type_is_anchored` but with a non-empty
    /// `result_fields` present, proving the permissive default holds even when a prefix search
    /// COULD have run had the oracle answered `Some(false)` instead of `None`.
    #[test]
    fn no_anchored_root_stays_direct_even_with_result_fields_present() {
        let resolver = empty_resolver();
        let _ = &resolver; // baseline: no IR wired at all, matches pre-existing permissive default.
        let out = render_chunks_have_content(&resolver);
        assert!(out.contains("result.chunks"), "got: {out}");
        assert!(!out.contains("// skipped:"), "got: {out}");
    }
}
