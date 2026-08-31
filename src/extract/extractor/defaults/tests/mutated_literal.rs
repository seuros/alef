//! `fn default()` bodies that build their value through a mutable local binding and mutate it
//! before returning it.
//!
//! Every expectation here is the value the Rust code *actually produces at runtime*, not the
//! value of the initial struct literal. Before the mutation-aware reader, the extractor
//! recorded the literal and stopped: case by case it wrote `IntLiteral(0)` for a field
//! assigned `9` two lines later, and `Empty` — which asserts "the default is exactly this
//! type's zero" — for a `Vec` that had two elements pushed into it. Those are not rounding
//! errors; they are confident claims that no downstream backend can distinguish from a value
//! alef genuinely read. ~keep

use super::*;

/// `p.max_depth = 9` after the literal. The recorded default must be `9`, the value
/// `P::default().max_depth` really has.
#[test]
fn scalar_assignment_after_the_literal_replaces_the_initial_value() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 0 };
                        prefs.max_depth = 9;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(9))],
        "the assignment after the literal is the default; reading only the literal records 0"
    );
}

/// Repeated `push` into a `Vec::new()`. `Empty` would assert the default is the empty vector,
/// which is the one thing it demonstrably is not.
#[test]
fn repeated_vec_push_becomes_a_list_literal_in_source_order() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub deny_list: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { deny_list: Vec::new() };
                        prefs.deny_list.push("alpha".to_string());
                        prefs.deny_list.push("beta".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("deny_list", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_eq!(
        resolved,
        vec![(
            "deny_list".to_string(),
            DefaultValue::ListLiteral(vec![
                DefaultValue::StringLiteral("alpha".to_string()),
                DefaultValue::StringLiteral("beta".to_string()),
            ]),
        )],
        "pushed elements are the default; `Empty` would claim the vector is empty"
    );
}

/// `extend` over a non-empty literal appends rather than replacing.
#[test]
fn vec_extend_appends_to_the_literals_own_elements() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub deny_list: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { deny_list: vec!["alpha".to_string()] };
                        prefs.deny_list.extend(vec!["beta".to_string()]);
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("deny_list", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_eq!(
        resolved,
        vec![(
            "deny_list".to_string(),
            DefaultValue::ListLiteral(vec![
                DefaultValue::StringLiteral("alpha".to_string()),
                DefaultValue::StringLiteral("beta".to_string()),
            ]),
        )],
        "extend appends to the literal's elements rather than discarding the extension"
    );
}

/// Extending with an empty collection adds nothing, so the field keeps exactly what the
/// literal gave it — `Empty` stays the honest answer here, and must not become a list.
#[test]
fn extending_with_an_empty_collection_leaves_the_field_at_its_literal_value() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub deny_list: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { deny_list: Vec::new() };
                        prefs.deny_list.extend(vec![]);
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("deny_list", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_eq!(
        resolved,
        vec![("deny_list".to_string(), DefaultValue::Empty)],
        "an empty extension changes nothing, so the known-zero claim stays true"
    );
}

/// The shape reported from the PHP lane: a scalar assignment and a collection push in the same
/// body. Both fields were wrong before, in two different ways.
#[test]
fn a_scalar_assignment_and_a_push_in_one_body_are_both_applied() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub max_depth: u32, pub deny_list: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 0, deny_list: Vec::new() };
                        prefs.deny_list.push("alpha".to_string());
                        prefs.max_depth = 9;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[
            ("max_depth", TypeRef::Unit),
            ("deny_list", TypeRef::Vec(Box::new(TypeRef::String))),
        ],
    );

    assert_eq!(
        resolved,
        vec![
            ("max_depth".to_string(), DefaultValue::IntLiteral(9)),
            (
                "deny_list".to_string(),
                DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral("alpha".to_string())]),
            ),
        ],
        "every mutation in the body applies, not just the ones of one kind"
    );
}

/// A delegating `fn default()` whose constructor is the one doing the mutating. The
/// mutation-aware reader has to run on the constructor body too, with the delegation's bound
/// arguments still in scope.
#[test]
fn a_constructor_that_mutates_before_returning_is_read_through_the_delegation() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub limit: u32, pub tags: Vec<String> }

                impl Prefs {
                    pub fn with_limit(limit: u32) -> Self {
                        let mut prefs = Self { limit: 0, tags: Vec::new() };
                        prefs.limit = limit;
                        prefs.tags.push("seed".to_string());
                        prefs
                    }
                }

                impl Default for Prefs {
                    fn default() -> Self { Self::with_limit(7) }
                }
            "#,
        "Prefs",
        &[("limit", TypeRef::Unit), ("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_eq!(
        resolved,
        vec![
            ("limit".to_string(), DefaultValue::IntLiteral(7)),
            (
                "tags".to_string(),
                DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral("seed".to_string())]),
            ),
        ],
        "the delegated constructor's mutations count exactly as the default's own would"
    );
}

/// `return prefs;` is the same tail as a bare `prefs`, and reading the literal there was wrong
/// for the same reason.
#[test]
fn an_explicit_return_of_the_binding_is_read_like_a_bare_tail() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 0 };
                        prefs.max_depth = 3;
                        return prefs;
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(3))],
        "an explicit return of the binding does not hide the mutations before it"
    );
}

/// A type annotation on the binding changes the pattern shape but not the value.
#[test]
fn a_type_annotated_binding_is_read_the_same_as_an_untyped_one() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs: Self = Self { max_depth: 0 };
                        prefs.max_depth = 5;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(5))],
        "`let prefs: Self = ..` binds the same value as `let prefs = ..`"
    );
}

/// A binding with no mutations at all is exactly the literal, and must keep working.
#[test]
fn a_binding_returned_without_mutations_keeps_the_literal_values() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let prefs = Self { max_depth: 4 };
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(4))],
        "no mutation means the literal is the whole answer"
    );
}

/// Guard against fixing the mutation case by breaking the ordinary one: a plain tail struct
/// literal is unchanged.
#[test]
fn a_plain_tail_struct_literal_is_unaffected() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self { Self { max_depth: 1 } }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(1))],
        "the tail-literal reading must survive the mutation-aware reader"
    );
}

/// A `let` that is *not* the returned value, followed by a tail struct literal, still reads the
/// tail literal — the binding is dead code as far as the default is concerned.
#[test]
fn a_preceding_unrelated_let_before_a_tail_literal_is_unaffected() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let ceiling = 5;
                        Self { max_depth: 1 }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_eq!(
        resolved,
        vec![("max_depth".to_string(), DefaultValue::IntLiteral(1))],
        "the tail expression is what the function returns, whatever precedes it"
    );
}
