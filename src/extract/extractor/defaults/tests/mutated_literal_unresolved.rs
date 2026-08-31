//! The negative half of the mutable-binding reader: every body whose final value cannot be
//! proven must answer [`DefaultValue::Unresolved`], never the initial literal.
//!
//! These are the tests that make the positive ones mean something. A reader that applied
//! mutations but kept trusting the literal whenever it met a shape it did not understand would
//! pass every test in `mutated_literal.rs` and still ship the original defect — the failure
//! mode is not "misses a mutation", it is "reports a value it did not read". Each case below
//! is a shape where the *pre-fix* extractor answered with confident, wrong data.
//!
//! **Every fixture here must be source that `rustc` actually accepts.** `syn` parses strictly
//! more than `rustc` compiles, so a fixture can look fine, parse fine, and still describe a
//! program that cannot exist — in which case the test protects against nothing. Checked against
//! `rustc` 1.98: `#[cfg(..)]` on a bare assignment statement (`prefs.depth = 9;`) is **rejected**
//! with `error[E0658]: attributes on expressions are experimental`, while the same attribute on
//! a method-call statement (`prefs.tags.push(..);`), on a block, on a `let`, or on a
//! struct-literal field is accepted. The cfg fixtures below use only the accepted spellings, and
//! each was compiled under both `--cfg feature="extras"` states to confirm it builds and to read
//! the two runtime values it really produces. ~keep

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

/// Method names alone do not prove collection semantics: a user-defined type may expose
/// `push` and `extend` while preserving invariants or applying transformations Alef cannot see. ~keep
#[test]
fn custom_named_push_and_extend_are_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                #[derive(Default)]
                pub struct Bag(Vec<String>);

                impl Bag {
                    fn push(&mut self, value: String) { self.0.push(value); }
                    fn extend(&mut self, values: Vec<String>) { self.0.extend(values); }
                }

                pub struct Prefs { pub pushed: Bag, pub extended: Bag }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self {
                            pushed: Bag::default(),
                            extended: Bag::default(),
                        };
                        prefs.pushed.push("alpha".to_string());
                        prefs.extended.extend(vec!["beta".to_string()]);
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[
            ("pushed", TypeRef::Named("Bag".to_string())),
            ("extended", TypeRef::Named("Bag".to_string())),
        ],
    );

    assert_every_field_unresolved(
        &resolved,
        "a custom named type's methods do not prove Vec mutation semantics",
    );
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

/// Whether a `cfg`-gated mutation runs depends on the features the *consumer* enables, which is
/// not knowable from the source alef reads. Both directions of the gate are tested because a
/// rule that refuses every attribute is easy to write and a rule that refuses the right ones has
/// to be shown.
///
/// This body compiles in both configurations and produces a *different* value in each: built
/// without `extras` the default is `[]`, built with it the default is `["alpha"]`. Verified by
/// compiling this exact source twice with `rustc`, once per `--cfg` state. One source text, two
/// runtime answers, and alef sees only the text. ~keep
#[test]
fn a_cfg_gated_push_is_unresolved_when_the_feature_would_be_on() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        #[cfg(feature = "extras")]
                        prefs.tags.push("alpha".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "a cfg-gated mutation may or may not exist in a build");
}

/// The cfg-FALSE direction, with the gate negated: built without `extras` the default is
/// `["alpha"]`, built with it the default is `[]` — the mirror image of the test above, and
/// likewise verified by compiling this source under both `--cfg` states. Recording either value
/// would be right in one build and wrong in the other.
#[test]
fn a_cfg_gated_push_is_unresolved_when_the_feature_would_be_off() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        #[cfg(not(feature = "extras"))]
                        prefs.tags.push("alpha".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "a cfg-gated push may or may not exist in a build");
}

