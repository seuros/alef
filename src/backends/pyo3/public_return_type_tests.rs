//! A wrapper's declared return type must name the type the package publishes under that word.
//!
//! `_rust` is the private extension module. When `options.py` defines a return type itself -- it
//! does for every `is_return_type` config type under the `typed-dict` output style -- that
//! definition is what `__init__.py` re-exports and what a consumer imports, so a wrapper
//! annotated `-> _rust.<Name>` publishes one name for two different types. The value has to move
//! with the annotation: the extension module still hands back its own `#[pyclass]`, so the
//! wrapper converts it, exactly as an adapter wrapper already does for a public `options` type. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, TypeDef, TypeRef};

const RETURN_TYPE: &str = "RenderOutcome";
const FUNCTION_NAME: &str = "render";

fn python_config(dto_section: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
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
{dto_section}
"#
    ))
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

fn typed_dict_config() -> ResolvedCrateConfig {
    python_config("\n[crates.dto]\npython = \"typed-dict\"\n")
}

fn dataclass_config() -> ResolvedCrateConfig {
    python_config("")
}

/// One `has_default` return-type struct and one function returning it.
fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: RETURN_TYPE.to_string(),
            rust_path: format!("test_lib::{RETURN_TYPE}"),
            has_serde: true,
            has_default: true,
            is_return_type: true,
            fields: vec![
                FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                FieldDef {
                    name: "width".to_string(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: FUNCTION_NAME.to_string(),
            rust_path: format!("test_lib::{FUNCTION_NAME}"),
            params: Vec::new(),
            return_type: TypeRef::Named(RETURN_TYPE.to_string()),
            ..Default::default()
        }],
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

/// The published name and the annotated name must be the same name.
#[test]
fn should_annotate_the_options_type_when_options_publishes_the_return_type() {
    let (api_py, options_py, init_py) = render_public_api(&typed_dict_config());

    assert!(
        options_py.contains(&format!("class {RETURN_TYPE}(TypedDict, total=False):")),
        "the fixture must reach the case under test -- options.py has to publish {RETURN_TYPE}:\n{options_py}"
    );
    assert!(
        init_py.contains(&format!("from .options import {RETURN_TYPE}")),
        "__init__.py must be re-exporting the options definition:\n{init_py}"
    );
    assert!(
        api_py.contains(&format!("def {FUNCTION_NAME}() -> {RETURN_TYPE}:")),
        "the wrapper must be annotated with the published name:\n{api_py}"
    );
}

/// Asserting only that the public spelling appears would pass while the private one was emitted
/// alongside it, so assert the private spelling is absent.
#[test]
fn should_not_name_the_private_extension_module_in_a_published_return_annotation() {
    let (api_py, _options_py, _init_py) = render_public_api(&typed_dict_config());

    assert!(
        !api_py.contains(&format!("_rust.{RETURN_TYPE}")),
        "`_rust.{RETURN_TYPE}` is the private extension module's class, not the type the package \
         publishes under that name:\n{api_py}"
    );
}

/// The annotation is only true if the value matches it: the native call still yields a
/// `#[pyclass]`, so the wrapper has to convert.
#[test]
fn should_convert_the_native_return_value_into_the_published_type() {
    let (api_py, options_py, _init_py) = render_public_api(&typed_dict_config());

    assert!(
        options_py.contains("def _from_native_render_outcome("),
        "options.py must define the converter that builds the published type:\n{options_py}"
    );
    assert!(
        api_py.contains(&format!(
            "    return _from_native_render_outcome(_rust.{FUNCTION_NAME}())\n"
        )),
        "api.py must convert the native return value before handing it back:\n{api_py}"
    );
    assert!(
        api_py.contains("_from_native_render_outcome,") || api_py.contains("_from_native_render_outcome\n"),
        "api.py must import the converter from .options:\n{api_py}"
    );
}

/// The private spelling stays where it is correct: under the dataclass output style `options.py`
/// publishes no `RenderOutcome`, so the native `#[pyclass]` is the only type with that name.
#[test]
fn should_keep_the_private_module_name_when_options_publishes_no_such_type() {
    let (api_py, options_py, init_py) = render_public_api(&dataclass_config());

    assert!(
        !options_py.contains(&format!("class {RETURN_TYPE}")),
        "the fixture must reach the case under test -- options.py must not publish {RETURN_TYPE}:\n{options_py}"
    );
    assert!(
        init_py.contains(&format!("from ._test_lib import {RETURN_TYPE}")),
        "__init__.py must re-export the native class here:\n{init_py}"
    );
    assert!(
        api_py.contains(&format!("def {FUNCTION_NAME}() -> _rust.{RETURN_TYPE}:")),
        "the wrapper must keep naming the native class:\n{api_py}"
    );
}
