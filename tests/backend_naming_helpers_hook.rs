use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn hook_path() -> PathBuf {
    repo_root().join("hooks/check_backend_naming_helpers.py")
}

fn run_hook(cwd: &Path, files: &[&str]) -> Output {
    let mut command = Command::new("python3");
    command.current_dir(cwd);
    command.arg(hook_path());
    for file in files {
        command.arg(file);
    }
    command.output().expect("hook command must run")
}

#[test]
fn rejects_backend_local_generic_naming_helpers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src/backends/node");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    fs::write(
        backend_dir.join("helpers.rs"),
        "pub(crate) fn to_snake_case(name: &str) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let output = run_hook(dir.path(), &["src/backends/node/helpers.rs"]);

    assert!(!output.status.success(), "hook should reject backend-local helper");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("backend-local helper `to_snake_case`"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("src/codegen/naming.rs"), "stderr: {stderr}");
}

#[test]
fn accepts_context_specific_backend_wrapper_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src/backends/go");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    fs::write(
        backend_dir.join("helpers.rs"),
        "fn go_visitor_bridge_function_component(name: &str) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let output = run_hook(dir.path(), &["src/backends/go/helpers.rs"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rejects_a_generic_helper_outside_src_backends() {
    let dir = tempfile::tempdir().expect("tempdir");
    let codegen_dir = dir.path().join("src/codegen/generators");
    fs::create_dir_all(&codegen_dir).expect("create codegen dir");
    fs::write(
        codegen_dir.join("enums.rs"),
        "fn apply_rename_all(name: &str, rule: &str) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let output = run_hook(dir.path(), &["src/codegen/generators/enums.rs"]);

    assert!(
        !output.status.success(),
        "hook should reject a generic helper outside src/backends/"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("backend-local helper `apply_rename_all`"),
        "stderr: {stderr}"
    );
}

#[test]
fn rejects_a_language_prefixed_variant_of_a_banned_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend_dir = dir.path().join("src/backends/java");
    fs::create_dir_all(&backend_dir).expect("create backend dir");
    fs::write(
        backend_dir.join("helpers.rs"),
        "pub(crate) fn java_apply_rename_all(name: &str, rename_all: Option<&str>) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let output = run_hook(dir.path(), &["src/backends/java/helpers.rs"]);

    assert!(
        !output.status.success(),
        "hook should reject a language-prefixed variant of a banned name"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("backend-local helper `java_apply_rename_all`"),
        "stderr: {stderr}"
    );
}

#[test]
fn accepts_the_allowlisted_canonical_and_wrapper_definitions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let naming_dir = dir.path().join("src/codegen");
    fs::create_dir_all(&naming_dir).expect("create codegen dir");
    fs::write(
        naming_dir.join("naming.rs"),
        "pub fn wire_variant_value(variant_name: &str, serde_rename: Option<&str>, rename_all: Option<&str>) -> String { variant_name.to_string() }\n\
         pub fn pascal_to_snake(name: &str) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let java_dir = dir.path().join("src/backends/java/gen_bindings");
    fs::create_dir_all(&java_dir).expect("create java dir");
    fs::write(
        java_dir.join("helpers.rs"),
        "pub(crate) fn java_apply_rename_all(name: &str, rename_all: Option<&str>) -> String { name.to_string() }\n",
    )
    .expect("write fixture");

    let output = run_hook(
        dir.path(),
        &["src/codegen/naming.rs", "src/backends/java/gen_bindings/helpers.rs"],
    );

    assert!(
        output.status.success(),
        "allowlisted canonical/wrapper definitions must not be flagged; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
