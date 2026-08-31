//! `Self { #[cfg(feature = "x")] limit: 9 }` used to be refused wholesale, purely because the
//! initializer carried an attribute at all — the expression was never even evaluated. That made
//! `unreadable_field_default` fire on every consumer with a `cfg`-gated config field, which is not
//! exotic: it is the ordinary way to spell an optional-feature field. The refusal's ground was
//! that the initializer is supplied only in some builds, but the field's declaration must carry
//! the identical gate or the other build would not compile, so wherever the field exists this is
//! the only initializer that could have supplied it.
//!
//! These are the failing repro and the regression guard. Each `_resolves` case reports
//! `Unresolved` before the fix; the `_stays_unresolved` cases are the two shapes where the
//! compile-or-gate argument genuinely does not hold and the refusal is kept.
use super::*;

#[test]
fn a_cfg_gated_int_literal_initializer_resolves() {
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
        &["max_depth", "width"],
    );

    assert_eq!(
        resolved,
        vec![
            ("max_depth".to_string(), DefaultValue::IntLiteral(9)),
            ("width".to_string(), DefaultValue::IntLiteral(2)),
        ],
        "a cfg-gated initializer with a matching field-declaration gate is deterministic wherever \
         the field exists, and must read exactly as its bare counterpart does"
    );
}

#[test]
fn a_cfg_gated_none_initializer_resolves() {
    let resolved = defaults_for(
        r#"
                pub struct Config {
                    #[cfg(feature = "pdf")]
                    pub pdf_options: Option<u32>,
                }

                impl Default for Config {
                    fn default() -> Self {
                        Self {
                            #[cfg(feature = "pdf")]
                            pdf_options: None,
                        }
                    }
                }
            "#,
        "Config",
        &["pdf_options"],
    );

    assert_eq!(
        resolved,
        vec![("pdf_options".to_string(), DefaultValue::None)],
        "a bare `None` under a matching `cfg` is exactly as readable as an ungated `None`"
    );
}

#[test]
fn a_cfg_gated_nested_type_default_resolves_to_empty() {
    let resolved = defaults_for(
        r#"
                pub struct Config {
                    #[cfg(feature = "svg")]
                    pub svg: SvgOptions,
                }

                impl Default for Config {
                    fn default() -> Self {
                        Self {
                            #[cfg(feature = "svg")]
                            svg: SvgOptions::default(),
                        }
                    }
                }
            "#,
        "Config",
        &["svg"],
    );

    assert_eq!(
        resolved,
        vec![("svg".to_string(), DefaultValue::Empty)],
        "`Type::default()` is the type's zero by definition regardless of the cfg gating its field"
    );
}

/// A cfg-gated initializer that calls a zero-arg function alef cannot fold must resolve exactly
/// as the identical *un-attributed* initializer already does elsewhere (see
/// `zero_argument_function_call_preserves_its_path` in `literals_and_consts.rs`): `FunctionCall`,
/// not `Unresolved`. `FunctionCall`'s own downstream handling —
/// `codegen::config_gen::validate_rust_default_functions` for the backends that need to preserve
/// it exactly, and the source-`Deserialize` recovery in `default_value_for_field_in_type` — is
/// unrelated to whether the field happened to be cfg-gated, so this pass must not invent a
/// stricter rule for the cfg-gated spelling than the bare one already gets.
#[test]
fn a_cfg_gated_function_call_initializer_resolves_to_function_call() {
    let resolved = defaults_for(
        r#"
                pub struct Config {
                    #[cfg(feature = "remote")]
                    pub remote: RemoteConfig,
                }

                impl Default for Config {
                    fn default() -> Self {
                        Self {
                            #[cfg(feature = "remote")]
                            remote: Config::default_remote_config(),
                        }
                    }
                }
            "#,
        "Config",
        &["remote"],
    );

    assert_eq!(
        resolved,
        vec![(
            "remote".to_string(),
            DefaultValue::FunctionCall("Config::default_remote_config".to_string())
        )],
        "a cfg-gated unfoldable call must read exactly as the bare version does; got {resolved:?}"
    );
}

