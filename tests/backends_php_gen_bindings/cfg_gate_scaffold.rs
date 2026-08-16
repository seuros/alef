use alef::core::config::Language;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeRef};
use alef::scaffold::scaffold;

/// Companion to `cfg_gate.rs`'s `php_standalone_function_never_wraps_body_in_cfg`: since PHP
/// facade methods are now always emitted unconditionally (never cfg-gated — ext-php-rs's
/// `#[php_impl]` derive can't tolerate a cfg'd-out method), the underlying core feature(s) those
/// methods depend on must be required unconditionally on the core dependency line, and must NOT
/// be declared as a togglable `[features]` entry on the php crate — a declared feature that no
/// generated code actually gates is its own defect.
#[test]
fn php_cargo_toml_requires_function_gated_core_features_unconditionally_and_never_declares_them_as_toggles() {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "demo_crate"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config must parse");
    let resolved = cfg.resolve().expect("config must resolve");
    let config = &resolved[0];

    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "1.0.0".into(),
        functions: vec![FunctionDef {
            name: "count_tokens".to_string(),
            rust_path: "demo_crate::count_tokens".to_string(),
            params: vec![ParamDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            cfg: Some(r#"feature = "tokenizer""#.to_string()),
            ..FunctionDef::default()
        }],
        ..Default::default()
    };

    let files = scaffold(&api, config, &[Language::Php]).expect("PHP scaffold must succeed");
    let cargo_toml = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("-php/Cargo.toml"))
        .expect("php crate Cargo.toml must be scaffolded");
    let content = &cargo_toml.content;

    assert!(
        content.contains(r#"features = ["tokenizer"]"#),
        "core dependency must unconditionally request the function-gated feature, got:\n{content}"
    );
    assert!(
        !content.contains(r#"tokenizer = ["demo-crate/tokenizer"]"#),
        "function-gated feature must not be declared as a togglable php-crate feature, got:\n{content}"
    );
    assert!(
        !content.contains("default = [\"tokenizer\"]"),
        "function-gated feature must not appear in the php crate's own `default` feature list, got:\n{content}"
    );
}
