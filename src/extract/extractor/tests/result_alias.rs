use super::*;

/// Regression test for a crate-local `type Result<T> = ...<T, E>;` alias where the alias
/// itself carries a generic parameter (`<T>`) — the normal, idiomatic shape of this pattern.
///
/// Functions that return the alias with a single type argument (`Result<Foo>`, relying on
/// the alias to supply the error type) must resolve `error_type` to the alias's real error
/// type (`ConversionError`), not fall back to a placeholder like `anyhow::Error` that gets
/// rendered downstream as a bare `Error` — a type the crate does not export. ~keep
#[test]
fn test_generic_result_alias_supplies_real_error_type() {
    let source = r#"
        pub struct ConversionError;

        pub struct ConversionResult;

        pub type Result<T> = std::result::Result<T, ConversionError>;

        pub fn convert(html: &str) -> Result<ConversionResult> {
            unimplemented!()
        }
    "#;

    let surface = extract_from_source(source);
    let convert = surface.functions.iter().find(|f| f.name == "convert").unwrap();
    assert_eq!(
        convert.error_type.as_deref(),
        Some("ConversionError"),
        "generic Result<T> alias must resolve error_type from its own definition, got: {:?}",
        convert.error_type
    );
}

/// Same as above, but for a method on an `impl` block rather than a free function, since
/// methods resolve their return type through a separate code path (`functions/methods.rs`). ~keep
#[test]
fn test_generic_result_alias_supplies_real_error_type_for_method() {
    let source = r#"
        pub struct ConversionError;

        pub struct ConversionResult;

        pub type Result<T> = std::result::Result<T, ConversionError>;

        pub struct Converter;

        impl Converter {
            pub fn convert(&self, html: &str) -> Result<ConversionResult> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let converter = surface.types.iter().find(|t| t.name == "Converter").unwrap();
    let convert = converter.methods.iter().find(|m| m.name == "convert").unwrap();
    assert_eq!(
        convert.error_type.as_deref(),
        Some("ConversionError"),
        "generic Result<T> alias must resolve error_type from its own definition, got: {:?}",
        convert.error_type
    );
}

/// The alias and the function that returns it normally live in *different* modules
/// (`error.rs` declares `Result`, `convert_api.rs` returns it). Extraction walks one module at a
/// time, so a per-module hint map that replaces rather than accumulates loses the alias before the
/// function is resolved — the single-module cases above pass while every real crate still renders
/// the placeholder `Error`. ~keep
#[test]
fn test_result_alias_resolves_when_declared_in_a_different_module() {
    let source = r#"
        pub struct ConversionError;

        pub struct ConversionResult;

        pub type Result<T> = std::result::Result<T, ConversionError>;

        pub mod convert_api {
            use super::{ConversionResult, Result};

            pub fn convert(html: &str) -> Result<ConversionResult> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let convert = surface.functions.iter().find(|f| f.name == "convert").unwrap();
    assert_eq!(
        convert.error_type.as_deref(),
        Some("ConversionError"),
        "alias declared in a sibling module must still supply the error type, got: {:?}",
        convert.error_type
    );
}

/// Source shape of a real crate: a canonical `Result` next to the crate error plus a private
/// `Result` inside a format subsystem, with the plugin traits declared in a third module that
/// imports `crate::Result`.
///
/// A hint map keyed by alias *name* makes the last alias walked win for the whole crate, so the
/// plugin traits pick up the subsystem's private error type — a type the crate does not re-export,
/// which then lands in generated bindings as an unresolvable import.
const CRATE_WITH_PRIVATE_SUBSYSTEM_ALIAS: &str = r#"
        pub mod error {
            pub struct SampleCrateError;
            pub type Result<T> = std::result::Result<T, SampleCrateError>;
        }

        pub mod extraction {
            pub mod binary {
                pub mod error {
                    pub struct BinaryFormatError;
                    pub type Result<T> = std::result::Result<T, BinaryFormatError>;
                }

                pub mod model {
                    use super::error::Result;

                    pub fn parse_header(bytes: &[u8]) -> Result<u32> {
                        unimplemented!()
                    }
                }
            }
        }

        pub mod plugins {
            use crate::Result;

            pub struct Embedding;

            pub trait EmbeddingBackend {
                fn embed(&self, texts: Vec<String>) -> Result<Embedding>;
            }
        }

        pub use error::{Result, SampleCrateError};
    "#;

#[test]
fn test_trait_method_uses_the_canonical_alias_not_a_module_private_one() {
    let surface = extract_from_source(CRATE_WITH_PRIVATE_SUBSYSTEM_ALIAS);
    let backend = surface
        .types
        .iter()
        .find(|t| t.name == "EmbeddingBackend")
        .expect("trait must be extracted");
    let embed = backend.methods.iter().find(|m| m.name == "embed").unwrap();
    assert_eq!(
        embed.error_type.as_deref(),
        Some("SampleCrateError"),
        "a trait importing crate::Result must resolve to the crate's exported error type, got: {:?}",
        embed.error_type
    );
}

#[test]
fn test_module_private_alias_still_applies_inside_its_own_subsystem() {
    let surface = extract_from_source(CRATE_WITH_PRIVATE_SUBSYSTEM_ALIAS);
    let parse_header = surface
        .functions
        .iter()
        .find(|f| f.name == "parse_header")
        .expect("subsystem function must be extracted");
    assert_eq!(
        parse_header.error_type.as_deref(),
        Some("BinaryFormatError"),
        "a module importing its own subsystem alias keeps that alias's error type, got: {:?}",
        parse_header.error_type
    );
}

/// A module that returns `anyhow::Result<T>` must keep `anyhow::Error`; substituting the crate's
/// own error type there would be the same defect in the opposite direction.
#[test]
fn test_foreign_result_alias_is_not_replaced_by_the_crate_error() {
    let source = r#"
        pub mod error {
            pub struct SampleCrateError;
            pub type Result<T> = std::result::Result<T, SampleCrateError>;
        }

        pub mod scripting {
            use anyhow::Result;

            pub struct Script;

            pub fn compile(source: &str) -> Result<Script> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let compile = surface.functions.iter().find(|f| f.name == "compile").unwrap();
    assert_eq!(
        compile.error_type.as_deref(),
        Some("anyhow::Error"),
        "anyhow::Result must not be rewritten to the crate error type, got: {:?}",
        compile.error_type
    );
}
