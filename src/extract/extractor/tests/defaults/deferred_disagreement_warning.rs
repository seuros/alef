//! Regression coverage for `postprocess::warn_on_default_disagreements`: the disagreement
//! diagnostic must be order-independent across source files, not just within one file.
//!
//! `extract_from_source` (the helper every other test in this module uses) parses a single
//! `syn::File`, so it cannot reproduce a *cross-file* ordering bug — every type and enum in it is
//! always fully known by the time any diagnostic runs. These tests instead drive the real,
//! multi-file crate entry point (`extractor::extract`) with two files written to a temp
//! directory, so the enum genuinely is unresolved in `surface.enums` at the moment the struct's
//! source file is parsed — exactly the shape a real crate hits when (for example) `mod
//! extraction;` is declared, and therefore extracted, before `mod ocr;`.
use std::path::Path;

use tracing_test::traced_test;

use crate::extract::extractor::extract;

/// Writes `sources` (filename, content) pairs to a fresh temp directory, in the given order, and
/// runs the real crate-level extractor over them. The order of `sources` is the order files are
/// parsed in — the exact lever the bug this module guards against depends on.
fn extract_ordered(sources: &[(&str, &str)]) -> crate::core::ir::ApiSurface {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths: Vec<_> = sources
        .iter()
        .map(|(name, content)| {
            let path = dir.path().join(name);
            std::fs::write(&path, content).expect("write source file");
            path
        })
        .collect();
    let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
    extract(&path_refs, "sample_crate", "0.0.0", None).expect("extract failed")
}

/// The struct (and its manual `impl Default`, which sets an enum field directly to
/// `OcrStrategy::Auto`) is parsed from `config.rs`, listed *first*. The enum, with its own
/// `#[derive(Default)]`/`#[default]` making `Auto` its real default, lives in `ocr.rs`, parsed
/// *second* — unresolved in `surface.enums` at the moment `config.rs`'s `impl Default` is read.
///
/// This is a genuine agreement: the field's serde-reader default (`Empty`, from the bare
/// `#[serde(default)]`) and the manual impl's `OcrStrategy::Auto` both name the same value, the
/// enum's own default. No disagreement warning must be logged, no matter which file is listed
/// first.
#[test]
#[traced_test]
fn enum_field_agreement_is_not_reported_when_the_enum_is_extracted_after_the_struct() {
    let config_rs = r#"
        pub struct ExtractionConfig {
            #[serde(default)]
            pub ocr_strategy: OcrStrategy,
        }

        impl Default for ExtractionConfig {
            fn default() -> Self {
                Self {
                    ocr_strategy: OcrStrategy::Auto,
                }
            }
        }
    "#;
    let ocr_rs = r#"
        #[derive(Clone, Copy, Default)]
        pub enum OcrStrategy {
            Never,
            #[default]
            Auto,
            Always,
        }
    "#;

    let surface = extract_ordered(&[("config.rs", config_rs), ("ocr.rs", ocr_rs)]);

    let config = surface
        .types
        .iter()
        .find(|typ| typ.name == "ExtractionConfig")
        .expect("ExtractionConfig must be extracted");
    let ocr_strategy_field = config
        .fields
        .iter()
        .find(|field| field.name == "ocr_strategy")
        .expect("ocr_strategy field must be extracted");
    assert_eq!(
        ocr_strategy_field.typed_default,
        Some(crate::core::ir::DefaultValue::EnumVariant("Auto".to_string())),
        "the field itself must still resolve to the concrete variant regardless of file order"
    );

    assert!(
        !logs_contain("disagrees"),
        "a genuine agreement (the field default names the enum's own #[default] variant) must \
         not be reported as a disagreement just because the enum's source file was parsed after \
         the struct's"
    );
}

/// Control: when the struct and enum swap file order (enum parsed first), the same agreement was
/// already correctly silent before this fix, and must remain silent after it.
#[test]
#[traced_test]
fn enum_field_agreement_is_not_reported_when_the_enum_is_extracted_before_the_struct() {
    let config_rs = r#"
        pub struct ExtractionConfig {
            #[serde(default)]
            pub ocr_strategy: OcrStrategy,
        }

        impl Default for ExtractionConfig {
            fn default() -> Self {
                Self {
                    ocr_strategy: OcrStrategy::Auto,
                }
            }
        }
    "#;
    let ocr_rs = r#"
        #[derive(Clone, Copy, Default)]
        pub enum OcrStrategy {
            Never,
            #[default]
            Auto,
            Always,
        }
    "#;

    let surface = extract_ordered(&[("ocr.rs", ocr_rs), ("config.rs", config_rs)]);

    let config = surface
        .types
        .iter()
        .find(|typ| typ.name == "ExtractionConfig")
        .expect("ExtractionConfig must be extracted");
    let ocr_strategy_field = config
        .fields
        .iter()
        .find(|field| field.name == "ocr_strategy")
        .expect("ocr_strategy field must be extracted");
    assert_eq!(
        ocr_strategy_field.typed_default,
        Some(crate::core::ir::DefaultValue::EnumVariant("Auto".to_string()))
    );

    assert!(!logs_contain("disagrees"));
}

/// A field whose bare `#[serde(default)]` genuinely disagrees with the manual `impl Default` —
/// `Option<u64>::default()` is `None`, not the `Some(60)` the manual impl sets — must still be
/// reported. This is not an enum-lookup case at all (`denotes_type_zero` handles it directly), so
/// deferring the check must not accidentally silence it.
#[test]
#[traced_test]
fn genuine_numeric_disagreement_is_still_reported_after_deferring_the_check() {
    let source = r#"
        pub struct EmbeddingConfig {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub max_embed_duration_secs: Option<u64>,
        }

        impl Default for EmbeddingConfig {
            fn default() -> Self {
                Self {
                    max_embed_duration_secs: Some(60),
                }
            }
        }
    "#;

    let surface = extract_ordered(&[("processing.rs", source)]);

    let config = surface
        .types
        .iter()
        .find(|typ| typ.name == "EmbeddingConfig")
        .expect("EmbeddingConfig must be extracted");
    let field = config
        .fields
        .iter()
        .find(|field| field.name == "max_embed_duration_secs")
        .expect("max_embed_duration_secs field must be extracted");
    assert_eq!(
        field.typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(60)),
        "the field's own resolved default must still be the real Some(60), not suppressed"
    );

    assert!(
        logs_contain("disagrees"),
        "Option<u64>::default() is None, genuinely different from the manual impl's Some(60); \
         this real disagreement must still be reported"
    );
}
