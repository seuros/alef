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

/// Regression: `[crates.java].exclude_functions` must hide a function from the Java surface
/// while leaving `[crates.ffi].exclude_functions` — and therefore the C ABI and every other
/// binding — untouched. Before this key existed, xberg's `alef.toml` named
/// `embed_sparse_async` under `[crates.java]`, the key was rejected outright, and the
/// function shipped in `Xberg.java`/`XbergRs.java` anyway. Mirrors
/// `GoConfig::exclude_functions`.
#[test]
fn java_exclude_functions_hides_the_function_without_touching_the_ffi_list() {
    use alef::core::ir::{FunctionDef, TypeRef};

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
exclude_functions = ["embed_sparse_async"]
"#,
    )
    .unwrap();
    let resolved = config.resolve().unwrap().remove(0);

    let make_fn = |name: &str| FunctionDef {
        name: name.to_string(),
        return_type: TypeRef::Unit,
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "sample_core".into(),
        version: "1.0.0".into(),
        functions: vec![make_fn("embed_sparse_async"), make_fn("other_func")],
        ..Default::default()
    };

    let files = JavaBackend.generate_bindings(&api, &resolved).unwrap();
    let joined = files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !joined.contains("embedSparseAsync"),
        "JavaConfig::exclude_functions must drop the function from every generated Java file:\n{joined}"
    );
    assert!(
        joined.contains("otherFunc"),
        "a function named in neither exclude list must still be generated:\n{joined}"
    );
}
