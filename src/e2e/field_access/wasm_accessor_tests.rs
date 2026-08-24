//! `wasm` is TypeScript, and its accessor must answer the optionality question the same way
//! `node` does -- differing only where the two bindings genuinely lower a map differently.
//!
//! ~keep `accessor()` dispatched `"typescript" | "node"` to the optional-aware renderer and let
//! `"wasm"` fall through to the catch-all, whose `render_wasm` knows nothing about optionality.
//! One fixture therefore produced `result.document?.nodes` for node and the `TS18048`
//! `result.document.nodes` for wasm, and every wasm structure snippet on an `Option<T>` field
//! failed to compile. The map difference (a NAPI `HashMap` is an object, a wasm-bindgen one is a
//! JS `Map`) is real and is the only thing the two renderings may still disagree about.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn resolver_with_optional(fields: &[&str]) -> FieldResolver {
    let optional: HashSet<String> = fields.iter().map(|field| (*field).to_string()).collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

#[test]
fn node_and_wasm_chain_an_optional_link_identically() {
    let resolver = resolver_with_optional(&["document"]);

    let node = resolver.accessor("document.nodes", "node", "result");
    let wasm = resolver.accessor("document.nodes", "wasm", "result");

    assert_eq!(node, wasm, "one language, one accessor");
    assert_eq!(wasm, "result.document?.nodes");
}

#[test]
fn node_and_wasm_leave_a_required_link_unguarded() {
    let resolver = resolver_with_optional(&[]);

    let node = resolver.accessor("summary.nodes", "node", "result");
    let wasm = resolver.accessor("summary.nodes", "wasm", "result");

    assert_eq!(node, wasm, "one language, one accessor");
    assert_eq!(wasm, "result.summary.nodes");
}

/// The one difference that survives, and the shape it takes under an optional receiver: a `get`
/// is a member access, so it takes `?.get(...)` where node's element access takes `?.[...]`.
/// Emitting the element form's `?.` before a `.get` would produce the un-parseable `?..get`.
#[test]
fn wasm_reads_a_map_through_get_and_node_through_an_index() {
    let resolver = resolver_with_optional(&["metadata"]);

    assert_eq!(
        resolver.accessor("metadata[title]", "node", "result"),
        "result.metadata?.[\"title\"]"
    );
    assert_eq!(
        resolver.accessor("metadata[title]", "wasm", "result"),
        "result.metadata?.get(\"title\")"
    );
}

#[test]
fn wasm_reads_a_required_map_through_get_without_a_guard() {
    let resolver = resolver_with_optional(&[]);

    assert_eq!(
        resolver.accessor("metadata[title]", "wasm", "result"),
        "result.metadata.get(\"title\")"
    );
}
