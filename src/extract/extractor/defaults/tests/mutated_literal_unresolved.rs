//! The negative half of the mutable-binding reader: every body whose final value cannot be
//! proven must answer [`DefaultValue::Unresolved`], never the initial literal.
//!
//! These are the tests that make the positive ones mean something. A reader that applied
//! mutations but kept trusting the literal whenever it met a shape it did not understand would
//! pass every test in `mutated_literal.rs` and still ship the original defect — the failure
//! mode is not "misses a mutation", it is "reports a value it did not read". Each case below
//! is a shape where the *pre-fix* extractor answered with confident, wrong data.

use super::*;

fn assert_every_field_unresolved(resolved: &[(String, DefaultValue)], reason: &str) {
    assert!(!resolved.is_empty(), "the fixture must resolve at least one field");
    for (name, value) in resolved {
        assert!(
            matches!(value, DefaultValue::Unresolved(_)),
            "`{name}` must be Unresolved because {reason}, got {value:?}"
        );
    }
}

/// `DefaultValue` has no key/value-carrying variant, so a populated map has no representation
/// at all. `Empty` — the pre-fix answer — asserts the map is empty, which is false; a
/// `ListLiteral` of the values would silently drop the keys. The honest answer is neither.
#[test]
fn map_insert_is_unresolved_because_the_ir_cannot_represent_a_populated_map() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub headers: HashMap<String, String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { headers: HashMap::new() };
                        prefs.headers.insert("accept".to_string(), "json".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[(
            "headers",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        )],
    );

    assert_every_field_unresolved(&resolved, "the IR has no populated-map default");
}

/// A set `insert` takes one argument rather than two and is refused for the same reason.
#[test]
fn set_insert_is_unresolved_for_the_same_reason_as_map_insert() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub allowed: HashSet<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { allowed: HashSet::new() };
                        prefs.allowed.insert("alpha".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("allowed", TypeRef::Named("HashSet".to_string()))],
    );

    assert_every_field_unresolved(&resolved, "a populated set has no IR representation");
}

/// The binding is handed to a helper by mutable reference. Whatever that helper does is
/// invisible to this pass, so nothing about the returned value is known any more.
#[test]
fn handing_the_binding_to_a_helper_makes_every_field_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 0 };
                        tune(&mut prefs);
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "the binding escaped into a helper");
}

/// The binding is read on the right-hand side of one of its own field assignments. The result
/// depends on a partially-built value this pass does not model.
#[test]
fn an_assignment_whose_value_reads_the_binding_makes_every_field_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub width: u32, pub height: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { width: 0, height: 3 };
                        prefs.width = scale(&prefs);
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["width", "height"],
    );

    assert_every_field_unresolved(&resolved, "the assignment's value aliases the binding");
}

/// Two branches return two different values. The pre-fix reader picked the one that happened
/// to sit in a `let`.
#[test]
fn a_branch_returning_two_different_values_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let base = Self { max_depth: 1 };
                        if compact() { base } else { Self { max_depth: 2 } }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "the body returns one of two different values");
}

/// An early return means the mutations after it may or may not have run.
#[test]
fn an_early_return_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 1 };
                        if compact() { return prefs; }
                        prefs.max_depth = 2;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "an early return leaves the mutation count unknown");
}

/// A loop pushes an unknown number of unknown elements.
#[test]
fn a_loop_that_pushes_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        for tag in seed_tags() { prefs.tags.push(tag); }
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "a loop pushes an unknown number of elements");
}

/// A mutating method whose effect is not modelled. `clear()` empties the vector the literal
/// filled, so trusting the literal is exactly backwards.
#[test]
fn an_unmodelled_mutating_method_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: vec!["alpha".to_string()] };
                        prefs.tags.clear();
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "`clear` empties the very list the literal filled");
}

/// A compound assignment is arithmetic on a value this pass does not evaluate.
#[test]
fn a_compound_assignment_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 1 };
                        prefs.max_depth += 4;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "a compound assignment is not a modelled mutation");
}

/// Mutating a field of a field reaches into a value whose own shape was never read.
#[test]
fn a_nested_field_assignment_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub limits: Limits }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { limits: Limits::new() };
                        prefs.limits.max_depth = 3;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("limits", TypeRef::Named("Limits".to_string()))],
    );

    assert_every_field_unresolved(&resolved, "the nested value's own shape was never read");
}

/// The nastiest pre-fix reading: a struct literal of an entirely *different* type, sitting in a
/// `let` the function does not return, was scanned backwards into and reported as this type's
/// default.
#[test]
fn a_second_binding_of_another_type_is_never_read_as_this_types_default() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let prefs = Self { max_depth: 1 };
                        let unrelated = Limits { max_depth: 2 };
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "an unreturned binding of another type is not the default");
}

/// A push whose element cannot be folded leaves that field unknown rather than an empty or
/// invented list.
#[test]
fn a_push_of_an_unfoldable_element_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        prefs.tags.push(detect_locale());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "the pushed element could not be folded");
}

/// A `..base` in the literal means the starting value already carried fields this pass never
/// saw, and mutating an unknown starting value cannot make it known.
#[test]
fn a_mutated_literal_with_a_rest_base_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub width: u32, pub height: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { width: 1, ..seed() };
                        prefs.width = 2;
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["width", "height"],
    );

    assert_every_field_unresolved(&resolved, "the literal's `..base` was never read");
}
