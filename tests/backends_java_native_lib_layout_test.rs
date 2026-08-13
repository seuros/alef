use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::ApiSurface;

#[test]
fn required_symbols_are_emitted_one_per_line() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "org.example.sample"
"#,
    )
    .unwrap();
    let resolved = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "sample_core".into(),
        version: "1.0.0".into(),
        ..Default::default()
    };

    let files = JavaBackend.generate_bindings(&api, &resolved).unwrap();
    let native_lib = files
        .iter()
        .find(|file| file.path.ends_with("NativeLib.java"))
        .expect("NativeLib.java");
    let required_symbols = native_lib
        .content
        .lines()
        .skip_while(|line| !line.contains("REQUIRED_SYMBOLS = {"))
        .skip(1)
        .take_while(|line| !line.trim().starts_with("};"))
        .collect::<Vec<_>>();

    assert!(required_symbols.len() > 1, "{}", native_lib.content);
    assert!(required_symbols.iter().all(|line| line.trim().starts_with('"')));
    assert!(required_symbols.iter().all(|line| line.len() <= 200));
}
