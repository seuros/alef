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