/// `cfg_attr` expands to arbitrary attributes under the same unknown condition.
#[test]
fn a_cfg_attr_gated_mutation_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        #[cfg_attr(feature = "extras", allow(unused))]
                        prefs.tags.push("alpha".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "cfg_attr hides an arbitrary attribute behind a feature");
}

/// `#[rustfmt::skip]` is a real, stable attribute that alef does not model. It happens to be
/// inert here — this body yields `["alpha"]` in every configuration — so refusing it costs
/// precision, and that cost is accepted deliberately: an allowlist of "attributes known to be
/// inert" is the part that silently goes stale when a new one appears, and the failure it would
/// admit is a wrong value rather than a missing one. ~keep
#[test]
fn an_unrecognized_attribute_on_a_mutation_is_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Prefs { pub tags: Vec<String> }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { tags: Vec::new() };
                        #[rustfmt::skip]
                        prefs.tags.push("alpha".to_string());
                        prefs
                    }
                }
            "#,
        "Prefs",
        &[("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
    );

    assert_every_field_unresolved(&resolved, "an unmodelled attribute is refused rather than allowlisted");
}

/// The attribute may equally sit on the binding itself. Spelled as a matched pair so both
/// configurations compile — a lone `#[cfg]` on the only `let` would leave the binding undefined
/// in the other build. Built without `extras` the default is `0`, with it `7`.
#[test]
fn an_attributed_local_binding_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        #[cfg(feature = "extras")]
                        let prefs = Self { max_depth: 7 };
                        #[cfg(not(feature = "extras"))]
                        let prefs = Self { max_depth: 0 };
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "the binding itself is cfg-gated");
}

/// A `cfg` on one initializer of the struct literal is the same lie one level down. The field
/// declaration carries the same gate, as it must for both configurations to compile: without
/// `extras` the field does not exist at all, with it the default is `9`. `width` is ungated and
/// stays readable, so the refusal is field-granular rather than whole-body. ~keep
#[test]
fn a_cfg_gated_struct_literal_initializer_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs {
                    #[cfg(feature = "extras")]
                    pub max_depth: u32,
                    pub width: u32,
                }

                impl Default for Prefs {
                    fn default() -> Self {
                        Self {
                            #[cfg(feature = "extras")]
                            max_depth: 9,
                            width: 2,
                        }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "the initializer is supplied only in some builds");
}

/// The tail literal is only *the* answer when nothing before it can return instead. A
/// conditional early return gives the body two exits, and the tail is merely one of them.
#[test]
fn a_tail_literal_after_a_conditional_early_return_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        if compact() {
                            return Self { max_depth: 1 };
                        }
                        Self { max_depth: 2 }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "an earlier exit returns a different value");
}

/// The same gap reached through a binding rather than a second literal.
#[test]
fn a_tail_literal_after_a_conditional_early_return_of_a_binding_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let fallback = Self { max_depth: 1 };
                        if compact() {
                            return fallback;
                        }
                        Self { max_depth: 2 }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "an earlier exit returns a different value");
}

/// A macro before the tail literal is read *past* without being understood, and its expansion
/// may contain the early return the scan would otherwise catch.
#[test]
fn a_macro_statement_before_a_tail_literal_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        return_if_configured!(compact());
                        Self { max_depth: 2 }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "a macro's expansion is not parsed");
}

/// The legal spelling of a cfg-gated assignment: the attribute sits on a block, because Rust
/// rejects it on a bare assignment statement (see this module's header). Refused on shape --
/// a block is not a modelled mutation -- rather than on its attribute, and the answer is the
/// same either way. Built without `extras` the default is `0`, with it `9`.
#[test]
fn a_cfg_gated_block_of_mutations_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs { pub max_depth: u32 }

                impl Default for Prefs {
                    fn default() -> Self {
                        let mut prefs = Self { max_depth: 0 };
                        #[cfg(feature = "extras")]
                        { prefs.max_depth = 9; }
                        prefs
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert_every_field_unresolved(&resolved, "the gated block may or may not run");
}
