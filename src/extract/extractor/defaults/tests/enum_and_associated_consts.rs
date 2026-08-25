use super::*;

/// The reported defect. `model: Self::DEFAULT_MODEL` is a two-segment path just like
/// `Mode::Fast`, and the extractor lowered both to `EnumVariant`. On a `String` field that
/// rendered as `"default_model"`.
///
/// The const is readable, so the honest answer is not "unresolved" but the const's own value.
#[test]
fn an_associated_const_default_on_a_string_field_resolves_to_the_consts_value() {
    let resolved = defaults_for_typed(
        r#"
                pub struct LlmConfig { pub model: String }

                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
        "LlmConfig",
        &[("model", TypeRef::String)],
    );

    assert_eq!(
        resolved,
        vec![(
            "model".to_string(),
            DefaultValue::StringLiteral("claude-sonnet-4-5".to_string())
        )]
    );
    assert_ne!(
        rendered_python_default("model", TypeRef::String, &resolved[0].1),
        "\"default_model\"",
        "the snake-cased const name is a fabricated value; it must not reach a binding"
    );
}

/// A bare `Self::CONST` — no `.to_string()` — takes the same route.
#[test]
fn a_bare_associated_const_path_resolves_through_the_owning_type() {
    let resolved = defaults_for_typed(
        r#"
                pub struct LlmConfig { pub base_url: String }

                impl LlmConfig {
                    const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { base_url: LlmConfig::DEFAULT_BASE_URL.into() }
                    }
                }
            "#,
        "LlmConfig",
        &[("base_url", TypeRef::String)],
    );

    assert_eq!(
        resolved,
        vec![(
            "base_url".to_string(),
            DefaultValue::StringLiteral("https://api.anthropic.com".to_string())
        )]
    );
}

/// The same shape with the const out of reach — declared in another module, or not a string
/// literal at all. There is no value to recover, so the answer is `Unresolved`. What it must
/// never be is an `EnumVariant`, because the field's declared type cannot hold one and the
/// renderer would invent `"default_model"` from the const's name.
#[test]
fn an_unreachable_associated_const_on_a_string_field_is_unresolved_not_an_enum_variant() {
    let resolved = defaults_for_typed(
        r#"
                pub struct LlmConfig { pub model: String }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
        "LlmConfig",
        &[("model", TypeRef::String)],
    );

    let value = &resolved[0].1;
    assert!(
        matches!(value, DefaultValue::Unresolved(_)),
        "an unreadable initializer must be reported, got {value:?}"
    );
    assert_ne!(
        value,
        &DefaultValue::EnumVariant("DEFAULT_MODEL".to_string()),
        "a `String` field cannot hold an enum variant, so this lowering was never sound"
    );
    assert_ne!(
        rendered_python_default("model", TypeRef::String, value),
        "\"default_model\"",
        "the fabricated snake-cased const name must be absent from generated output"
    );
}

/// The control for the fix: a two-segment path on a field that really is enum-typed must
/// still lower to `EnumVariant`. Breaking this would make every genuine enum default
/// unresolved and arm the refusal across the fleet.
#[test]
fn a_genuine_enum_variant_default_still_lowers_to_an_enum_variant() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub mode: Mode, pub fallback: Option<Mode>, pub stages: Vec<Mode> }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            mode: Mode::Fast,
                            fallback: Some(Mode::Slow),
                            stages: vec![Mode::Fast, Mode::Slow],
                        }
                    }
                }
            "#,
        "Cfg",
        &[
            ("mode", TypeRef::Named("Mode".to_string())),
            (
                "fallback",
                TypeRef::Optional(Box::new(TypeRef::Named("Mode".to_string()))),
            ),
            ("stages", TypeRef::Vec(Box::new(TypeRef::Named("Mode".to_string())))),
        ],
    );

    assert_eq!(
        resolved,
        vec![
            ("mode".to_string(), DefaultValue::EnumVariant("Fast".to_string())),
            ("fallback".to_string(), DefaultValue::EnumVariant("Slow".to_string())),
            (
                "stages".to_string(),
                DefaultValue::ListLiteral(vec![
                    DefaultValue::EnumVariant("Fast".to_string()),
                    DefaultValue::EnumVariant("Slow".to_string()),
                ])
            ),
        ],
        "an enum-typed field — bare, optional or in a list — must keep its variant default"
    );
}

