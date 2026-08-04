use super::*;

/// Regression test for a crate-local `type Result<T> = ...<T, E>;` alias where the alias
/// itself carries a generic parameter (`<T>`) — the normal, idiomatic shape of this pattern.
///
/// Functions that return the alias with a single type argument (`Result<Foo>`, relying on
/// the alias to supply the error type) must resolve `error_type` to the alias's real error
/// type (`ConversionError`), not fall back to a placeholder like `anyhow::Error` that gets
/// rendered downstream as a bare `Error` — a type the crate does not export.
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
/// methods resolve their return type through a separate code path (`functions/methods.rs`).
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
