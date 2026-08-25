use super::*;

/// The reported defect, reduced. `PaddleOcrConfig` really is
/// `impl Default { fn default() -> Self { Self::new("en") } }`, and before this the
/// extractor wrote `Empty` to all seven fields, which C#, Java, Kotlin, Swift, Python and
/// Go each rendered as their own type-zero — `0.0f` for `det_db_thresh`, sitting under a
/// generated doc comment reading "(default: 0.3)". ~keep
#[test]
fn a_default_delegating_to_a_constructor_recovers_the_constructors_literals() {
    let resolved = defaults_for(
        r#"
                pub struct PaddleOcrConfig {
                    pub language: String,
                    pub det_db_thresh: f32,
                    pub det_limit_side_len: u32,
                    pub use_angle_cls: bool,
                }

                impl PaddleOcrConfig {
                    pub fn new(language: &str) -> Self {
                        Self {
                            language: language.to_string(),
                            det_db_thresh: 0.3,
                            det_limit_side_len: 1024,
                            use_angle_cls: true,
                        }
                    }
                }

                impl Default for PaddleOcrConfig {
                    fn default() -> Self {
                        Self::new("en")
                    }
                }
            "#,
        "PaddleOcrConfig",
        &["language", "det_db_thresh", "det_limit_side_len", "use_angle_cls"],
    );

    assert_eq!(
        resolved,
        vec![
            ("language".to_string(), DefaultValue::StringLiteral("en".to_string())),
            ("det_db_thresh".to_string(), DefaultValue::FloatLiteral(0.3)),
            ("det_limit_side_len".to_string(), DefaultValue::IntLiteral(1024)),
            ("use_angle_cls".to_string(), DefaultValue::BoolLiteral(true)),
        ],
        "a delegating `fn default()` must yield the constructor's literals, never a type-zero"
    );
}

/// The same recovery through the type's own name rather than `Self`, and through a
/// constructor whose parameter is consumed by `.into()` rather than `.to_string()`.
#[test]
fn a_delegation_named_by_the_type_and_consumed_by_into_also_recovers() {
    let resolved = defaults_for(
        r#"
                pub struct Client { pub endpoint: String, pub retries: u32 }

                impl Client {
                    pub fn for_endpoint(endpoint: &str) -> Self {
                        Self { endpoint: endpoint.into(), retries: 5 }
                    }
                }

                impl Default for Client {
                    fn default() -> Self {
                        Client::for_endpoint("https://api.example.com")
                    }
                }
            "#,
        "Client",
        &["endpoint", "retries"],
    );

    assert_eq!(
        resolved,
        vec![
            (
                "endpoint".to_string(),
                DefaultValue::StringLiteral("https://api.example.com".to_string())
            ),
            ("retries".to_string(), DefaultValue::IntLiteral(5)),
        ]
    );
}

/// A delegation whose argument is a module const, resolved through the same const index
/// the direct path already uses.
#[test]
fn a_delegation_passing_a_module_const_resolves_it() {
    let resolved = defaults_for(
        r#"
                const DEFAULT_LANG: &str = "en";

                pub struct Ocr { pub language: String }

                impl Ocr {
                    pub fn new(language: &str) -> Self {
                        Self { language: language.to_string() }
                    }
                }

                impl Default for Ocr {
                    fn default() -> Self { Self::new(DEFAULT_LANG) }
                }
            "#,
        "Ocr",
        &["language"],
    );

    assert_eq!(
        resolved,
        vec![("language".to_string(), DefaultValue::StringLiteral("en".to_string()))]
    );
}

/// Two hops, which the delegation follower is bounded to allow.
#[test]
fn a_delegation_chained_through_a_second_constructor_still_resolves() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new() -> Self { Self::with_level(7) }
                    pub fn with_level(level: u32) -> Self { Self { level } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
        "Cfg",
        &["level"],
    );

    assert_eq!(resolved, vec![("level".to_string(), DefaultValue::IntLiteral(7))]);
}

