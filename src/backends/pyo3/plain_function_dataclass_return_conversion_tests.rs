//! A plain (non-adapter) async function whose return type is a public *input* `@dataclass` --
//! not a return-only `TypedDict` -- must still convert the engine's native return value before
//! handing it back.
//!
//! `emit_function_wrappers`'s return-conversion check (`public_return_leaf` in
//! `function_wrappers.rs`) and `orchestration.rs`'s `function_return_converters` used to be
//! handed only `options_return_types` -- the return-only-`TypedDict` family computed by
//! `options_return_typeddict_names`. `api.py`'s own import classification (`options_type_names`)
//! already consulted the union of that family with `options_dataclass_type_names` (the public
//! *input* dataclass family), so a return type published as an ordinary `@dataclass` -- reachable
//! under the default (non-`typed-dict`) DTO style whenever the type is also a public input type
//! elsewhere -- still got imported from `.options` and named in the `-> ReturnType` annotation,
//! while the wrapper body, unable to find a converter, fell back to
//! `return await _rust.<name>(...)`, the untouched native `#[pyclass]`. The annotation named the
//! public type; the value was the private one. This mirrors the fix `adapter_return_type_tests.rs`
//! proves for the adapter wrapper path -- this file proves the same fix for the plain-function
//! wrapper path. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, TypeDef, TypeRef};

const RETURN_TYPE: &str = "ScanResult";
const FUNCTION_NAME: &str = "scan_target";
const NATIVE_ONLY_TYPE: &str = "ScanHandle";
const NATIVE_FUNCTION_NAME: &str = "open_scan";

fn dataclass_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#,
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

/// `ScanResult` is `has_default` and NOT `is_return_type` -- the shape of a type that
/// `options.py` publishes as a public *input* `@dataclass` (e.g. it is also accepted as a
/// parameter somewhere else in the real surface) which a plain async function also happens to
/// return. `ScanHandle` is native-pyclass-only (no `has_default`): nothing in `options.py` ever
/// publishes it, so its return value must stay unconverted.
fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: RETURN_TYPE.to_string(),
                rust_path: format!("test_lib::{RETURN_TYPE}"),
                has_serde: true,
                has_default: true,
                is_return_type: false,
                fields: vec![FieldDef {
                    name: "url".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: NATIVE_ONLY_TYPE.to_string(),
                rust_path: format!("test_lib::{NATIVE_ONLY_TYPE}"),
                is_opaque: true,
                // No `has_default`, but genuinely `is_return_type` (some function returns it) --
                // the shape of a type `options.py` never publishes at all, so the wrapper must
                // both keep the native `_rust.` prefix on the annotation and skip conversion.
                is_return_type: true,
                ..Default::default()
            },
        ],
        functions: vec![
            FunctionDef {
                name: FUNCTION_NAME.to_string(),
                rust_path: format!("test_lib::{FUNCTION_NAME}"),
                params: Vec::new(),
                return_type: TypeRef::Named(RETURN_TYPE.to_string()),
                is_async: true,
                ..Default::default()
            },
            FunctionDef {
                name: NATIVE_FUNCTION_NAME.to_string(),
                rust_path: format!("test_lib::{NATIVE_FUNCTION_NAME}"),
                params: Vec::new(),
                return_type: TypeRef::Named(NATIVE_ONLY_TYPE.to_string()),
                is_async: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `api.py`, `options.py` and `__init__.py` as the backend writes them.
fn render_public_api(config: &ResolvedCrateConfig) -> (String, String, String) {
    let files = crate::backends::pyo3::Pyo3Backend
        .generate_public_api(&surface(), config)
        .expect("public API generation succeeds");
    let find = |suffix: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} is generated"))
            .content
            .clone()
    };
    (find("api.py"), find("options.py"), find("__init__.py"))
}

/// REPRODUCTION: a plain async function whose return type `options.py` publishes as a public
/// `@dataclass` must convert the engine's native return value before handing it back -- the same
/// requirement an adapter wrapper already gets (`adapter_return_type_tests.rs`).
#[test]
fn a_plain_async_function_converts_its_native_return_value_into_a_dataclass_published_type() {
    let config = dataclass_config();
    let (api_py, options_py, init_py) = render_public_api(&config);

    assert!(
        options_py.contains(&format!("class {RETURN_TYPE}:")),
        "the fixture must reach the case under test -- options.py has to publish {RETURN_TYPE} as a \
         dataclass:\n{options_py}"
    );
    assert!(
        !options_py.contains(&format!("class {RETURN_TYPE}(TypedDict")),
        "the fixture must reach the dataclass case, not the return-only TypedDict case already \
         covered elsewhere:\n{options_py}"
    );
    assert!(
        init_py.contains(&format!("from .options import {RETURN_TYPE}")),
        "__init__.py must re-export the options definition:\n{init_py}"
    );
    assert!(
        api_py.contains(&format!("async def {FUNCTION_NAME}() -> {RETURN_TYPE}:")),
        "the wrapper must be annotated with the published name:\n{api_py}"
    );
    assert!(
        options_py.contains("def _from_native_scan_result("),
        "options.py must define the converter the wrapper calls:\n{options_py}"
    );
    assert!(
        api_py.contains(&format!(
            "    return _from_native_scan_result(await _rust.{FUNCTION_NAME}())\n"
        )),
        "the wrapper must convert the native return value before handing it back:\n{api_py}"
    );
    assert!(
        !api_py.contains(&format!("    return await _rust.{FUNCTION_NAME}()\n")),
        "the wrapper must not hand back the unconverted native value:\n{api_py}"
    );
}

/// CONTROL: a plain async function returning a type `options.py` never publishes at all (a
/// plain, opaque native `#[pyclass]` with no `has_default`) must keep returning the engine's
/// value untouched -- there is no public type to convert into, and applying a converter here
/// would call an undefined name.
#[test]
fn a_plain_async_function_returning_a_type_options_py_never_publishes_stays_unconverted() {
    let config = dataclass_config();
    let (api_py, options_py, _init_py) = render_public_api(&config);

    assert!(
        !options_py.contains(&format!("class {NATIVE_ONLY_TYPE}")),
        "the fixture must reach the case under test -- options.py must not publish {NATIVE_ONLY_TYPE}:\n{options_py}"
    );
    assert!(
        api_py.contains(&format!(
            "async def {NATIVE_FUNCTION_NAME}() -> _rust.{NATIVE_ONLY_TYPE}:"
        )),
        "the wrapper must still be generated and keep naming the native class:\n{api_py}"
    );
    assert!(
        api_py.contains(&format!("    return await _rust.{NATIVE_FUNCTION_NAME}()\n")),
        "with nothing published under that name, the native value must pass through unconverted:\n{api_py}"
    );
    assert!(
        !api_py.contains("_from_native_scan_handle"),
        "no converter exists for a type options.py never publishes:\n{api_py}"
    );
}
