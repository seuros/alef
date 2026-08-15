#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use support::{compile_java, java_available, opaque_source, run_java_args, service_source, write_file};

#[test]
fn opaque_transfer_allocation_failure_keeps_owner_closeable() {
    run_transfer_scenario("opaque");
}

#[test]
fn service_transfer_allocation_failure_keeps_owner_closeable() {
    run_transfer_scenario("service");
}

fn run_transfer_scenario(scenario: &str) {
    if !java_available() {
        return;
    }
    let opaque = inject_opaque_transfer_failure(&opaque_source());
    let service = inject_service_transfer_failure(&service_source(Vec::new()));
    let directory = tempfile::tempdir().expect("temporary transfer directory");
    write_transfer_sources(directory.path(), &opaque, &service);
    compile_java(
        directory.path(),
        &[
            "com/test/NativeLib.java",
            "com/test/Callable.java",
            "com/test/TestLibRsException.java",
            "com/test/Resource.java",
            "com/test/TestService.java",
            "com/test/TransferFailureMain.java",
        ],
    );
    run_java_args(
        directory.path(),
        &["-cp", ".", "com.test.TransferFailureMain", scenario],
    );
}

fn inject_opaque_transfer_failure(source: &str) -> String {
    source.replace(
        "private HandleTransfer(MemorySegment transferredHandle) {",
        "private HandleTransfer(MemorySegment transferredHandle) {\n            if (Boolean.getBoolean(\"alef.fail.transfer\")) throw new OutOfMemoryError(\"transfer\");",
    )
}

fn inject_service_transfer_failure(source: &str) -> String {
    source.replace(
        "private OwnerHandleTransfer(long transferredHandle) {",
        "private OwnerHandleTransfer(long transferredHandle) {\n            if (Boolean.getBoolean(\"alef.fail.transfer\")) throw new OutOfMemoryError(\"transfer\");",
    )
}

fn write_transfer_sources(directory: &std::path::Path, opaque: &str, service: &str) {
    write_file(directory, "com/test/Resource.java", opaque);
    write_file(directory, "com/test/TestService.java", service);
    write_file(
        directory,
        "com/test/NativeLib.java",
        include_str!("fixtures/java_transfer_native_lib.java"),
    );
    write_file(
        directory,
        "com/test/Callable.java",
        "package com.test; interface Callable { String handle(String request); }\n",
    );
    write_file(
        directory,
        "com/test/TestLibRsException.java",
        "package com.test; class TestLibRsException extends Exception { TestLibRsException(int code, String message) { super(message); } TestLibRsException(String message, Throwable cause) { super(message, cause); } }\n",
    );
    write_file(
        directory,
        "com/test/TransferFailureMain.java",
        include_str!("fixtures/java_transfer_failure_main.java"),
    );
}
