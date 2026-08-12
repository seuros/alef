use super::super::FfiBackend;
use super::common::{resolved_one, sample_api, sample_config};
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, MethodDef, TypeDef, TypeRef};

fn generated_lib(api: &ApiSurface) -> String {
    let content = FfiBackend
        .generate_bindings(api, &sample_config())
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    syn::parse_file(&content).expect("generated FFI runtime must be valid Rust");
    content
}

fn generated_export_block<'a>(content: &'a str, function_name: &str) -> &'a str {
    let signature = format!("fn {function_name}(");
    let signature_start = content.find(&signature).unwrap();
    let block_start = content[..signature_start]
        .rfind("\n\n")
        .map_or(signature_start, |offset| offset + 2);
    let tail = &content[block_start..];
    let block_end = tail.find("\n}\n").map_or(tail.len(), |offset| offset + 3);
    &tail[..block_end]
}

#[test]
fn string_return_lengths_are_isolated_by_export() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![
            FunctionDef {
                name: "short_value".to_string(),
                rust_path: "sample_lib::short_value".to_string(),
                return_type: TypeRef::String,
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "long_value".to_string(),
                rust_path: "sample_lib::long_value".to_string(),
                return_type: TypeRef::String,
                ..FunctionDef::default()
            },
        ],
        ..ApiSurface::default()
    };
    let lib = generated_lib(&api);

    assert!(lib.contains("LAST_RETURN_LENGTHS"));
    assert!(!lib.contains("static LAST_RETURN_LEN:"));
    assert!(lib.contains("set_last_return_len(\"my_lib_short_value\", 0);"));
    assert!(lib.contains("set_last_return_len(\"my_lib_long_value\", 0);"));
    assert!(lib.contains("last_return_len(\"my_lib_short_value\")"));
    assert!(lib.contains("last_return_len(\"my_lib_long_value\")"));
    let short_export = generated_export_block(&lib, "my_lib_short_value");
    assert!(
        short_export.find("catch_unwind").unwrap() < short_export.find("set_last_return_len").unwrap(),
        "return-length storage may allocate and must remain inside the panic boundary"
    );
}

#[test]
fn feature_gated_opaque_lifecycle_exports_are_symmetric() {
    let gated_type = TypeDef {
        name: "DownloadManager".to_string(),
        rust_path: "sample_lib::DownloadManager".to_string(),
        methods: vec![MethodDef {
            name: "new".to_string(),
            return_type: TypeRef::Named("DownloadManager".to_string()),
            is_static: true,
            ..MethodDef::default()
        }],
        is_opaque: true,
        cfg: Some(r#"feature = "download""#.to_string()),
        ..TypeDef::default()
    };
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![gated_type],
        ..ApiSurface::default()
    };
    let lib = generated_lib(&api);

    let constructor = generated_export_block(&lib, "my_lib_download_manager_new");
    let destructor = generated_export_block(&lib, "my_lib_download_manager_free");
    assert!(constructor.contains("#[cfg(feature = \"download\")]"));
    assert!(destructor.contains("#[cfg(feature = \"download\")]"));

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|file| file.path.ends_with("cbindgen.toml")).unwrap();
    assert!(
        cbindgen
            .content
            .contains(r#""feature = \"download\"" = "SAMPLE_FEATURE_DOWNLOAD""#)
    );
}

#[test]
fn field_accessor_reports_conversion_errors_and_documents_ownership() {
    let lib = generated_lib(&sample_api());
    let accessor = generated_export_block(&lib, "my_lib_config_name");

    assert!(accessor.contains("catch_ffi_panic"));
    assert!(accessor.contains("set_last_error(ALEF_FFI_CONVERSION_ERROR"));
    assert!(accessor.contains("FFI field value contains an interior NUL byte"));
    assert!(accessor.contains("A non-null returned pointer is owned by the caller."));
    assert!(accessor.contains("It must be freed with `my_lib_free_string`."));

    let named_api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Metrics".to_string(),
                rust_path: "sample_lib::Metrics".to_string(),
                is_clone: true,
                ..TypeDef::default()
            },
            TypeDef {
                name: "ProcessResult".to_string(),
                rust_path: "sample_lib::ProcessResult".to_string(),
                fields: vec![FieldDef {
                    name: "metrics".to_string(),
                    ty: TypeRef::Named("Metrics".to_string()),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };
    let named_lib = generated_lib(&named_api);
    let named_accessor = generated_export_block(&named_lib, "my_lib_process_result_metrics");
    assert!(named_accessor.contains("A non-null returned handle is owned by the caller."));
    assert!(named_accessor.contains("It must be freed with `my_lib_metrics_free`."));
}
