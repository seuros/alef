//! The `_to_rust_<snake>` converter's native-constructor call and that constructor's own
//! signature must agree on the exact keyword set.
//!
//! `**({"field": value} if value is not None else {})` hides the keyword from the checker: pyrefly
//! then tries every remaining parameter against the unpacked value and reports one
//! `[bad-argument-type]` per pair, so N such unpacks in one call cost N*(N-1) errors in a dozen
//! lines. The omission is only meaningful when the public field can actually be absent, which is
//! exactly when `options.py` defaults it to `None` -- a `#[serde(default)]` enum field renders
//! `= "start"` there and can never be absent. ~keep

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{
    ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef,
};

const CONFIG_TYPE: &str = "LayoutSpec";
const SERDE_DEFAULT: &str = "/* serde(default) */";
const ENUM_FIELDS: [(&str, &str, &str, &str); 3] = [
    ("alignment", "Alignment", "Start", "End"),
    ("density", "Density", "Loose", "Tight"),
    ("casing", "Casing", "Lower", "Upper"),
];

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

fn unit_enum(name: &str, default_variant: &str, other_variant: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        variants: vec![
            EnumVariant {
                name: default_variant.to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: other_variant.to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `#[serde(default)]` on a non-`Option` enum field: `options.py` renders it as the enum's
/// `#[default]` variant string, so the public field is never `None`.
fn serde_default_enum_field(field_name: &str, enum_name: &str) -> FieldDef {
    FieldDef {
        name: field_name.to_string(),
        ty: TypeRef::Named(enum_name.to_string()),
        default: Some(SERDE_DEFAULT.to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    }
}

/// `#[serde(default = "some_fn")]`: alef cannot render the function's value as a Python literal,
/// so `options.py` defaults the field to `None` and the field genuinely can be absent.
fn function_default_field(field_name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: field_name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        typed_default: Some(DefaultValue::FunctionCall("test_lib::default_theme".to_string())),
        ..Default::default()
    }
}

fn surface(config_fields: Vec<FieldDef>, enums: Vec<EnumDef>, extra_types: Vec<TypeDef>) -> ApiSurface {
    let mut types = vec![TypeDef {
        name: CONFIG_TYPE.to_string(),
        rust_path: format!("test_lib::{CONFIG_TYPE}"),
        has_serde: true,
        has_default: true,
        fields: config_fields,
        ..Default::default()
    }];
    types.extend(extra_types);
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types,
        enums,
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "test_lib::render".to_string(),
            params: vec![ParamDef {
                name: "spec".to_string(),
                ty: TypeRef::Named(CONFIG_TYPE.to_string()),
                ..Default::default()
            }],
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

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

/// The argument list of `return _rust.<CONFIG_TYPE>(` in the rendered facade, one entry per
/// top-level comma.
fn constructor_call_arguments(facade: &str) -> Vec<String> {
    let marker = format!("return _rust.{CONFIG_TYPE}(");
    let start = facade
        .find(&marker)
        .map(|idx| idx + marker.len())
        .unwrap_or_else(|| panic!("`{marker}` is missing from:\n{facade}"));
    split_top_level(&balanced_slice(&facade[start..], facade))
}

/// The parameter names of `<CONFIG_TYPE>.__init__` in the rendered `.pyi`, `self` excluded.
fn stub_constructor_parameters(stub: &str) -> Vec<String> {
    let class_marker = format!("\nclass {CONFIG_TYPE}:");
    let class_start = stub
        .find(&class_marker)
        .unwrap_or_else(|| panic!("`class {CONFIG_TYPE}:` is missing from:\n{stub}"));
    let init_marker = "def __init__(";
    let init_start = stub[class_start..]
        .find(init_marker)
        .map(|idx| class_start + idx + init_marker.len())
        .unwrap_or_else(|| panic!("`{CONFIG_TYPE}.__init__` is missing from:\n{stub}"));
    split_top_level(&balanced_slice(&stub[init_start..], stub))
        .into_iter()
        .map(|entry| entry.split(':').next().unwrap_or_default().trim().to_string())
        .filter(|name| name.as_str() != "self")
        .collect()
}

/// Everything up to the paren that closes an already-opened call.
fn balanced_slice(rest: &str, whole: &str) -> String {
    let mut depth = 1usize;
    let mut inner = String::new();
    for ch in rest.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return inner;
                }
            }
            _ => {}
        }
        inner.push(ch);
    }
    panic!("unbalanced call parentheses in:\n{whole}");
}

