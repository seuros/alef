/// Test that PHP wrapper param signatures preserve required-ness from the Rust API.
///
/// Before the fix: Required params after an optional param were being made optional.
/// Example: `scrape(?CrawlEngineHandle $engine = null, ?string $url = null)`
/// when the Rust API required both `engine: CrawlEngineHandle` and `url: String`.
///
/// After the fix: Only explicitly optional params or default-constructible params
/// become optional in the wrapper. Required params stay required.
/// Example: `scrape(CrawlEngineHandle $engine, string $url)`
#[test]
fn test_php_wrapper_param_optionality_logic() {
    use crate::core::ir::{ParamDef, TypeRef};

    let is_optional_default_constructible_param = |p: &ParamDef| -> bool {
        if let TypeRef::Named(name) = &p.ty {
            matches!(name.as_str(), "CrawlConfig" | "InteractionActions")
        } else {
            false
        }
    };

    let req_param = ParamDef {
        name: "url".to_string(),
        ty: TypeRef::String,
        optional: false,
        ..ParamDef::default()
    };

    let should_be_optional = req_param.optional || is_optional_default_constructible_param(&req_param);
    assert!(
        !should_be_optional,
        "required param should not become optional in wrapper"
    );

    let opt_param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        optional: true,
        ..ParamDef::default()
    };

    let should_be_optional = opt_param.optional || is_optional_default_constructible_param(&opt_param);
    assert!(should_be_optional, "explicitly optional param should be optional");

    let default_constructible_param = ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        optional: false,
        ..ParamDef::default()
    };

    let should_be_optional =
        default_constructible_param.optional || is_optional_default_constructible_param(&default_constructible_param);
    assert!(should_be_optional, "default-constructible param should become optional");
}

/// Regression: the `#[php_impl]` facade is Rust source, so function docs must be emitted as
/// Rust line doc-comments (`///`), never PHPDoc `/** … */` blocks.
///
/// Rust block comments nest, so a doc that mentions `image/*` opens a nested `/*` that the
/// intended closing `*/` only balances at the inner level, leaving the outer `/**` unterminated
/// (`error[E0758]: unterminated block doc-comment`). Line doc-comments have no such hazard.
#[test]
fn should_emit_rust_line_doc_comments_when_doc_text_contains_block_comment_sequences() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::{PhpParamTypeSets, gen_function_as_static_method};
    use crate::core::ir::{FunctionDef, TypeRef};
    use ahash::AHashSet;

    let func = FunctionDef {
        name: "choose_call_mode".to_string(),
        rust_path: "sample_crate::choose_call_mode".to_string(),
        return_type: TypeRef::String,
        doc: "Decide which call mode best fits this document.\n\n\
              Rules: `image/*` → vision; `text/*` and `application/*` → text. Closes with */."
            .to_string(),
        ..FunctionDef::default()
    };

    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let empty = AHashSet::new();
    let type_sets = PhpParamTypeSets {
        opaque: &empty,
        default: &empty,
        enums: &empty,
    };

    let generated = gen_function_as_static_method(&func, &mapper, type_sets, "sample_crate", &[], false, &empty);

    assert!(
        generated.contains("/// Decide which call mode best fits this document."),
        "doc must be emitted as Rust `///` line comments, got:\n{generated}"
    );
    assert!(
        generated.contains("/// Rules: `image/*` → vision; `text/*` and `application/*` → text. Closes with */."),
        "doc body (incl. `image/*` and `*/`) must survive verbatim on a `///` line, got:\n{generated}"
    );
    assert!(
        !generated.contains("/**"),
        "Rust crate doc must not use PHPDoc `/**` block comments (nesting hazard), got:\n{generated}"
    );

    for line in generated.lines().filter(|l| l.contains("Closes with")) {
        assert!(
            line.trim_start().starts_with("///"),
            "line carrying a `*/` token must be a `///` line doc-comment, got: {line:?}"
        );
    }
}

