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

/// A module that imports `anyhow::Result` for its internal helpers, then writes the crate's own
/// alias fully qualified as `crate::Result<T>` on its public API — the reason to qualify at all is
/// that the bare name is already taken. The qualification is the whole signal, so resolution that
/// reads only the module's `use` statements answers `anyhow::Error` for a function whose source
/// plainly names the crate alias. ~keep
#[test]
fn test_crate_qualified_result_beats_a_foreign_result_import() {
    let source = r#"
        pub mod error {
            pub struct SampleCrateError;
        }

        pub type Result<T> = std::result::Result<T, error::SampleCrateError>;

        pub mod api {
            use anyhow::Result;

            pub struct Extraction;

            pub fn extract(input: &str) -> crate::Result<Extraction> {
                unimplemented!()
            }

            pub fn helper(input: &str) -> Result<u32> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let extract = surface.functions.iter().find(|f| f.name == "extract").unwrap();
    assert_eq!(
        extract.error_type.as_deref(),
        Some("error::SampleCrateError"),
        "crate::Result<T> must resolve through the crate alias, got: {:?}",
        extract.error_type
    );
    let helper = surface.functions.iter().find(|f| f.name == "helper").unwrap();
    assert_eq!(
        helper.error_type.as_deref(),
        Some("anyhow::Error"),
        "the bare imported anyhow::Result must keep anyhow::Error, got: {:?}",
        helper.error_type
    );
}

/// The mirror image, and the negative control for the fix: a module that imports the crate alias
/// but writes `anyhow::Result<T>` fully qualified on one function must keep `anyhow::Error` there.
#[test]
fn test_foreign_qualified_result_is_not_rewritten_to_the_crate_error() {
    let source = r#"
        pub struct SampleCrateError;

        pub type Result<T> = std::result::Result<T, SampleCrateError>;

        pub mod api {
            use crate::Result;

            pub struct Script;

            pub fn compile(source: &str) -> anyhow::Result<Script> {
                unimplemented!()
            }

            pub fn run(source: &str) -> Result<u32> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let compile = surface.functions.iter().find(|f| f.name == "compile").unwrap();
    assert_eq!(
        compile.error_type.as_deref(),
        Some("anyhow::Error"),
        "an inline anyhow::Result must keep anyhow::Error, got: {:?}",
        compile.error_type
    );
    let run = surface.functions.iter().find(|f| f.name == "run").unwrap();
    assert_eq!(
        run.error_type.as_deref(),
        Some("SampleCrateError"),
        "the imported crate alias must still supply the crate error, got: {:?}",
        run.error_type
    );
}

/// `super::error::Result<T>` written inline must pick the subsystem alias it names, not the
/// crate's canonical one.
#[test]
fn test_super_qualified_result_resolves_against_the_parent_module() {
    let source = r#"
        pub mod error {
            pub struct SampleCrateError;
            pub type Result<T> = std::result::Result<T, SampleCrateError>;
        }

        pub mod binary {
            pub mod error {
                pub struct BinaryFormatError;
                pub type Result<T> = std::result::Result<T, BinaryFormatError>;
            }

            pub mod model {
                pub fn parse_header(bytes: &str) -> super::error::Result<u32> {
                    unimplemented!()
                }
            }
        }

        pub use error::{Result, SampleCrateError};
    "#;

    let surface = extract_from_source(source);
    let parse_header = surface.functions.iter().find(|f| f.name == "parse_header").unwrap();
    assert_eq!(
        parse_header.error_type.as_deref(),
        Some("BinaryFormatError"),
        "super::error::Result must resolve to the parent module's alias, got: {:?}",
        parse_header.error_type
    );
}

/// An unqualified `Result<T>` in a module with no `Result` import keeps falling back to the
/// crate's canonical alias — the behavior the qualified-path handling must not disturb.
#[test]
fn test_unqualified_result_still_falls_back_to_the_canonical_alias() {
    let source = r#"
        pub mod error {
            pub struct SampleCrateError;
            pub type Result<T> = std::result::Result<T, SampleCrateError>;
        }

        pub mod api {
            pub fn load(path: &str) -> Result<u32> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let load = surface.functions.iter().find(|f| f.name == "load").unwrap();
    assert_eq!(
        load.error_type.as_deref(),
        Some("SampleCrateError"),
        "an unqualified Result must still reach the canonical alias, got: {:?}",
        load.error_type
    );
}

/// A crate alias that genuinely resolves to `anyhow::Error` must keep answering `anyhow::Error`,
/// even when it is reached through a `crate::Result<T>` qualification.
#[test]
fn test_crate_alias_over_anyhow_error_stays_anyhow() {
    let source = r#"
        pub type Result<T> = std::result::Result<T, anyhow::Error>;

        pub mod api {
            pub struct Report;

            pub fn build() -> crate::Result<Report> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let build = surface.functions.iter().find(|f| f.name == "build").unwrap();
    assert_eq!(
        build.error_type.as_deref(),
        Some("anyhow::Error"),
        "an alias whose error really is anyhow::Error must stay anyhow::Error, got: {:?}",
        build.error_type
    );
}

/// An inline `Result<T, E>` with both parameters spelled out never consults an alias at all.
#[test]
fn test_inline_two_parameter_result_ignores_the_alias() {
    let source = r#"
        pub struct SampleCrateError;
        pub struct ParseError;

        pub type Result<T> = std::result::Result<T, SampleCrateError>;

        pub mod api {
            use crate::{ParseError, Result};

            pub fn parse(input: &str) -> std::result::Result<u32, ParseError> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let parse = surface.functions.iter().find(|f| f.name == "parse").unwrap();
    assert_eq!(
        parse.error_type.as_deref(),
        Some("ParseError"),
        "an explicit two-parameter Result must keep its own error type, got: {:?}",
        parse.error_type
    );
}

/// An alias generic over its *error* parameter has the concrete type only in the parameter's
/// default, so reading the right-hand side literally records the parameter name (`E`) as the error
/// type — a name no backend can resolve to anything. ~keep
#[test]
fn test_alias_generic_over_its_error_parameter_uses_the_default() {
    let source = r#"
        pub struct SampleCrateError;

        pub type Result<T, E = SampleCrateError> = std::result::Result<T, E>;

        pub mod api {
            use crate::Result;

            pub fn load(path: &str) -> Result<u32> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let load = surface.functions.iter().find(|f| f.name == "load").unwrap();
    assert_eq!(
        load.error_type.as_deref(),
        Some("SampleCrateError"),
        "the error parameter's default is the alias's real error type, got: {:?}",
        load.error_type
    );
}

/// The same alias shape without a default has no concrete error type to offer, so it must record
/// no hint at all and let the fallback stand — never the bare parameter name.
#[test]
fn test_alias_generic_over_an_undefaulted_error_parameter_records_no_hint() {
    let source = r#"
        pub type Result<T, E> = std::result::Result<T, E>;

        pub mod api {
            use crate::Result;

            pub fn load(path: &str) -> Result<u32> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let load = surface.functions.iter().find(|f| f.name == "load").unwrap();
    assert_eq!(
        load.error_type.as_deref(),
        Some("anyhow::Error"),
        "a bare generic parameter must never be recorded as an error type, got: {:?}",
        load.error_type
    );
}
