use alef::core::ir::PrimitiveType;

#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use support::{compile_java, java_available, primitive_param, run_java, service_source, write_file};

#[test]
fn service_upcall_contains_all_failures_and_returns_null() {
    let source = service_source(Vec::new());
    let method = source
        .split("private static MemorySegment invokeHandlerWithMarshal(")
        .nth(1)
        .expect("generated upcall adapter");
    let catch = method.find("catch (Throwable").expect("upcall Throwable containment");
    let null_return = method[catch..]
        .find("return MemorySegment.NULL;")
        .expect("upcall null failure result");
    assert!(method.find("handler.handle(requestStr)").unwrap() < catch, "{source}");
    assert!(
        method.find("scratchArena.allocateFrom(responseStr)").unwrap() < catch,
        "{source}"
    );
    assert!(
        method.find("mallocHandle.invoke(responseLen)").unwrap() < catch,
        "{source}"
    );
    assert!(null_return > 0, "{source}");
}

#[test]
fn service_upcall_runtime_contains_handler_marshal_and_allocation_failures() {
    if !java_available() {
        return;
    }
    let source = inject_allocation_failure(&service_source(Vec::new()));
    let directory = tempfile::tempdir().expect("temporary upcall directory");
    write_upcall_sources(directory.path(), &source);
    compile_java(
        directory.path(),
        &[
            "com/test/NativeLib.java",
            "com/test/Callable.java",
            "com/test/TestService.java",
            "com/test/UpcallFailureMain.java",
        ],
    );
    run_java(directory.path(), "com.test.UpcallFailureMain");
}

fn inject_allocation_failure(source: &str) -> String {
    source.replace(
        "MethodHandle mallocHandle = LINKER.downcallHandle(",
        "if (responseStr.equals(\"allocation\")) throw new OutOfMemoryError(\"allocation\");\n            MethodHandle mallocHandle = LINKER.downcallHandle(",
    )
}

fn write_upcall_sources(directory: &std::path::Path, service: &str) {
    write_file(directory, "com/test/TestService.java", service);
    write_file(
        directory,
        "com/test/NativeLib.java",
        "package com.test; final class NativeLib {}\n",
    );
    write_file(
        directory,
        "com/test/Callable.java",
        "package com.test; interface Callable { String handle(String request); }\n",
    );
    write_file(
        directory,
        "com/test/UpcallFailureMain.java",
        include_str!("fixtures/java_upcall_failure_main.java"),
    );
}

#[test]
fn service_free_symbol_is_resolved_lazily_not_at_class_init() {
    let source = service_source(Vec::new());

    let static_block = source
        .split("static {")
        .nth(1)
        .expect("generated static initializer block")
        .split("private static MemorySegment invokeHandlerWithMarshal")
        .next()
        .expect("static initializer body");
    assert!(
        !static_block.contains("LOOKUP.find(\"free\")"),
        "static initializer must not eagerly resolve \"free\" — a service that never frees a \
         handler response would otherwise fail to even load: {source}"
    );
    assert!(
        !source.contains("private static final MethodHandle FREE"),
        "the free downcall handle must not be memoized as a class-init-time static field: {source}"
    );

    let free_method = source
        .split("private static void freeHandlerResponse(")
        .nth(1)
        .expect("generated freeHandlerResponse method");
    assert!(
        free_method.contains("LOOKUP.find(\"free\").orElseThrow()"),
        "freeHandlerResponse must still resolve \"free\" itself, lazily, on first use: {source}"
    );
}

#[test]
fn service_metadata_uses_exact_c_abi_layouts_and_carriers() {
    let source = service_source(vec![
        primitive_param("flag", PrimitiveType::Bool),
        primitive_param("i8_value", PrimitiveType::I8),
        primitive_param("u8_value", PrimitiveType::U8),
        primitive_param("i32_value", PrimitiveType::I32),
        primitive_param("u32_value", PrimitiveType::U32),
        primitive_param("i64_value", PrimitiveType::I64),
        primitive_param("u64_value", PrimitiveType::U64),
    ]);
    for name in ["flag", "i8Value", "u8Value"] {
        assert!(
            source.contains(&format!("ValueLayout.JAVA_BYTE    // {name} param")),
            "{source}"
        );
    }
    for name in ["i32Value", "u32Value"] {
        assert!(
            source.contains(&format!("ValueLayout.JAVA_INT    // {name} param")),
            "{source}"
        );
    }
    for name in ["i64Value", "u64Value"] {
        assert!(
            source.contains(&format!("ValueLayout.JAVA_LONG    // {name} param")),
            "{source}"
        );
    }
    assert!(source.contains("(byte) (flag ? 1 : 0)"), "{source}");
    assert!(!source.contains("(long) i8Value"), "{source}");
    assert!(!source.contains("(long) i32Value"), "{source}");
}
