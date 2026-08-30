//! Regression coverage for the `Named`-resolution cycle guard in `file_inputs.rs`.
//!
//! `#[serde(flatten)]` fields recurse against the SAME JSON value rather than a smaller
//! sub-value (see `fields_use_test_documents`), so a self-referential type reached only
//! through flattened fields is no longer bounded by shrinking JSON depth the way every other
//! branch is. `resolve_named_uses_test_documents` guards this with a path-scoped `visited` set
//! that is removed on the way back out. These tests pin both halves of that contract: a true
//! cycle on the active path terminates instead of recursing forever, and a sibling revisit of
//! the same named type at the same level -- not a cycle -- still finds a real file input. The
//! second case is the one a regression to a global (non-backtracking) visited set would
//! silently break: it would mark `SampleLeaf` visited while checking the first sibling and
//! never re-enter it for the second, turning a real file input into a false negative. ~keep

use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

fn arg_for(element_type: &str) -> ArgMapping {
    ArgMapping {
        name: "request".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some(element_type.into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// A struct whose only field flattens itself: `#[serde(flatten)] wrapped: SampleNode`. ~keep
fn self_referential_flatten_type() -> TypeDef {
    TypeDef {
        name: "SampleNode".into(),
        fields: vec![FieldDef {
            name: "wrapped".into(),
            ty: TypeRef::Named("SampleNode".into()),
            serde_flatten: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn self_referential_flatten_field_terminates_and_reports_no_file_input() {
    let fixture = Fixture {
        input: serde_json::json!({}),
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![arg_for("SampleNode")],
        ..Default::default()
    };

    // If this hangs or overflows the stack, the cycle guard regressed. ~keep
    assert!(!super::fixture_uses_test_documents(
        &fixture,
        &call,
        &[self_referential_flatten_type()],
        &[]
    ));
}

fn leaf_type() -> TypeDef {
    TypeDef {
        name: "SampleLeaf".into(),
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Bytes,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Two fields of the SAME named type at the same level -- not a cycle, since each field is an
/// independent branch, but it revisits `SampleLeaf` after the first branch has fully returned. ~keep
fn sibling_pair_type() -> TypeDef {
    TypeDef {
        name: "SampleRequest".into(),
        fields: vec![
            FieldDef {
                name: "first".into(),
                ty: TypeRef::Named("SampleLeaf".into()),
                ..Default::default()
            },
            FieldDef {
                name: "second".into(),
                ty: TypeRef::Named("SampleLeaf".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn sibling_fields_of_the_same_named_type_both_get_checked() {
    // `first` has no real file path; only `second` does. A global (non-backtracking) visited
    // set would mark `SampleLeaf` visited while checking `first` and never re-enter it for
    // `second`, wrongly returning false. ~keep
    let fixture = Fixture {
        input: serde_json::json!({
            "first": {"content": "inline text"},
            "second": {"content": "documents/sample.bin"}
        }),
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![arg_for("SampleRequest")],
        ..Default::default()
    };

    assert!(super::fixture_uses_test_documents(
        &fixture,
        &call,
        &[sibling_pair_type(), leaf_type()],
        &[]
    ));
}