/// A mutually recursive constructor pair must terminate rather than blow the stack, and
/// must report the failure instead of inventing values.
#[test]
fn a_cyclic_delegation_terminates_and_reports_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new() -> Self { Self::fresh() }
                    pub fn fresh() -> Self { Self::new() }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
        "Cfg",
        &["level"],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "a cycle must resolve to `Unresolved`, got {resolved:?}"
    );
}

/// The honest boundary of the technique, pinned so nobody mistakes the fold for an
/// interpreter. A constructor that *computes* a field is not followed, and the outcome is
/// `Unresolved` — reported — rather than a type-zero.
#[test]
fn a_default_delegating_to_a_builder_is_unresolved_not_a_type_zero() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn builder() -> CfgBuilder { CfgBuilder::new() }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::builder().level(9).build() }
                }
            "#,
        "Cfg",
        &["level"],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "an unfollowable body must be reported, not silently zeroed; got {resolved:?}"
    );
    assert_ne!(
        resolved[0].1,
        DefaultValue::Empty,
        "`Empty` would claim the default *is* the type-zero, which is the conflation this fixes"
    );
}

/// The general shape the reported `Weight` warning is an instance of: a single-field
/// newtype (`pub struct Weight(pub u32)`) plus `pub const ONE: Weight = Weight(1);` plus
/// `impl Default { fn default() -> Self { Self::ONE } }`. `Self::ONE` is neither a struct
/// literal nor a delegation call — it names an associated const of `Self` — but `Weight` has
/// exactly one field and the const's tuple-struct literal folds all the way down to `1`.
///
/// Note: the *inner field* must be `pub` for a real `pub struct Weight(u32)` to reach this
/// path through the full extractor — `extract_struct` (`extract::extractor::types`) only
/// extracts a single-unnamed-field tuple struct's field when `is_pub` holds for that field's
/// own visibility, not just the struct's. A consumer newtype whose inner field is *private*
/// is extracted as a fully opaque type with zero `FieldDef`s — the warning
/// still fires (this fn's `Unresolved` fallback logs unconditionally), but it is inert there:
/// `unreadable_field_default_diagnostics` iterates `typ.fields`, which is empty, so no
/// diagnostic and no wrong-`0` ever reaches a backend for that specific type today. This test
/// exercises `extract_default_values` directly (as every sibling test in this file does,
/// bypassing `extract_struct`'s field-visibility gate) to prove the fold works for the shape
/// that *does* reach a backend: any single-field newtype whose one field is `pub`. ~keep
#[test]
fn a_single_field_types_associated_const_tail_folds_to_the_consts_scalar() {
    let resolved = defaults_for(
        r#"
                pub struct Weight(pub u32);

                impl Weight {
                    pub const ONE: Weight = Weight(1);
                }

                impl Default for Weight {
                    fn default() -> Self {
                        Self::ONE
                    }
                }
            "#,
        "Weight",
        &["_0"],
    );

    assert_eq!(
        resolved,
        vec![("_0".to_string(), DefaultValue::IntLiteral(1))],
        "a foldable associated-const tail must recover the real default, not `Unresolved` and not `0`"
    );
}

/// Negative control for the same shape: when the const's own initializer is not itself
/// foldable (`Weight(u32::MAX)` — the inner expression is a path, not a literal), the field
/// must stay `Unresolved`. Without this, the fix above could pass by making every
/// `Self::NAME` tail fold to *something*, which is the failure mode that matters most here.
#[test]
fn an_associated_consts_unfoldable_inner_value_stays_unresolved_not_a_type_zero() {
    let resolved = defaults_for(
        r#"
                pub struct Weight(pub u32);

                impl Weight {
                    pub const MAX: Weight = Weight(u32::MAX);
                }

                impl Default for Weight {
                    fn default() -> Self {
                        Self::MAX
                    }
                }
            "#,
        "Weight",
        &["_0"],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "an unfoldable const initializer must be reported, not guessed; got {resolved:?}"
    );
    assert_ne!(
        resolved[0].1,
        DefaultValue::IntLiteral(0),
        "collapsing to a zero would be silently wrong: `u32::MAX` is not `0`"
    );
}