/// Two types in one module may each declare a const of the same name. Keying the index by
/// the owning type is what stops one from answering for the other, which would substitute a
/// value that is wrong rather than merely missing.
#[test]
fn an_associated_const_of_another_type_does_not_answer_for_this_one() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Other { pub model: String }
                pub struct LlmConfig { pub model: String }

                impl Other {
                    pub const DEFAULT_MODEL: &str = "not-this-one";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
        "LlmConfig",
        &[("model", TypeRef::String)],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "a same-named const on a different type must not be substituted; got {resolved:?}"
    );
}

/// The field-granular half of the `Empty`/`Unresolved` split: an initializer alef cannot read
/// inside an otherwise-readable struct literal. Each of these previously wrote `Empty`, which
/// licensed every backend to emit its own type-zero for a value it had never read.
#[test]
fn an_unreadable_field_initializer_is_unresolved_not_empty() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg {
                    pub threshold: f32,
                    pub name: String,
                    pub root: PathBuf,
                    pub window: [u32; 2],
                    pub mode: u8,
                }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            threshold: compute().clamp(0.0, 1.0),
                            name: make_name(1, 2),
                            root: PathBuf::from("/tmp"),
                            window: [1, 2],
                            mode: if cfg!(unix) { 1 } else { 2 },
                        }
                    }
                }
            "#,
        "Cfg",
        &["threshold", "name", "root", "window", "mode"],
    );

    for (name, value) in &resolved {
        assert!(
            matches!(value, DefaultValue::Unresolved(_)),
            "`{name}` is not readable, so it must be reported rather than zeroed; got {value:?}"
        );
    }
}

/// The control that must survive the relabelling. `Empty` still means "the default *is* this
/// type's zero", and these three initializers still assert exactly that. Widening
/// `Unresolved` over them would arm the refusal on every crate in the fleet.
#[test]
fn genuine_type_zero_initializers_stay_empty() {
    let resolved = defaults_for(
        r#"
                pub struct Cfg {
                    pub tags: Vec<String>,
                    pub index: AHashMap<String, u32>,
                    pub count: u32,
                    pub stages: Vec<String>,
                }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            tags: Vec::new(),
                            index: AHashMap::new(),
                            count: u32::default(),
                            stages: vec![],
                        }
                    }
                }
            "#,
        "Cfg",
        &["tags", "index", "count", "stages"],
    );

    assert_eq!(
        resolved,
        vec![
            ("tags".to_string(), DefaultValue::Empty),
            ("index".to_string(), DefaultValue::Empty),
            ("count".to_string(), DefaultValue::Empty),
            ("stages".to_string(), DefaultValue::Empty),
        ],
        "a known type-zero must stay `Empty`; only an unread value becomes `Unresolved`"
    );
}

/// The dominant unreadable-default shape in the consumer crates, measured across every repo
/// with an `alef.toml`: a field initialized from a module-level const of non-`&str` type.
/// Nine of the eighteen would-be-unresolved fields fleet-wide are exactly this, and two of
/// them already ship as `0` in generated Python against Rust values of `1024` and `6`. ~keep
#[test]
fn a_module_const_of_any_literal_type_resolves_to_its_value() {
    let resolved = defaults_for(
        r#"
                const DEFAULT_DETECTION_LIMIT_SIDE_LEN: u32 = 1024;
                const DEFAULT_RECOGNITION_BATCH_SIZE: usize = 6;
                const DEFAULT_DB_THRESH: f32 = 0.3;
                const DEFAULT_VERBOSE: bool = true;

                pub struct PaddleOcrConfig {
                    pub det_limit_side_len: u32,
                    pub rec_batch_num: usize,
                    pub det_db_thresh: f32,
                    pub verbose: bool,
                }

                impl Default for PaddleOcrConfig {
                    fn default() -> Self {
                        Self {
                            det_limit_side_len: DEFAULT_DETECTION_LIMIT_SIDE_LEN,
                            rec_batch_num: DEFAULT_RECOGNITION_BATCH_SIZE,
                            det_db_thresh: DEFAULT_DB_THRESH,
                            verbose: DEFAULT_VERBOSE,
                        }
                    }
                }
            "#,
        "PaddleOcrConfig",
        &["det_limit_side_len", "rec_batch_num", "det_db_thresh", "verbose"],
    );

    assert_eq!(
        resolved,
        vec![
            ("det_limit_side_len".to_string(), DefaultValue::IntLiteral(1024)),
            ("rec_batch_num".to_string(), DefaultValue::IntLiteral(6)),
            ("det_db_thresh".to_string(), DefaultValue::FloatLiteral(0.3)),
            ("verbose".to_string(), DefaultValue::BoolLiteral(true)),
        ],
        "a numeric module const is readable; substituting the type-zero for it is the same \
             fabrication as substituting one for an unread default"
    );
}

