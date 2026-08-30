use super::NewAlefConfig;

fn resolve_error_for(toml_source: &str) -> String {
    let config: NewAlefConfig = toml::from_str(toml_source).expect("test config parses");
    config.resolve().expect_err("test config must be rejected").to_string()
}

#[test]
fn resolve_rejects_invalid_derived_ffi_identifiers() {
    let error = resolve_error_for(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "sample.core"
sources = ["src/lib.rs"]
"#,
    );
    assert!(error.contains("effective C-ABI prefix"), "{error}");
}

#[test]
fn resolve_rejects_make_active_derived_ffi_library_name() {
    let error = resolve_error_for(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.output]
ffi = "crates/$(shell-payload)/src"
"#,
    );
    assert!(error.contains("effective C-ABI lib_name"), "{error}");
}

#[test]
fn resolve_rejects_make_active_c_package_path() {
    let error = resolve_error_for(&c_package_config("../../$(shell touch pwned)", None));
    assert!(error.contains("e2e.packages.c.path"), "{error}");
}

#[test]
fn resolve_rejects_repository_escaping_c_package_path() {
    let error = resolve_error_for(&c_package_config("../../../outside", None));
    assert!(error.contains("escapes the repository root"), "{error}");
}

#[test]
fn resolve_rejects_c_call_overrides_that_break_target_grammars() {
    let bad_prefix = resolve_error_for(&c_override_config("e2e.call", "int", "../escape.h"));
    assert!(bad_prefix.contains("e2e.call.overrides.c.prefix"), "{bad_prefix}");
    let bad_header = resolve_error_for(&c_override_config("e2e.call", "sample_core", "../escape.h"));
    assert!(bad_header.contains("e2e.call.overrides.c.header"), "{bad_header}");
}

#[test]
fn resolve_uses_effective_e2e_languages_for_c_validation() {
    let enabled = resolve_error_for(&c_package_config("../../$(shell touch pwned)", Some("c")));
    assert!(enabled.contains("e2e.packages.c.path"), "{enabled}");
    let disabled: NewAlefConfig =
        toml::from_str(&c_package_config("../../$(shell touch pwned)", Some("python"))).expect("test config parses");
    disabled
        .resolve()
        .expect("disabled C e2e config does not reach C emission");
}

#[test]
fn resolve_validates_named_c_call_overrides() {
    let error = resolve_error_for(&c_override_config("e2e.calls.secondary", "int", "sample.h"));
    assert!(error.contains("e2e.calls.secondary.overrides.c.prefix"), "{error}");
}

#[test]
fn resolve_validates_registry_c_package_after_local_resolution() {
    let invalid_name = resolve_error_for(&registry_c_config("test_apps", "bad/name", "../../crates/sample-ffi"));
    assert!(invalid_name.contains("e2e.registry.packages.c.name"), "{invalid_name}");
    let invalid_path = resolve_error_for(&registry_c_config("test_apps", "sample-ffi", "../../../outside"));
    assert!(invalid_path.contains("e2e.registry.packages.c.path"), "{invalid_path}");
}

#[test]
fn resolve_validates_registry_output_after_local_resolution() {
    let error = resolve_error_for(&registry_c_config("../outside", "sample-ffi", "../crates/sample-ffi"));
    assert!(error.contains("e2e.registry.output"), "{error}");
    assert!(error.contains("escapes the repository root"), "{error}");
}

fn c_package_config(path: &str, e2e_language: Option<&str>) -> String {
    let top_language = if e2e_language == Some("c") { "python" } else { "c" };
    let e2e_languages = e2e_language.map_or_else(String::new, |language| format!("languages = [\"{language}\"]"));
    format!(
        r#"
[workspace]
languages = ["{top_language}"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.e2e]
{e2e_languages}
[crates.e2e.call]
function = "sample"
[crates.e2e.packages.c]
path = "{path}"
"#
    )
}

fn c_override_config(call_table: &str, prefix: &str, header: &str) -> String {
    let named_call = if call_table == "e2e.call" {
        String::new()
    } else {
        format!("[crates.{call_table}]\nfunction = \"secondary\"\n")
    };
    format!(
        r#"
[workspace]
languages = ["c"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.e2e.call]
function = "sample"
{named_call}[crates.{call_table}.overrides.c]
prefix = "{prefix}"
header = "{header}"
"#
    )
}

fn registry_c_config(output: &str, name: &str, path: &str) -> String {
    format!(
        r#"
[workspace]
languages = ["c"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.e2e]
output = "e2e"
[crates.e2e.call]
function = "sample"
[crates.e2e.packages.c]
name = "sample-ffi"
path = "../crates/sample-ffi"
[crates.e2e.registry]
output = "{output}"
[crates.e2e.registry.packages.c]
name = "{name}"
path = "{path}"
"#
    )
}
