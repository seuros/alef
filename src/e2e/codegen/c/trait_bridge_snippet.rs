use crate::codegen::naming::{abi_symbol, pascal_to_snake, to_class_name};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
use crate::e2e::fixture::Fixture;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct Callback {
    field: String,
    name: String,
    params: String,
    return_type: String,
    initializers: Vec<String>,
    return_value: String,
}

pub(super) fn render(
    fixture: &Fixture,
    header: &str,
    prefix: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> Result<String> {
    let argument = fixture
        .args
        .iter()
        .find(|argument| argument.arg_type == "test_backend")
        .ok_or_else(|| anyhow::anyhow!("C trait bridge recipe has no test_backend argument"))?;
    let trait_name = argument
        .trait_name
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("C trait bridge recipe has no trait identity"))?;
    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.trait_name == trait_name)
        .ok_or_else(|| anyhow::anyhow!("C trait bridge recipe has no configured bridge for `{trait_name}`"))?;
    let register = bridge
        .register_fn
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("C trait bridge `{trait_name}` has no registration function"))?;
    let trait_def = type_defs
        .iter()
        .find(|definition| definition.name == trait_name)
        .ok_or_else(|| anyhow::anyhow!("C trait bridge recipe has no IR definition for `{trait_name}`"))?;
    let callbacks = callbacks(bridge, trait_def, type_defs);
    let prefix_upper = prefix.to_uppercase();
    let vtable_type = format!("{prefix_upper}{}{}VTable", to_class_name(prefix), trait_name);
    let register_symbol = abi_symbol(prefix, register);
    let unregister_symbol = bridge
        .unregister_fn
        .as_deref()
        .map(|_| abi_symbol(prefix, &format!("unregister_{}", pascal_to_snake(trait_name))));

    Ok(crate::e2e::template_env::render(
        "c/trait_bridge_snippet.jinja",
        minijinja::context! {
            header => header,
            callbacks => callbacks,
            vtable_type => vtable_type,
            register_symbol => register_symbol,
            unregister_symbol => unregister_symbol,
            free_string_symbol => abi_symbol(prefix, "free_string"),
        },
    ))
}

fn callbacks(
    bridge: &TraitBridgeConfig,
    trait_def: &crate::core::ir::TypeDef,
    type_defs: &[crate::core::ir::TypeDef],
) -> Vec<Callback> {
    let mut values = Vec::new();
    if bridge.super_trait.is_some() {
        values.extend(super_callbacks());
    }
    let mut methods: Vec<&MethodDef> = trait_def.methods.iter().collect();
    if let Some(super_trait) = bridge.super_trait.as_deref()
        && let Some(definition) = type_defs.iter().find(|definition| definition.rust_path == super_trait)
    {
        methods.extend(definition.methods.iter());
    }
    for method in methods {
        if method.has_default_impl
            || bridge.ffi_skip_methods.contains(&method.name)
            || method.trait_source.is_some()
            || bridge.super_trait.is_some()
                && matches!(
                    method.name.as_str(),
                    "name" | "version" | "initialize" | "shutdown" | "description" | "author"
                )
        {
            continue;
        }
        values.push(method_callback(method));
    }
    values
}

fn super_callbacks() -> Vec<Callback> {
    vec![
        string_callback("name_fn", "name", "test-backend"),
        string_callback("version_fn", "version", "1.0.0"),
        status_callback("initialize_fn", "initialize"),
        status_callback("shutdown_fn", "shutdown"),
    ]
}

fn string_callback(field: &str, name: &str, value: &str) -> Callback {
    Callback {
        field: field.into(),
        name: format!("sample_{name}"),
        params: format!("const void *user_data, char **out_result, char **out_error"),
        return_type: "int32_t".into(),
        initializers: vec!["(void)user_data;".into(), "*out_error = NULL;".into()],
        return_value: format!(
            "*out_result = sample_copy_string(\"{value}\");\n    return *out_result == NULL ? -1 : 0;"
        ),
    }
}