/// A variant may be named through any number of module segments. Only the last segment is the
/// variant, and stopping at exactly two made three fleet-wide enum defaults unreadable.
#[test]
fn a_fully_qualified_enum_path_still_lowers_to_its_last_segment() {
    let resolved = defaults_for_typed(
        r#"
                pub struct ExtractionConfig { pub result_format: ResultFormat }

                impl Default for ExtractionConfig {
                    fn default() -> Self {
                        Self { result_format: crate::types::ResultFormat::Unified }
                    }
                }
            "#,
        "ExtractionConfig",
        &[("result_format", TypeRef::Named("ResultFormat".to_string()))],
    );

    assert_eq!(
        resolved,
        vec![(
            "result_format".to_string(),
            DefaultValue::EnumVariant("Unified".to_string())
        )]
    );
}

/// `Cow` is a representation the binding layer already erases via `FieldDef::core_wrapper`,
/// so the value is whatever it wraps. Reading through it is not a guess, and refusing to
/// would turn a field that generates the correct `""` today into a generation error.
#[test]
fn a_cow_wrapped_literal_resolves_to_the_literal_it_wraps() {
    let resolved = defaults_for_typed(
        r#"
                pub struct ProcessConfig { pub language: Cow<'static, str>, pub tag: Cow<'static, str> }

                impl Default for ProcessConfig {
                    fn default() -> Self {
                        Self {
                            language: Cow::Borrowed(""),
                            tag: std::borrow::Cow::Borrowed("stable"),
                        }
                    }
                }
            "#,
        "ProcessConfig",
        &[("language", TypeRef::String), ("tag", TypeRef::String)],
    );

    assert_eq!(
        resolved,
        vec![
            ("language".to_string(), DefaultValue::StringLiteral(String::new())),
            ("tag".to_string(), DefaultValue::StringLiteral("stable".to_string())),
        ]
    );
}

/// The boundary the `Cow` reading must not cross: a `Cow` around something alef cannot read
/// is still unread.
#[test]
fn a_cow_wrapping_an_unreadable_expression_stays_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct ProcessConfig { pub language: Cow<'static, str> }

                impl Default for ProcessConfig {
                    fn default() -> Self {
                        Self { language: Cow::Owned(detect_language()) }
                    }
                }
            "#,
        "ProcessConfig",
        &[("language", TypeRef::String)],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "got {resolved:?}"
    );
}

#[test]
fn collect_literal_consts_indexes_associated_consts_under_their_owning_type() {
    let file: syn::File = syn::parse_str(
        r#"
                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
                    pub const MAX_TOKENS: u32 = 4096;
                }
                impl Default for LlmConfig {
                    const NOT_A_CONSTRUCTOR: &str = "trait-impl";
                    fn default() -> Self { Self {} }
                }
                #[cfg(test)]
                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "test-only";
                }
            "#,
    )
    .expect("valid file");

    let consts = collect_literal_consts(&file.items);

    assert_eq!(
        consts.get("LlmConfig::DEFAULT_MODEL"),
        Some(&DefaultValue::StringLiteral("claude-sonnet-4-5".to_string())),
        "a `#[cfg(test)]` impl must not shadow the real associated const"
    );
    assert_eq!(
        consts.get("LlmConfig::MAX_TOKENS"),
        Some(&DefaultValue::IntLiteral(4096))
    );
    assert_eq!(
        consts
            .get("Default::NOT_A_CONSTRUCTOR")
            .or(consts.get("LlmConfig::NOT_A_CONSTRUCTOR")),
        None,
        "trait-impl associated consts are not inherent consts of the type"
    );
}
