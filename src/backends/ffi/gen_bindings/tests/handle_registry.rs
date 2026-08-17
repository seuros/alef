use crate::backends::ffi::template_env;
use crate::core::backend::Backend;

#[test]
fn generated_registry_uses_typed_generational_tokens() {
    let source = template_env::render("handle_registry.rs.jinja", minijinja::context! {});

    assert!(source.contains("type AlefHandle = u64"));
    assert!(source.contains("generation: u32"));
    assert!(source.contains("Box<dyn std::any::Any + Send>"));
    assert!(source.contains("downcast_ref::<T>()"));
    assert!(source.contains("downcast_mut::<T>()"));
    assert!(source.contains("slot.generation = next_generation"));
    assert!(source.contains("slot.value.take()"));
    assert!(!source.contains("const ALEF_INVALID_HANDLE_ERROR"));
    syn::parse_file(&source).expect("generated handle registry must parse as Rust");
}

#[test]
fn registry_does_not_reconstruct_boxes_from_host_values() {
    let source = template_env::render("handle_registry.rs.jinja", minijinja::context! {});

    assert!(!source.contains("Box::from_raw"));
    assert!(!source.contains("unsafe"));
}

#[test]
fn registry_rejects_stale_forged_and_wrong_type_handles() {
    let mut source = String::from("const ALEF_INVALID_HANDLE_ERROR: i32 = 4;\nfn set_last_error(_: i32, _: &str) {}\n");
    let mut registry = template_env::render("handle_registry.rs.jinja", minijinja::context! {});
    let serialized_start = registry
        .find("struct SerializedHandle")
        .expect("serialized helper start");
    let core_registry_resume = registry[serialized_start..]
        .find("fn with_handle")
        .map(|offset| serialized_start + offset)
        .expect("core registry helpers resume");
    registry.replace_range(serialized_start..core_registry_resume, "");
    source.push_str(&registry);
    source.push_str(
        r#"
fn main() {
    let first = insert_handle(String::from("sample")).expect("insert");
    assert_eq!(with_handle::<String, _>(first, |value| value.len()).expect("borrow"), 6);
    assert!(matches!(with_handle::<u64, _>(first, |_| ()), Err(HandleError::WrongType)));
    remove_handle::<String>(first).expect("remove");
    assert!(matches!(with_handle::<String, _>(first, |_| ()), Err(HandleError::StaleGeneration)));
    assert!(matches!(remove_handle::<String>(first), Err(HandleError::StaleGeneration)));
    assert!(matches!(with_handle::<String, _>(u64::MAX, |_| ()), Err(HandleError::UnknownSlot)));
    assert!(matches!(with_handle::<String, _>(0, |_| ()), Err(HandleError::InvalidZero)));
    let second = insert_handle(String::from("next")).expect("reuse");
    assert_ne!(first, second);
    let third = insert_handle(7_u64).expect("second type");
    let aliased = [
        HandleRequest { handle: second, expected_type: std::any::TypeId::of::<String>() },
        HandleRequest { handle: second, expected_type: std::any::TypeId::of::<String>() },
    ];
    assert!(matches!(acquire_handles(&aliased), Err(HandleError::AliasedHandle)));
    let partial = [
        HandleRequest { handle: second, expected_type: std::any::TypeId::of::<String>() },
        HandleRequest { handle: u64::MAX, expected_type: std::any::TypeId::of::<u64>() },
    ];
    assert!(acquire_handles(&partial).is_err());
    assert_eq!(with_handle::<String, _>(second, Clone::clone).expect("not consumed"), "next");
    let forward = [
        HandleRequest { handle: second, expected_type: std::any::TypeId::of::<String>() },
        HandleRequest { handle: third, expected_type: std::any::TypeId::of::<u64>() },
    ];
    let reverse = [
        HandleRequest { handle: third, expected_type: std::any::TypeId::of::<u64>() },
        HandleRequest { handle: second, expected_type: std::any::TypeId::of::<String>() },
    ];
    let forward_values = acquire_handles(&forward).expect("forward acquisition");
    let reverse_values = acquire_handles(&reverse).expect("reverse acquisition");
    assert_eq!(forward_values.iter().map(|(handle, _)| *handle).collect::<Vec<_>>(), reverse_values.iter().map(|(handle, _)| *handle).collect::<Vec<_>>());
}

"#,
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("registry.rs");
    let binary_path = directory.path().join("registry-test");
    std::fs::write(&source_path, source).expect("write harness");
    let compile = std::process::Command::new("rustc")
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let run = std::process::Command::new(&binary_path)
        .output()
        .expect("run registry harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn acquisition_rejects_aliases_before_locking_entries() {
    let source = template_env::render("handle_registry.rs.jinja", minijinja::context! {});

    let acquire = source.split("fn acquire_handles").nth(1).expect("acquisition helper");
    let duplicate_check = acquire.find("ordered.windows(2)").expect("duplicate-token check");
    let entry_lock = acquire.find("let guard = value.lock()").expect("entry lock");
    assert!(
        duplicate_check < entry_lock,
        "aliases must fail before entry locks are acquired"
    );
    assert!(source.contains("ordered.sort_by_key(|request| request.handle)"));
    assert!(source.contains("HandleError::AliasedHandle"));
}

#[test]
fn opaque_type_exports_use_scalar_handles_and_parse() {
    let mut resource = crate::core::ir::TypeDef {
        name: "Resource".into(),
        is_opaque: true,
        ..Default::default()
    };
    resource.methods.push(crate::core::ir::MethodDef {
        name: "label".into(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        cfg: None,
        return_type: crate::core::ir::TypeRef::String,
        ..Default::default()
    });
    let api = crate::core::ir::ApiSurface {
        crate_name: "sample".into(),
        types: vec![resource],
        ..Default::default()
    };
    let config = super::common::sample_config();
    let files = super::super::FfiBackend
        .generate_bindings(&api, &config)
        .expect("FFI generation");
    let lib = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library");
    let cbindgen = files
        .iter()
        .find(|file| file.path.ends_with("cbindgen.toml"))
        .expect("cbindgen config");

    syn::parse_file(&lib.content).expect("ordinary opaque handle exports must parse");
    assert!(lib.content.contains("this: AlefHandle"), "{}", lib.content);
    assert!(
        lib.content.contains("locked_handle_ptr::<my_lib::Resource"),
        "{}",
        lib.content
    );
    assert!(
        cbindgen.content.contains("typedef uint64_t MY_LIBResource;"),
        "{}",
        cbindgen.content
    );
    assert!(!lib.content.contains("Box::from_raw(this)"), "{}", lib.content);
}

#[test]
fn generated_calls_acquire_all_handles_before_use_or_owned_take() {
    use crate::core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

    let mut resource = TypeDef {
        name: "Resource".into(),
        is_opaque: true,
        ..Default::default()
    };
    resource.methods.push(MethodDef {
        name: "merge".into(),
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
        params: vec![ParamDef {
            name: "other".into(),
            ty: TypeRef::Named("Resource".into()),
            is_ref: true,
            ..Default::default()
        }],
        ..Default::default()
    });
    let context = TypeDef {
        name: "Context".into(),
        is_opaque: true,
        ..Default::default()
    };
    let function = FunctionDef {
        name: "compare".into(),
        rust_path: "my_lib::compare".into(),
        params: vec![
            ParamDef {
                name: "left".into(),
                ty: TypeRef::Named("Resource".into()),
                is_ref: true,
                ..Default::default()
            },
            ParamDef {
                name: "context".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("Context".into()))),
                optional: true,
                is_ref: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let api = crate::core::ir::ApiSurface {
        crate_name: "sample".into(),
        types: vec![resource, context],
        functions: vec![function],
        ..Default::default()
    };
    let files = super::super::FfiBackend
        .generate_bindings(&api, &super::common::sample_config())
        .expect("FFI generation");
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library")
        .content;

    syn::parse_file(source).unwrap_or_else(|error| panic!("multi-handle wrappers must parse: {error}\n{source}"));
    let free_function = source.split("fn my_lib_compare").nth(1).expect("free function wrapper");
    assert!(free_function.contains("left: AlefHandle"), "{free_function}");
    assert!(free_function.contains("context: AlefHandle"), "{free_function}");
    let acquisition = free_function
        .find("acquire_handles")
        .expect("free-function acquisition");
    let conversion = free_function
        .find("locked_handle_ptr::<my_lib::Resource>")
        .expect("free-function conversion");
    assert!(acquisition < conversion, "{free_function}");

    let owned_method = source
        .split("fn my_lib_resource_merge")
        .nth(1)
        .expect("owned method wrapper");
    assert!(owned_method.contains("this: AlefHandle"), "{owned_method}");
    let alias_check = owned_method.find("request.handle == this").expect("owned alias check");
    let owned_take = owned_method
        .find("take_handle::<my_lib::Resource>")
        .expect("owned take");
    assert!(alias_check < owned_take, "{owned_method}");
}

#[test]
fn borrowed_types_keep_owned_lifecycle_and_accessor_exports() {
    use crate::core::ir::{FieldDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};

    let borrowed = TypeDef {
        name: "BorrowedNode".into(),
        rust_path: "sample_lib::BorrowedNode".into(),
        has_lifetime_params: true,
        has_serde: true,
        fields: vec![FieldDef {
            name: "attributes".into(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        methods: vec![
            MethodDef {
                name: "into_owned".into(),
                return_type: TypeRef::Named("BorrowedNode".into()),
                receiver: Some(crate::core::ir::ReceiverKind::Owned),
                cfg: None,
                ..MethodDef::default()
            },
            MethodDef {
                name: "with_owned_attributes".into(),
                return_type: TypeRef::Named("BorrowedNode".into()),
                is_static: true,
                ..MethodDef::default()
            },
        ],
        ..TypeDef::default()
    };
    let defaultable = |name: &str| TypeDef {
        name: name.into(),
        rust_path: format!("sample_lib::{name}"),
        has_lifetime_params: true,
        has_serde: true,
        methods: vec![MethodDef {
            name: "default".into(),
            return_type: TypeRef::Named(name.into()),
            is_static: true,
            returns_ref: true,
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let owner = TypeDef {
        name: "Document".into(),
        rust_path: "sample_lib::Document".into(),
        fields: vec![FieldDef {
            name: "node".into(),
            ty: TypeRef::Named("BorrowedNode".into()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let inspect = FunctionDef {
        name: "inspect".into(),
        rust_path: "sample_lib::inspect".into(),
        params: vec![ParamDef {
            name: "node".into(),
            ty: TypeRef::Named("BorrowedNode".into()),
            is_ref: true,
            ..ParamDef::default()
        }],
        ..FunctionDef::default()
    };
    let owned_default = |name: &str, return_type: &str| FunctionDef {
        name: name.into(),
        rust_path: format!("sample_lib::{name}"),
        return_type: TypeRef::Named(return_type.into()),
        ..FunctionDef::default()
    };
    let borrowed_default = FunctionDef {
        name: "borrowed_options_default".into(),
        rust_path: "sample_lib::borrowed_options_default".into(),
        return_type: TypeRef::Named("RenderOptions".into()),
        returns_ref: true,
        ..FunctionDef::default()
    };
    let api = crate::core::ir::ApiSurface {
        crate_name: "sample".into(),
        types: vec![
            borrowed,
            owner,
            defaultable("RenderOptions"),
            defaultable("PreprocessOptions"),
        ],
        functions: vec![
            inspect,
            owned_default("conversion_options_default", "RenderOptions"),
            owned_default("preprocessing_options_default", "PreprocessOptions"),
            borrowed_default,
        ],
        ..Default::default()
    };
    let files = super::super::FfiBackend
        .generate_bindings(&api, &super::common::sample_config())
        .expect("FFI generation");
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library")
        .content;

    syn::parse_file(source).expect("borrowed-type owned exports must parse");
    assert!(source.contains("my_lib_borrowed_node_from_json"), "{source}");
    assert!(source.contains("my_lib_borrowed_node_to_json"), "{source}");
    assert!(source.contains("my_lib_borrowed_node_free"), "{source}");
    assert!(source.contains("my_lib_borrowed_node_attributes"), "{source}");
    assert!(source.contains("my_lib_borrowed_node_into_owned"), "{source}");
    assert!(
        source.contains("my_lib_borrowed_node_with_owned_attributes"),
        "{source}"
    );
    assert!(source.contains("my_lib_render_options_default"), "{source}");
    assert!(source.contains("my_lib_preprocess_options_default"), "{source}");
    assert!(source.contains("my_lib_conversion_options_default"), "{source}");
    assert!(source.contains("my_lib_preprocessing_options_default"), "{source}");
    assert!(!source.contains("my_lib_borrowed_options_default"), "{source}");
    assert!(
        source.contains("SerializedHandle<sample_lib::BorrowedNode<'static>>"),
        "borrowed contexts must use a Send snapshot wrapper:\n{source}"
    );
    assert!(source.contains("insert_serialized_handle(&val)"), "{source}");
    assert!(source.contains("insert_serialized_handle(&result)"), "{source}");
    assert!(source.contains("serde_json::from_str(&snapshot.json)"), "{source}");
    assert!(
        source.contains("fn my_lib_borrowed_node_into_owned(")
            && source.contains("take_handle::<SerializedHandle<sample_lib::BorrowedNode<'static>>>(this)")
            && source.contains("match insert_serialized_handle(&result)"),
        "owned methods must consume and replace typed snapshots:\n{source}"
    );
    assert!(
        !source.contains("insert_handle(val)"),
        "the non-Send borrowed value itself must never enter the registry:\n{source}"
    );
    assert!(!source.contains("my_lib_document_node"), "{source}");
    assert!(!source.contains("my_lib_inspect"), "{source}");
}

#[test]
fn owned_receiver_alias_check_has_concrete_request_type() {
    let source = crate::backends::ffi::template_env::render(
        "handle_acquisition.rs.jinja",
        minijinja::context! {
            requests => "",
            request_count => 0,
            fail_ret => "return 0;",
            owned_handle => "this",
        },
    );

    assert!(
        source.contains("let mut __alef_requests: Vec<HandleRequest> = Vec::with_capacity(0)"),
        "{source}"
    );
}

#[test]
fn generated_ffi_lib_rs_carries_the_handle_abi_stamp() {
    let files = super::super::FfiBackend
        .generate_bindings(&super::common::sample_api(), &super::common::sample_config())
        .expect("FFI generation");
    let lib = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library");

    assert!(
        lib.content.contains("type AlefHandle = u64"),
        "fixture must actually contain the handle representation being stamped"
    );
    crate::backends::ffi::handle_abi_stamp::assert_stamped_before_hashing(&lib.content, "ffi lib.rs");
}