fn status_callback(field: &str, name: &str) -> Callback {
    Callback {
        field: field.into(),
        name: format!("sample_{name}"),
        params: "const void *user_data, char **out_error".into(),
        return_type: "int32_t".into(),
        initializers: vec!["(void)user_data;".into(), "*out_error = NULL;".into()],
        return_value: "return 0;".into(),
    }
}

fn method_callback(method: &MethodDef) -> Callback {
    let mut params = vec!["const void *user_data".to_string()];
    let mut initializers = vec!["(void)user_data;".to_string()];
    for parameter in &method.params {
        params.push(format!("{} {}", c_type(&parameter.ty), parameter.name));
        initializers.push(format!("(void){};", parameter.name));
        if matches!(parameter.ty, TypeRef::Bytes) {
            params.push(format!("size_t {}_len", parameter.name));
            initializers.push(format!("(void){}_len;", parameter.name));
        }
    }
    let (out_params, return_type) = crate::backends::ffi::trait_bridge::FfiBridgeGenerator::c_return_convention(
        &method.return_type,
        method.error_type.is_some(),
    );
    for parameter in out_params {
        let (name, rust_type) = parameter.split_once(':').expect("FFI out parameter");
        params.push(format!("{} {}", c_rust_type(rust_type.trim()), name));
        initializers.push(format!("*{name} = NULL;"));
    }
    Callback {
        field: method.name.clone(),
        name: format!("sample_{}", pascal_to_snake(&method.name)),
        params: params.join(", "),
        return_type: c_rust_type(&return_type).replace("()", "void"),
        initializers,
        return_value: default_return(&method.return_type, &return_type, method.error_type.is_some()),
    }
}

fn c_type(ty: &TypeRef) -> String {
    c_rust_type(&crate::backends::ffi::trait_bridge::FfiBridgeGenerator::c_param_type(
        ty,
    ))
}

fn c_rust_type(value: &str) -> String {
    match value {
        "*const std::ffi::c_void" => "const void *",
        "*mut std::ffi::c_void" => "void *",
        "*const std::ffi::c_char" => "const char *",
        "*mut *mut std::ffi::c_char" => "char **",
        "*const u8" => "const uint8_t *",
        "usize" => "size_t",
        "isize" => "ptrdiff_t",
        "u8" => "uint8_t",
        "u16" => "uint16_t",
        "u32" => "uint32_t",
        "u64" => "uint64_t",
        "i8" => "int8_t",
        "i16" => "int16_t",
        "i32" => "int32_t",
        "i64" => "int64_t",
        "f32" => "float",
        "f64" => "double",
        "()" => "void",
        other => other,
    }
    .to_string()
}

fn default_return(ty: &TypeRef, return_type: &str, fallible: bool) -> String {
    if fallible
        || return_type == "i32"
            && matches!(
                ty,
                TypeRef::String | TypeRef::Json | TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _)
            )
    {
        return "return 0;".into();
    }
    match ty {
        TypeRef::Unit => "return;".into(),
        TypeRef::Primitive(PrimitiveType::Bool) => "return 0;".into(),
        TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::Optional(_) => "return 0;".into(),
        _ => "return 0;".into(),
    }
}

