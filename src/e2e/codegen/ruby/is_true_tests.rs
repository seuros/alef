//! Regression tests for `is_true`/`is_false` on an `Option<T>` field.
//!
//! Split into its own file rather than added to `ruby/assertions.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep

use super::assertions::render_assertion;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn optional_resolver(field: &str) -> FieldResolver {
    let optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn is_true_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_true".to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
    let mut out = String::new();
    let e2e_config = E2eConfig::default();
    render_assertion(
        &mut out,
        assertion,
        "result",
        resolver,
        false,
        &e2e_config,
        &HashSet::new(),
        &HashMap::new(),
    );
    out
}

/// `Option<DataNode>` presence: before the fix this rendered `expect(result.data).to be
/// true`, RSpec's identity matcher (`equal?(true)`), which never matches a present
/// non-boolean object.
#[test]
fn is_true_on_optional_struct_field_checks_presence() {
    let out = render(&optional_resolver("data"), &is_true_assertion("data"));
    assert_eq!(out, "    expect(result.data).not_to be_nil\n");
}

#[test]
fn is_false_on_optional_struct_field_checks_absence() {
    let out = render(
        &optional_resolver("data"),
        &Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("data".to_string()),
            ..Assertion::default()
        },
    );
    assert_eq!(out, "    expect(result.data).to be_nil\n");
}

#[test]
fn is_true_on_non_optional_field_is_unchanged() {
    let out = render(&empty_resolver(), &is_true_assertion("active"));
    assert_eq!(out, "    expect(result.active).to be true\n");
}
