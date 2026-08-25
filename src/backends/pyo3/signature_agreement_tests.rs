//! Agreement tests between the two Python artifacts that describe the same free function:
//! the `api.py` facade (`gen_bindings::functions`) and the `<module>.pyi` stub (`gen_stubs`).
//!
//! The two describe different layers — the facade can construct a `Default` for a parameter the
//! native module still requires — so their *defaults* legitimately differ. Their **parameter
//! order** may not: `api.py` is what `__init__.py` re-exports, so a facade that reorders
//! parameters relative to the stub, the native `#[pyo3(signature = ...)]`, and the Rust source
//! silently swaps every positional call. These tests render both artifacts from one fixture and
//! assert the two agree. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

const FUNCTION_NAME: &str = "render_widget";

fn python_config() -> ResolvedCrateConfig {
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

/// A struct with `#[derive(Default)]` — the shape that makes the facade able to synthesize a
/// value the caller omitted.
fn widget_options() -> TypeDef {
    TypeDef {
        name: "WidgetOptions".to_string(),
        rust_path: "test_lib::WidgetOptions".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

fn surface(params: Vec<ParamDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![widget_options()],
        functions: vec![FunctionDef {
            name: FUNCTION_NAME.to_string(),
            rust_path: format!("test_lib::{FUNCTION_NAME}"),
            params,
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// One rendered parameter: its name and whether the emitted signature gave it a default.
#[derive(Debug, PartialEq, Eq)]
struct RenderedParam {
    name: String,
    default: Option<String>,
}

/// Read the parameter list straight out of rendered Python source, so the assertions describe the
/// artifact a consumer actually installs rather than an intermediate struct. Handles both the
/// single-line and the wrapped multi-line signature shapes both emitters can produce. ~keep
fn rendered_params(source: &str, function_name: &str) -> Vec<RenderedParam> {
    let marker = format!("\ndef {function_name}(");
    let start = source
        .find(&marker)
        .map(|idx| idx + marker.len())
        .unwrap_or_else(|| panic!("`def {function_name}(` is missing from:\n{source}"));
    let mut depth = 1usize;
    let mut inner = String::new();
    for ch in source[start..].chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        inner.push(ch);
    }
    assert_eq!(depth, 0, "unbalanced signature parens in:\n{source}");

    let mut params = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                push_param(&mut params, &current);
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    push_param(&mut params, &current);
    params
}

fn push_param(params: &mut Vec<RenderedParam>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let (name, rest) = trimmed.split_once(':').unwrap_or((trimmed, ""));
    let default = rest.split_once('=').map(|(_, value)| {
        value
            .split_once('#')
            .map_or(value, |(before, _)| before)
            .trim()
            .to_string()
    });
    params.push(RenderedParam {
        name: name.trim().to_string(),
        default,
    });
}

/// Render `api.py` and `<module>.pyi` from one surface.
fn render_facade_and_stub(api: &ApiSurface) -> (String, String) {
    let backend = crate::backends::pyo3::Pyo3Backend;
    let config = python_config();
    let stub = backend
        .generate_type_stubs(api, &config)
        .expect("stub generation succeeds")
        .into_iter()
        .find(|file| file.path.extension().is_some_and(|ext| ext == "pyi"))
        .expect("a .pyi stub is generated")
        .content;
    let facade = backend
        .generate_public_api(api, &config)
        .expect("public API generation succeeds")
        .into_iter()
        .find(|file| file.path.ends_with("api.py"))
        .expect("api.py is generated")
        .content;
    (facade, stub)
}

fn assert_same_parameter_order(
    params: Vec<ParamDef>,
    expected_order: &[&str],
) -> (Vec<RenderedParam>, Vec<RenderedParam>) {
    let api = surface(params);
    let (facade, stub) = render_facade_and_stub(&api);
    let facade_params = rendered_params(&facade, FUNCTION_NAME);
    let stub_params = rendered_params(&stub, FUNCTION_NAME);

    let facade_names: Vec<&str> = facade_params.iter().map(|p| p.name.as_str()).collect();
    let stub_names: Vec<&str> = stub_params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        stub_names, expected_order,
        "the .pyi stub must keep the declared parameter order:\n{stub}"
    );
    assert_eq!(
        facade_names, expected_order,
        "api.py must declare the same parameters in the same order as the .pyi stub — a reordered \
         facade silently swaps every positional call:\n{facade}"
    );
    (facade_params, stub_params)
}

/// The regression: a `Default`-able parameter followed by a required one. The facade used to
/// promote it to `= None` and shuffle it behind the required parameter, so `render_widget(opts,
/// "text")` — the order the Rust source, the native `#[pyo3(signature)]`, and the `.pyi` all
/// declare — bound `opts` to `source`. ~keep
#[test]
fn facade_does_not_reorder_a_defaultable_param_behind_a_required_one() {
    let (facade_params, stub_params) = assert_same_parameter_order(
        vec![
            param("options", TypeRef::Named("WidgetOptions".to_string()), false),
            param("source", TypeRef::String, false),
        ],
        &["options", "source"],
    );
    for (facade, stub) in facade_params.iter().zip(&stub_params) {
        assert_eq!(
            facade.default, None,
            "`{}` cannot carry a default without displacing the required parameter after it",
            facade.name
        );
        assert_eq!(stub.default, None, "`{}` is required in the native module", stub.name);
    }
}

/// A `Default`-able parameter in trailing position may still be defaulted in the facade — the
/// facade constructs the default itself — while the stub keeps it required, because the native
/// module has no such facility. Order still matches.
#[test]
fn facade_may_default_a_trailing_defaultable_param_without_reordering() {
    let (facade_params, stub_params) = assert_same_parameter_order(
        vec![
            param("source", TypeRef::String, false),
            param("options", TypeRef::Named("WidgetOptions".to_string()), false),
        ],
        &["source", "options"],
    );
    assert_eq!(facade_params[0].default, None, "`source` is required in the facade");
    assert_eq!(
        facade_params[1].default.as_deref(),
        Some("None"),
        "a trailing Default-able param stays omissible in the facade"
    );
    assert_eq!(
        stub_params.iter().filter(|p| p.default.is_some()).count(),
        0,
        "the native module requires both parameters"
    );
}

/// A genuinely optional parameter (`Option<T>` in the source) must be defaulted in BOTH
/// artifacts, and so must every parameter after it — PyO3's signature rule.
#[test]
fn a_genuinely_optional_param_is_defaulted_in_both_artifacts() {
    let (facade_params, stub_params) = assert_same_parameter_order(
        vec![
            param("source", TypeRef::Optional(Box::new(TypeRef::String)), true),
            param("limit", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
        ],
        &["source", "limit"],
    );
    for params in [&facade_params, &stub_params] {
        for rendered in params {
            assert_eq!(
                rendered.default.as_deref(),
                Some("None"),
                "`{}` must be defaulted in both artifacts, got {:?}",
                rendered.name,
                rendered.default
            );
        }
    }
}

/// Negative control: with no `Default`-able and no optional parameter, both artifacts must mark
/// every parameter required. Guards against a fix that simply defaults everything.
#[test]
fn a_required_param_stays_required_in_both_artifacts() {
    let (facade_params, stub_params) = assert_same_parameter_order(
        vec![
            param("source", TypeRef::String, false),
            param("limit", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
        ],
        &["source", "limit"],
    );
    for params in [&facade_params, &stub_params] {
        for rendered in params {
            assert_eq!(rendered.default, None, "`{}` must stay required", rendered.name);
        }
    }
}
