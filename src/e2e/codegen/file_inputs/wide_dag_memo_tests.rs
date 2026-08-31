//! Complexity regression for the `Named`-resolution traversal in `file_inputs.rs`.
//!
//! `#[serde(flatten)]` recurses against the SAME JSON value rather than a smaller sub-value, so a
//! type graph in which one definition is reachable by many flattened paths is a DAG over a single
//! value, not a tree. Without memoization each path re-walks the whole subtree below it, which is
//! exponential in the number of diamonds -- and the blow-up is only visible when the answer is
//! `false`, because `Iterator::any` short-circuits the moment a real file input is found.
//!
//! These tests assert a COUNT, not a duration: `scan_fixture` reports how many `Named` resolutions
//! the traversal entered, so the bound is deterministic, survives CI noise, and pins the
//! complexity class itself rather than the speed of the machine running it. ~keep

use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

/// Number of diamonds chained head-to-tail. Twelve keeps the un-memoized cost (16_381
/// resolutions) large enough to be unmistakable while the graph itself stays at 37 definitions. ~keep
const DIAMOND_LEVELS: u32 = 12;

/// Definitions in the fixture graph: one `Level` and two bridges per diamond, plus the leaf. ~keep
const GRAPH_DEFINITIONS: usize = 3 * DIAMOND_LEVELS as usize + 1;

/// Every distinct path is walked separately, so `Level{i}` is entered 2^i times and each of its
/// two bridges likewise: `sum(2^i, 0..=k) + 2 * sum(2^i, 0..k)` = `2^(k+2) - 3`. ~keep
const UNMEMOIZED_NAMED_RESOLUTIONS: usize = 2usize.pow(DIAMOND_LEVELS + 2) - 3;

/// With the memo, every edge into an already-computed (value, name) pair is an O(1) hit, so the
/// traversal enters `Named` resolution once per EDGE plus once for the root: `4k + 1`. ~keep
const MEMOIZED_NAMED_RESOLUTIONS: usize = 4 * DIAMOND_LEVELS as usize + 1;

/// When the leaf does carry a document path, `any` short-circuits down the leftmost spine and
/// neither traversal explores the width: `2k + 1` either way. ~keep
const SHORT_CIRCUIT_NAMED_RESOLUTIONS: usize = 2 * DIAMOND_LEVELS as usize + 1;

/// The bound asserted below is only worth asserting because the pre-fix traversal blows past it by
/// more than two orders of magnitude. Pinning that relationship at compile time keeps a future
/// edit from shrinking `DIAMOND_LEVELS` until the fixture no longer separates the two. ~keep
const _: () = assert!(
    MEMOIZED_NAMED_RESOLUTIONS * 300 < UNMEMOIZED_NAMED_RESOLUTIONS,
    "the diamond chain must be wide enough that memoization changes the complexity class"
);

fn root_arg() -> ArgMapping {
    ArgMapping {
        name: "request".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("Level0".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn level_name(level: u32) -> String {
    if level == DIAMOND_LEVELS {
        "SampleLeaf".to_string()
    } else {
        format!("Level{level}")
    }
}

fn flattened_field(name: &str, target: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Named(target.into()),
        serde_flatten: true,
        ..Default::default()
    }
}

/// `Level{i}` flattens into two bridges; each bridge flattens into `Level{i+1}`. Because every
/// edge is flattened, all 37 definitions are walked against the very same root JSON object -- the
/// exact shape memoization has to collapse. ~keep
fn diamond_chain_types() -> Vec<TypeDef> {
    let mut types = Vec::with_capacity(GRAPH_DEFINITIONS);
    for level in 0..DIAMOND_LEVELS {
        let left = format!("LeftBridge{level}");
        let right = format!("RightBridge{level}");
        let next = level_name(level + 1);
        types.push(TypeDef {
            name: level_name(level),
            fields: vec![flattened_field("left", &left), flattened_field("right", &right)],
            ..Default::default()
        });
        types.push(TypeDef {
            name: left,
            fields: vec![flattened_field("down", &next)],
            ..Default::default()
        });
        types.push(TypeDef {
            name: right,
            fields: vec![flattened_field("down", &next)],
            ..Default::default()
        });
    }
    types.push(TypeDef {
        name: "SampleLeaf".into(),
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Bytes,
            ..Default::default()
        }],
        ..Default::default()
    });
    types
}

fn scan(content: &str) -> (bool, usize) {
    let fixture = Fixture {
        input: serde_json::json!({ "content": content }),
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![root_arg()],
        ..Default::default()
    };
    let types = diamond_chain_types();
    assert_eq!(
        types.len(),
        GRAPH_DEFINITIONS,
        "the fixture graph must actually be wide -- a smaller graph would make the count bound vacuous"
    );
    let index = super::build_named_index(&types, &[]);
    super::scan_fixture(&index, &fixture, &call)
}

#[test]
fn wide_flatten_diamond_chain_enters_named_resolution_a_linear_number_of_times() {
    // The leaf's bytes are inline, so nothing short-circuits and the whole DAG is explored.
    // Pre-fix this cost 16_381 `Named` resolutions; memoized it costs 49. ~keep
    let (found, named_resolutions) = scan("inline text");

    assert!(!found, "inline leaf bytes are not a document path");
    assert_eq!(
        named_resolutions, MEMOIZED_NAMED_RESOLUTIONS,
        "each (value, name) pair must be computed once and every further edge served from the memo"
    );
}

#[test]
fn memoized_wide_diamond_chain_still_finds_the_leaf_document_path() {
    // Same graph, real document path at the leaf. This is the control that proves the traversal
    // still descends all 12 levels rather than the count above being cheap by not looking. ~keep
    let (found, named_resolutions) = scan("documents/sample.bin");

    assert!(
        found,
        "the leaf document path must still be reached through 12 flattened diamonds"
    );
    assert_eq!(named_resolutions, SHORT_CIRCUIT_NAMED_RESOLUTIONS);
}