/// `#[cfg_attr(..)]` and any other non-`cfg` attribute on a struct-literal initializer keep the
/// wholesale refusal: the attribute's effect on the initializer's very presence is not knowable
/// from source, for the same reason an attributed mutation statement is refused
/// (`defaults::mutation`'s module doc).
#[test]
fn a_cfg_attr_gated_struct_literal_initializer_stays_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs {
                    pub max_depth: u32,
                }

                impl Default for Prefs {
                    fn default() -> Self {
                        Self {
                            #[cfg_attr(feature = "extras", allow(unused))]
                            max_depth: 9,
                        }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert!(
        matches!(resolved.as_slice(), [(name, DefaultValue::Unresolved(_))] if name == "max_depth"),
        "cfg_attr hides an arbitrary attribute behind a condition and must stay refused; got {resolved:?}"
    );
}

/// Two initializers for the same field name compile only because they are mutually exclusive
/// `cfg` arms (`target_arch = "wasm32"` vs. its negation) — legal Rust because exactly one arm
/// survives `cfg`-stripping in any single build. Alef must still pick one value as *the*
/// documented default for every non-wasm backend's generated doc comment and per-field literal.
///
/// Policy: prefer the arm that is not positively gated on `target_arch = "wasm32"` — the native
/// value — because alef's wasm backend already has its own mechanism for fields that differ under
/// wasm32 (`core::config::languages::wasm`), so every other backend should quote the native
/// default rather than whichever arm happened to be inserted into a hash map last.
#[test]
fn duplicate_wasm32_cfg_arms_prefer_the_native_value() {
    let resolved = defaults_for(
        r#"
                pub struct TesseractConfig {
                    pub psm: i32,
                }

                impl Default for TesseractConfig {
                    fn default() -> Self {
                        Self {
                            #[cfg(target_arch = "wasm32")]
                            psm: 6,
                            #[cfg(not(target_arch = "wasm32"))]
                            psm: 3,
                        }
                    }
                }
            "#,
        "TesseractConfig",
        &["psm"],
    );

    assert_eq!(
        resolved,
        vec![("psm".to_string(), DefaultValue::IntLiteral(3))],
        "the native (non-wasm32) arm must win, deterministically, not whichever arm parses last"
    );
}

/// The same pair with source order reversed must resolve identically — the policy is about which
/// `cfg` predicate wins, not about position in the struct literal.
#[test]
fn duplicate_wasm32_cfg_arms_prefer_the_native_value_regardless_of_source_order() {
    let resolved = defaults_for(
        r#"
                pub struct TesseractConfig {
                    pub psm: i32,
                }

                impl Default for TesseractConfig {
                    fn default() -> Self {
                        Self {
                            #[cfg(not(target_arch = "wasm32"))]
                            psm: 3,
                            #[cfg(target_arch = "wasm32")]
                            psm: 6,
                        }
                    }
                }
            "#,
        "TesseractConfig",
        &["psm"],
    );

    assert_eq!(
        resolved,
        vec![("psm".to_string(), DefaultValue::IntLiteral(3))],
        "source order must not decide the winner; got {resolved:?}"
    );
}

/// The readability argument for a `cfg`-gated initializer rests on the literal initializing every
/// field, so the field's declaration must carry the same gate or the other build would not
/// compile. A `..base` rest expression removes that guarantee: `max_depth` below is *ungated*, so
/// in a build without `extras` it takes its value from `base()`, which this pass never read. `9`
/// is then the default in one build and a guess in the other, which is exactly what
/// `unreadable_field_default` exists to refuse.
#[test]
fn a_cfg_gated_initializer_beside_a_rest_base_stays_unresolved() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs {
                    pub max_depth: u32,
                }

                impl Default for Prefs {
                    fn default() -> Self {
                        Self {
                            #[cfg(feature = "extras")]
                            max_depth: 9,
                            ..Prefs::base()
                        }
                    }
                }
            "#,
        "Prefs",
        &["max_depth"],
    );

    assert!(
        matches!(resolved.as_slice(), [(name, DefaultValue::Unresolved(_))] if name == "max_depth"),
        "a rest base can supply the field in the build where the cfg is off, so the gated \
         initializer is not the only source and must not be read; got {resolved:?}"
    );
}

/// The rest guard is scoped to *gated* initializers. An ungated initializer beside a `..base` is
/// unambiguous — it wins over the base in every build — and must keep reading as it always has.
#[test]
fn an_ungated_initializer_beside_a_rest_base_still_resolves() {
    let resolved = defaults_for(
        r#"
                pub struct Prefs {
                    pub width: u32,
                }

                impl Default for Prefs {
                    fn default() -> Self {
                        Self {
                            width: 2,
                            ..Prefs::base()
                        }
                    }
                }
            "#,
        "Prefs",
        &["width"],
    );

    assert_eq!(
        resolved,
        vec![("width".to_string(), DefaultValue::IntLiteral(2))],
        "an explicit ungated initializer overrides the base in every build; got {resolved:?}"
    );
}