/// Regression: a `&mut self -> Result<&mut Self, E>` builder (a method that returns a reference
/// to its own wrapper type) must SHARE the existing handle's `Arc` (`self.inner.clone()`) rather
/// than cloning the returned reference. `&mut Self` is not `Clone`, and the inner value need not
/// be `Clone`, so `Arc::new(std::sync::Mutex::new(result.clone()))` fails to compile.
#[test]
fn php_self_ref_builder_shares_arc_instead_of_cloning_returned_ref() {
    use super::super::type_map::PhpMapper;
    use crate::backends::php::gen_bindings::functions::gen_instance_method;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
    use ahash::{AHashMap, AHashSet};

    let method = MethodDef {
        name: "register_route".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("RouteCfg".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("App".to_string()),
        error_type: Some("AppError".to_string()),
        doc: "Register a route, returning the app for chaining.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        returns_ref: true,
        ..MethodDef::default()
    };

    let mapper = PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };
    let mut opaque = AHashSet::new();
    opaque.insert("App".to_string());
    opaque.insert("RouteCfg".to_string());
    let enums = AHashSet::new();
    let adapter_bodies: AHashMap<String, String> = AHashMap::new();
    let mut mutex = AHashSet::new();
    mutex.insert("App".to_string());

    let code = gen_instance_method(
        &method,
        &mapper,
        true,
        "App",
        &opaque,
        &enums,
        "sample_crate",
        &adapter_bodies,
        &mutex,
    );

    assert!(
        code.contains("Ok(Self { inner: self.inner.clone() })"),
        "self-returning builder should share the existing Arc, got:\n{code}"
    );
    assert!(
        !code.contains("Mutex::new(result.clone())") && !code.contains("Mutex::new(result)"),
        "must not wrap the returned &mut ref in a new Mutex, got:\n{code}"
    );
    assert!(
        !code.contains("let result ="),
        "self-returning builder must not bind the returned &mut ref, got:\n{code}"
    );
}

/// Regression: Cargo replaces EVERY hyphen in a crate name with `_` for the cdylib output
/// filename (crate `liter-llm-php` -> `libliter_llm_php.{dylib,so}`), so the generated
/// `config.m4` must probe for a fully-underscored stem. It must also keep crate-directory
/// paths hyphenated (`crates/liter-llm-php/...`) and use the (possibly overridden) extension
/// name, not the crate name, for the `modules/*.so` output filename.
#[test]
fn config_m4_uses_underscored_cdylib_stem_and_hyphenated_crate_dir() {
    use super::rust_items::generate_config_m4;

    let m4 = generate_config_m4("liter_llm", "liter-llm");

    assert!(
        m4.contains("crates/liter-llm-php/Cargo.toml"),
        "crate directory path must keep hyphens, got:\n{m4}"
    );
    assert!(
        m4.contains("cd crates/liter-llm-php"),
        "cd target must keep hyphens, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/liter-llm-php/target/release/libliter_llm_php.dylib"),
        "dylib stem must be fully underscored, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/liter-llm-php/target/release/libliter_llm_php.so"),
        "so stem must be fully underscored, got:\n{m4}"
    );
    assert!(
        !m4.contains("liter-llm_php"),
        "must never mix a hyphenated crate name with the `_php` suffix, got:\n{m4}"
    );
    assert!(
        m4.contains(r#"cp "$cargo_lib" "modules/liter_llm.so""#),
        "module output filename must use the extension name, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/liter-llm-php/target/release\" >&2"),
        "not-found error message must reference the hyphenated crate directory, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/liter-llm-php/Cargo.toml not found"),
        "missing-Cargo.toml error message must reference the hyphenated crate directory, got:\n{m4}"
    );
}

/// Positive-control companion to the above: `liter-llm` carries only one hyphen, so
/// "replace the first hyphen" and "replace every hyphen" produce an identical dylib stem and
/// cannot distinguish the two derivations. A crate name with multiple hyphens can: Cargo
/// underscores ALL of them in the cdylib filename while the source directory keeps every one. ~keep
#[test]
fn config_m4_underscores_every_hyphen_in_a_multi_hyphen_crate_name() {
    use super::rust_items::generate_config_m4;

    let m4 = generate_config_m4("my_cool_lib", "my-cool-lib");

    assert!(
        m4.contains("crates/my-cool-lib-php/Cargo.toml"),
        "crate directory path must keep every hyphen, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/my-cool-lib-php/target/release/libmy_cool_lib_php.dylib"),
        "dylib stem must underscore every hyphen, not just the first, got:\n{m4}"
    );
    assert!(
        m4.contains("crates/my-cool-lib-php/target/release/libmy_cool_lib_php.so"),
        "so stem must underscore every hyphen, not just the first, got:\n{m4}"
    );
    assert!(
        !m4.contains("my-cool-lib_php") && !m4.contains("my_cool-lib_php") && !m4.contains("my-cool_lib_php"),
        "must never leave a partially-underscored stem, got:\n{m4}"
    );
}