fn split_top_level(inner: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    entries.push(current.trim().to_string());
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        entries.push(current.trim().to_string());
    }
    entries
}

fn enum_field_surface() -> ApiSurface {
    surface(
        ENUM_FIELDS
            .iter()
            .map(|(field, enum_name, _, _)| serde_default_enum_field(field, enum_name))
            .collect(),
        ENUM_FIELDS
            .iter()
            .map(|(_, enum_name, default_variant, other)| unit_enum(enum_name, default_variant, other))
            .collect(),
        Vec::new(),
    )
}

/// Every `#[serde(default)]` enum field is passed as a plain keyword argument, spelled exactly.
#[test]
fn should_pass_serde_default_enum_fields_as_plain_kwargs_when_options_never_defaults_them_to_none() {
    let (facade, _stub) = render_facade_and_stub(&enum_field_surface());
    let arguments = constructor_call_arguments(&facade);

    let expected: Vec<String> = ENUM_FIELDS
        .iter()
        .map(|(field, enum_name, _, _)| format!("{field}=_coerce_enum(_rust.{enum_name}, value.{field})"))
        .collect();
    assert_eq!(
        arguments, expected,
        "each serde(default) enum field must be passed by keyword, not hidden behind a \
         `**({{...}} if ... else {{}})` unpack:\n{facade}"
    );
}

/// The unpack form must not appear at all for this shape -- asserting only that the plain form is
/// present would pass while an unpack was emitted alongside it.
#[test]
fn should_not_emit_a_kwargs_unpack_when_no_field_can_be_absent() {
    let (facade, _stub) = render_facade_and_stub(&enum_field_surface());
    let arguments = constructor_call_arguments(&facade);

    let unpacks: Vec<&String> = arguments.iter().filter(|entry| entry.starts_with("**")).collect();
    assert_eq!(
        unpacks,
        Vec::<&String>::new(),
        "a field `options.py` defaults to a real value can never be absent, so the omission \
         unpack is dead code that costs one pyrefly [bad-argument-type] per other unpack in the \
         same call:\n{facade}"
    );
}

/// The keyword set the converter passes and the keyword set the native constructor declares must
/// be the same set -- an unpack removes a keyword from the former without removing the parameter.
#[test]
fn constructor_call_and_native_constructor_signature_agree_on_the_parameter_set() {
    let (facade, stub) = render_facade_and_stub(&enum_field_surface());

    let mut called: Vec<String> = constructor_call_arguments(&facade)
        .into_iter()
        .map(|entry| entry.split('=').next().unwrap_or_default().trim().to_string())
        .collect();
    called.sort();
    let mut declared = stub_constructor_parameters(&stub);
    declared.sort();

    assert_eq!(
        called, declared,
        "the `_to_rust_*` constructor call and the native `__init__` must name the same \
         parameters:\nfacade:\n{facade}\nstub:\n{stub}"
    );
}

/// The omission unpack is still emitted where it is load-bearing: a field whose Rust default is a
/// function call has no Python literal, so `options.py` defaults it to `None` and the field really
/// can be absent.
#[test]
fn should_keep_the_kwargs_unpack_when_options_defaults_the_field_to_none() {
    let theme = TypeDef {
        name: "Theme".to_string(),
        rust_path: "test_lib::Theme".to_string(),
        ..Default::default()
    };
    let api = surface(vec![function_default_field("theme", "Theme")], Vec::new(), vec![theme]);
    let (facade, _stub) = render_facade_and_stub(&api);
    let arguments = constructor_call_arguments(&facade);

    assert_eq!(
        arguments,
        vec![r#"**({"theme": value.theme} if value.theme is not None else {})"#.to_string()],
        "a field `options.py` defaults to `None` must stay omittable -- passing `None` to a \
         non-`Option` pyo3 parameter fails extraction:\n{facade}"
    );
}