pub(super) fn emit_test_backend(
    _bridge: &TraitBridgeConfig,
    _methods: &[&MethodDef],
    _fixture: &Fixture,
) -> super::super::TestBackendEmission {
    super::super::TestBackendEmission::unimplemented("c")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, ReceiverKind, TypeDef};
    use std::process::Command;

    #[test]
    fn generated_trait_bridge_snippet_compiles_against_public_header() {
        let fixture = Fixture {
            id: "register_sample_backend".into(),
            call: Some("register_sample_backend".into()),
            args: vec![crate::core::config::e2e::ArgMapping {
                name: "backend".into(),
                field: "backend".into(),
                arg_type: "test_backend".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: Some("SampleBackend".into()),
            }],
            ..Fixture::default()
        };
        let bridge = TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            super_trait: Some("sample::Plugin".into()),
            register_fn: Some("register_sample_backend".into()),
            unregister_fn: Some("unregister_sample_backend".into()),
            ..TraitBridgeConfig::default()
        };
        let method = MethodDef {
            name: "supports".into(),
            params: vec![ParamDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        };
        let types = [TypeDef {
            name: "SampleBackend".into(),
            is_trait: true,
            methods: vec![method],
            ..TypeDef::default()
        }];
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![bridge],
            ..ResolvedCrateConfig::default()
        };
        let rendered = render(&fixture, "sample_ffi.h", "sample", &config, &types).expect("render C bridge snippet");
        let header = concat!(
            "#include <stddef.h>\n#include <stdint.h>\n",
            "typedef struct SAMPLESampleSampleBackendVTable {\n",
            "int32_t (*name_fn)(const void *, char **, char **);\n",
            "int32_t (*version_fn)(const void *, char **, char **);\n",
            "int32_t (*initialize_fn)(const void *, char **);\n",
            "int32_t (*shutdown_fn)(const void *, char **);\n",
            "int32_t (*supports)(const void *, const char *);\n",
            "void (*free_string)(char *);\nvoid (*free_user_data)(void *);\n",
            "} SAMPLESampleSampleBackendVTable;\n",
            "int32_t sample_register_sample_backend(const char *, const SAMPLESampleSampleBackendVTable *, const void *, char **);\n",
            "int32_t sample_unregister_sample_backend(const char *, char **);\n",
            "void sample_free_string(char *);\n",
        );
        super::super::snippet_regressions::compile_snippet(&rendered, header);
        run_cleanup_harness(&rendered, header);
    }

    #[test]
    fn rust_ffi_spellings_map_to_c_header_spellings() {
        let cases = [
            ("*const std::ffi::c_void", "const void *"),
            ("*mut std::ffi::c_void", "void *"),
            ("*const std::ffi::c_char", "const char *"),
            ("*mut *mut std::ffi::c_char", "char **"),
            ("*const u8", "const uint8_t *"),
            ("usize", "size_t"),
            ("isize", "ptrdiff_t"),
            ("u8", "uint8_t"),
            ("i16", "int16_t"),
            ("i32", "int32_t"),
            ("u64", "uint64_t"),
            ("f64", "double"),
        ];

        for (rust, c) in cases {
            assert_eq!(c_rust_type(rust), c);
        }
    }

    fn run_cleanup_harness(rendered: &str, header: &str) {
        let Some(compiler) = ["cc", "clang", "gcc"]
            .into_iter()
            .find(|candidate| which::which(candidate).is_ok())
        else {
            return;
        };
        let directory = tempfile::tempdir().expect("temporary C runtime directory");
        std::fs::write(directory.path().join("sample_ffi.h"), header).expect("write neutral C header");
        std::fs::write(directory.path().join("snippet.c"), rendered).expect("write generated C snippet");
        std::fs::write(
            directory.path().join("runtime.c"),
            concat!(
                "#include \"sample_ffi.h\"\n",
                "static const SAMPLESampleSampleBackendVTable *saved_vtable;\n",
                "static void *saved_user_data;\n",
                "static int releases;\n",
                "int32_t sample_register_sample_backend(const char *name, const SAMPLESampleSampleBackendVTable *vtable, const void *user_data, char **out_error) {\n",
                "  (void)name; *out_error = 0; saved_vtable = vtable; saved_user_data = (void *)user_data; return 0;\n",
                "}\n",
                "int32_t sample_unregister_sample_backend(const char *name, char **out_error) {\n",
                "  (void)name; *out_error = 0; saved_vtable->free_user_data(saved_user_data); releases += 1; return releases == 1 ? 0 : -1;\n",
                "}\n",
                "void sample_free_string(char *value) { (void)value; }\n",
            ),
        )
        .expect("write neutral C runtime");
        let binary = directory.path().join("snippet");
        let compile = Command::new(compiler)
            .args(["-std=c11", "-Wall", "-Werror", "-I"])
            .arg(directory.path())
            .arg(directory.path().join("snippet.c"))
            .arg(directory.path().join("runtime.c"))
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("compile C runtime harness");
        assert!(
            compile.status.success(),
            "C runtime harness failed to compile: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let status = Command::new(binary).status().expect("run C runtime harness");
        assert!(status.success(), "C runtime harness did not clean up exactly once");
    }
}
