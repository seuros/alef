use crate::backends::ffi::template_env;

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
    assert!(source.contains("ALEF_INVALID_HANDLE_ERROR: i32 = 4"));
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
    let mut source = String::from("fn set_last_error(_: i32, _: &str) {}\n");
    source.push_str(&template_env::render(
        "handle_registry.rs.jinja",
        minijinja::context! {},
    ));
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