/// The direct path is untouched: a `fn default()` that spells its own struct literal still
/// reads exactly as before, including the per-field `Empty` for an initializer that is
/// genuinely the type's zero.
#[test]
fn a_struct_literal_default_is_unchanged_and_keeps_empty_for_genuine_zeros() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub level: u32, pub tags: Vec<String> }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { level: 3, tags: Vec::new() }
                    }
                }
            "#,
        "Cfg",
        &["level", "tags"],
    );

    assert_eq!(
        resolved,
        vec![
            ("level".to_string(), DefaultValue::IntLiteral(3)),
            ("tags".to_string(), DefaultValue::Empty),
        ]
    );
}

/// An arity mismatch means the constructor index resolved something other than the
/// function actually called; reading its body would invent values.
#[test]
fn a_delegation_with_mismatched_arity_is_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new(level: u32, name: &str) -> Self { Self { level } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new(4) }
                }
            "#,
        "Cfg",
        &["level"],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "got {resolved:?}"
    );
}

/// A constructor parameter that is not a foldable literal must not be bound: binding a
/// placeholder would put a guessed value in a field that reads the parameter. The field that
/// *does* read it is reported unresolved rather than zeroed; its sibling is untouched.
#[test]
fn a_delegation_with_an_unfoldable_argument_reports_only_the_field_that_reads_it() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg { pub name: String, pub level: u32 }

                impl Cfg {
                    pub fn new(name: &str) -> Self { Self { name: name.to_string(), level: 2 } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new(compute_name()) }
                }
            "#,
        "Cfg",
        &["name", "level"],
    );

    assert!(
        matches!(
            resolved.as_slice(),
            [
                (name, DefaultValue::Unresolved(_)),
                (level, DefaultValue::IntLiteral(2)),
            ] if name == "name" && level == "level"
        ),
        "the unfoldable argument must not poison the sibling field it does not reach, and the \
             field it does reach must be reported rather than zeroed; got {resolved:?}"
    );
}

#[test]
fn collect_constructors_indexes_associated_fns_and_skips_methods_and_trait_impls() {
    let file: syn::File = syn::parse_str(
        r#"
                impl Cfg {
                    pub fn new() -> Self { Self {} }
                    pub fn tweak(&self) -> Self { Self {} }
                }
                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
    )
    .expect("valid file");

    let constructors = collect_constructors(&file.items);

    assert!(constructors.contains_key(&("Cfg".to_string(), "new".to_string())));
    assert!(
        !constructors.contains_key(&("Cfg".to_string(), "tweak".to_string())),
        "a `&self` method cannot be reached by `Self::name(..)` in `fn default()`"
    );
    assert!(
        !constructors.contains_key(&("Cfg".to_string(), "default".to_string())),
        "trait impls must not be indexed as constructors"
    );
}

/// The parameter binding must not leak past the constructor it belongs to: a module const
/// with the same name as a parameter is shadowed inside the callee, and the parameter's
/// bound value is the one that applies.
#[test]
fn a_constructor_parameter_shadows_a_module_const_of_the_same_name() {
    let resolved = defaults_for(
        r#"
                const language: &str = "shadowed";

                pub struct Cfg { pub language: String }

                impl Cfg {
                    pub fn new(language: &str) -> Self { Self { language: language.to_string() } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new("en") }
                }
            "#,
        "Cfg",
        &["language"],
    );

    assert_eq!(
        resolved,
        vec![("language".to_string(), DefaultValue::StringLiteral("en".to_string()))]
    );
}
