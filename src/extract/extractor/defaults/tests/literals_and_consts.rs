use super::*;

#[test]
fn some_int_literal_unwraps_to_inner_int() {
    assert_eq!(
        default_value_of("Some(50 * 1024 * 1024)"),
        DefaultValue::IntLiteral(52_428_800)
    );
}

#[test]
fn some_string_literal_unwraps_to_inner_string() {
    assert_eq!(
        default_value_of(r#"Some("hi".to_string())"#),
        DefaultValue::StringLiteral("hi".to_string())
    );
}

#[test]
fn qualified_option_some_unwraps() {
    assert_eq!(default_value_of("Option::Some(5)"), DefaultValue::IntLiteral(5));
}

#[test]
fn bare_none_stays_none() {
    assert_eq!(default_value_of("None"), DefaultValue::None);
}

#[test]
fn zero_argument_function_call_preserves_its_path() {
    assert_eq!(
        default_value_of("defaults::retry_limit()"),
        DefaultValue::FunctionCall("defaults::retry_limit".to_string())
    );
}

#[test]
fn const_to_string_resolves_to_the_consts_literal_value() {
    assert_eq!(
        default_value_of_with_consts(
            "DEFAULT_CATALOG_URL.to_string()",
            &[("DEFAULT_CATALOG_URL", "https://example.com/catalog.json")]
        ),
        DefaultValue::StringLiteral("https://example.com/catalog.json".to_string())
    );
}

#[test]
fn const_into_resolves_to_the_consts_literal_value() {
    assert_eq!(
        default_value_of_with_consts("HOST.into()", &[("HOST", "localhost")]),
        DefaultValue::StringLiteral("localhost".to_string())
    );
}

#[test]
fn bare_const_path_resolves_to_the_consts_literal_value() {
    assert_eq!(
        default_value_of_with_consts("HOST", &[("HOST", "localhost")]),
        DefaultValue::StringLiteral("localhost".to_string())
    );
}

#[test]
fn unresolvable_const_reference_is_unresolved_not_empty() {
    // No matching entry in `literal_consts`: alef does not know the value. `Empty` would
    // assert the default *is* the empty string, which for a const named `UNKNOWN_CONST` it
    // demonstrably is not.
    assert!(
        matches!(
            default_value_of("UNKNOWN_CONST.to_string()"),
            DefaultValue::Unresolved(_)
        ),
        "an unresolvable const reference must be reported, not silently zeroed"
    );
}

#[test]
fn collect_literal_consts_collects_every_literal_kind_and_nothing_computed() {
    let file: syn::File = syn::parse_str(
        r#"
                pub const DEFAULT_CATALOG_URL: &str = "https://example.com/catalog.json";
                const CACHE_DIR_NAME: &str = "sample-crate";
                const RETRY_LIMIT: u32 = 3;
                const DET_DB_THRESH: f32 = 0.3;
                const VERBOSE: bool = false;
                const MIN_OFFSET: i32 = -5;
                const COMPUTED: &str = some_fn();
                const WINDOW: Duration = Duration::from_secs(5);
            "#,
    )
    .expect("valid file");

    let consts = collect_literal_consts(&file.items);

    assert_eq!(
        consts.get("DEFAULT_CATALOG_URL"),
        Some(&DefaultValue::StringLiteral(
            "https://example.com/catalog.json".to_string()
        ))
    );
    assert_eq!(
        consts.get("CACHE_DIR_NAME"),
        Some(&DefaultValue::StringLiteral("sample-crate".to_string()))
    );
    // A numeric const is exactly as readable as a string one, and leaving it out made alef
    // render `0` for the single most common unreadable-default shape in the consumer crates.
    assert_eq!(consts.get("RETRY_LIMIT"), Some(&DefaultValue::IntLiteral(3)));
    assert_eq!(consts.get("DET_DB_THRESH"), Some(&DefaultValue::FloatLiteral(0.3)));
    assert_eq!(consts.get("VERBOSE"), Some(&DefaultValue::BoolLiteral(false)));
    assert_eq!(consts.get("MIN_OFFSET"), Some(&DefaultValue::IntLiteral(-5)));
    assert_eq!(
        consts.get("COMPUTED"),
        None,
        "non-literal initializers must not be collected"
    );
    assert_eq!(
        consts.get("WINDOW"),
        None,
        "evaluating a const-fn initializer would be interpretation, not reading"
    );
}

/// `pub const ONE: Weight = Weight(1);` — a single-argument tuple-struct literal — folds to
/// the literal it wraps. `DefaultValue` has no struct-shaped variant, so this is the only way
/// `Self::ONE` inside `impl Default for Weight` can ever resolve to anything but `Unresolved`.
#[test]
fn collect_literal_consts_folds_a_tuple_struct_literal_to_its_inner_scalar() {
    let file: syn::File = syn::parse_str(
        r#"
                impl Weight {
                    pub const ONE: Weight = Weight(1);
                    pub const MAX: Weight = Weight(u32::MAX);
                }
            "#,
    )
    .expect("valid file");

    let consts = collect_literal_consts(&file.items);

    assert_eq!(consts.get("Weight::ONE"), Some(&DefaultValue::IntLiteral(1)));
    assert_eq!(
        consts.get("Weight::MAX"),
        None,
        "the inner expression `u32::MAX` is not itself a literal; guessing its value is worse \
             than leaving the const unindexed"
    );
}

/// A lower-case callee is a function by Rust convention, not a tuple-struct constructor, so
/// folding `compute(3)` the same way as `Weight(1)` would be interpreting code, not reading
/// a literal.
#[test]
fn collect_literal_consts_does_not_fold_a_lowercase_call_as_a_tuple_struct_literal() {
    let file: syn::File = syn::parse_str(r#"const RETRY_LIMIT: u32 = compute(3);"#).expect("valid file");

    let consts = collect_literal_consts(&file.items);

    assert_eq!(
        consts.get("RETRY_LIMIT"),
        None,
        "a snake_case call is a function invocation and must not be folded"
    );
}
