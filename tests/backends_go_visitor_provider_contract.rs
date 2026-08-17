use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

const PROVIDER_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }
"#;

fn options_field_config() -> ResolvedCrateConfig {
    let source = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "example.invalid/test-lib"
"#;
    let config: NewAlefConfig = toml::from_str(source).expect("test config parses");
    let mut resolved = config.resolve().expect("test config resolves").remove(0);
    resolved.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("Options".to_string()),
        options_field: Some("visitor".to_string()),
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
        ..TraitBridgeConfig::default()
    }];
    resolved
}

fn node_context_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "test_lib::NodeContext".to_string(),
            methods: vec![MethodDef {
                name: "to_json".to_string(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            }],
            has_serde: true,
            has_lifetime_params: true,
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn native_library_dir() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(".lib/macos-arm64"),
        ("macos", "x86_64") => Some(".lib/macos-x86_64"),
        ("linux", "x86_64") => Some(".lib/linux-x86_64"),
        ("linux", "aarch64") => Some(".lib/linux-aarch64"),
        _ => None,
    }
}

fn write_empty_native_archive(directory: &std::path::Path, library_dir: &str) -> bool {
    let (Ok(compiler), Ok(archiver)) = (which::which("cc"), which::which("ar")) else {
        return false;
    };
    let object = directory.join("empty.o");
    std::fs::write(directory.join("empty.c"), "void test_link_anchor(void) {}\n").expect("write empty C source");
    let compiled = std::process::Command::new(compiler)
        .args(["-c", "empty.c", "-o", "empty.o"])
        .current_dir(directory)
        .status()
        .expect("run C compiler");
    assert!(compiled.success(), "empty native object compiles");
    let library_dir = directory.join(library_dir);
    std::fs::create_dir_all(&library_dir).expect("create native library directory");
    std::process::Command::new(archiver)
        .arg("rcs")
        .arg(library_dir.join("libtest_ffi.a"))
        .arg(object)
        .status()
        .expect("run native archiver")
        .success()
}

fn assert_real_go_build(binding: &str) {
    let (Some(library_dir), Ok(go)) = (native_library_dir(), which::which("go")) else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary Go package directory");
    std::fs::create_dir_all(directory.path().join("include")).expect("create include directory");
    std::fs::write(directory.path().join("include/test.h"), PROVIDER_HEADER).expect("write provider header");
    std::fs::write(directory.path().join("binding.go"), binding).expect("write generated binding");
    if !write_empty_native_archive(directory.path(), library_dir) {
        return;
    }
    let output = std::process::Command::new(go)
        .args(["build", "./..."])
        .env("GO111MODULE", "off")
        .env("CGO_ENABLED", "1")
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler");
    assert!(
        output.status.success(),
        "generated Go package failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn options_field_associated_types_do_not_call_unprovided_ffi_symbols() {
    let files = GoBackend
        .generate_bindings(&node_context_api(), &options_field_config())
        .expect("Go bindings generate");
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated");

    assert!(
        !binding.content.contains("C.test_node_context_"),
        "options-field associated types are provided by visitor.go and must not bind unexported FFI symbols:\n{}",
        binding.content
    );
    assert_real_go_build(&binding.content);
}
