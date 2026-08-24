//! Real-`javac` regression coverage for oversized string arguments in generated doc snippets.
//!
//! Split out of `snippet.rs`'s inline `tests` module to keep that file under the 1000-line cap.
//! These are the only Java snippet tests that invoke the toolchain, and they have to: the JVM's
//! 65535-byte `CONSTANT_Utf8` cap is invisible to any assertion over rendered text. The previous
//! fix for this defect shipped `"a" + "b"` chunking, which satisfied every render-only assertion
//! and still failed to compile, because concatenation between literals is a compile-time constant
//! expression (JLS 15.29) that `javac` folds back into a single pool entry. ~keep

use super::super::snippet::render_snippet_body;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::fixture::Fixture;

/// Compiles `sources` (relative-path, content) pairs with the real `javac` in a fresh temp
/// directory. Returns `(success, combined stdout+stderr)`. A render-only assertion cannot
/// see the JVM's 65535-byte `CONSTANT_Utf8` cap -- the generated string looks fine; only the
/// compiler refuses it -- so the two tests below both go through the real toolchain instead
/// of inspecting the rendered text. ~keep
fn compile_java_sources(sources: &[(&str, String)]) -> (bool, String) {
    let dir = tempfile::tempdir().expect("temp dir for javac regression");
    let mut paths = Vec::new();
    for (relative, content) in sources {
        let path = dir.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source parent dir");
        }
        std::fs::write(&path, content).expect("write java source");
        paths.push(path);
    }
    let classes = dir.path().join("classes");
    std::fs::create_dir_all(&classes).expect("create classes dir");
    let output = std::process::Command::new("javac")
        .args(["-d"])
        .arg(&classes)
        .args(&paths)
        .output()
        .expect("javac runs");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Sanity check for the two tests that follow: this repo's own house bug is a check that
/// silently passes without ever exercising the thing it claims to verify. Before trusting
/// `compile_java_sources` to prove anything, prove `javac` itself rejects a genuinely
/// oversized single string literal with the JVM's own diagnostic -- so a green result below
/// is evidence the compiler ran and approved, not evidence the compiler never ran at all.
#[test]
fn javac_itself_rejects_a_single_oversized_string_literal() {
    let Ok(_javac) = which::which("javac") else {
        panic!(
            "javac is not on PATH in this environment -- this test (and the compile-based \
             regression next to it) cannot verify anything without it. Install a JDK before \
             trusting either result."
        );
    };
    // One raw, unchunked literal -- exactly the pre-fix shape -- well over the JVM's
    // 65535-byte CONSTANT_Utf8 cap.
    let oversized_payload = "abcdefghij".repeat(10_000); // 100,000 bytes
    let broken_source = format!(
        "public final class Broken {{ public static void main(String[] args) {{ String s = \"{oversized_payload}\"; }} }}\n"
    );
    let (success, output) = compile_java_sources(&[("Broken.java", broken_source)]);
    assert!(
        !success,
        "a single 100,000-byte string literal must be rejected by javac -- if this passes, \
         the harness is not actually invoking a real compiler: {output}"
    );
    assert!(
        output.contains("constant string too long"),
        "expected javac's own 'constant string too long' diagnostic, got: {output}"
    );
}

/// Regression for alef task #301: a plain (non-`json_object`) `string` fixture arg whose
/// value is large enough to threaten the JVM's 65535-byte `CONSTANT_Utf8` cap must produce a
/// doc snippet `javac` actually accepts -- not just one that looks fine when rendered. This
/// is the exact shape a `json_object` fixture never exercises: the value is inlined straight
/// into the call argument list via `json_to_java` -> `java_string_literal`, not through
/// `snippet_json_object_setup.jinja`'s `json_literal` seam. Neutral synthetic payload
/// (`project-agnostic-codegen`): not any real consumer's fixture, just large enough to force
/// the generator across the constant-pool limit. Companion fixture id (`error_handling_huge`)
/// deliberately echoes the reported shape without naming any real consumer. ~keep
#[test]
fn a_doc_snippet_with_an_oversized_plain_string_arg_compiles_with_javac() {
    let Ok(_javac) = which::which("javac") else {
        panic!(
            "javac is not on PATH in this environment -- this test cannot verify anything \
             without it. Install a JDK before trusting either result."
        );
    };
    let oversized_source = "abcdefghij".repeat(10_000); // 100,000 bytes
    let fixture = Fixture {
        id: "error_handling_huge".into(),
        description: "Reject an oversized source input".into(),
        input: serde_json::json!({"source": oversized_source}),
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "process_source".into(),
        result_var: "result".into(),
        returns_void: true,
        args: vec![crate::e2e::config::ArgMapping {
            name: "source".into(),
            field: "source".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "java".into(),
        CallOverride {
            class: Some("unconfigured.alef.HugeSourceFacade".into()),
            ..CallOverride::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
    );
    // ~keep Anti-vacuity premise: the value really did reach the generator oversized and
    // really was split. Asserted as `String.join`, never as `" + "` — a `+` between literals
    // is a compile-time constant expression (JLS 15.29) that `javac` folds straight back into
    // one `CONSTANT_Utf8`, so a premise phrased that way is satisfied by output that does not
    // compile. That is precisely how the original fix passed its tests while still emitting
    // code `javac` rejects.
    assert!(
        body.contains("String.join(\"\", "),
        "the generator must have split the oversized value into runtime-joined literal \
         chunks, not one giant literal:\n{}",
        &body[..body.len().min(500)]
    );

    let facade_source = "package unconfigured.alef;\n\
         public final class HugeSourceFacade {\n    \
         public static void processSource(String source) { }\n\
         }\n"
    .to_string();

    let (success, output) = compile_java_sources(&[
        ("Example.java", body.clone()),
        ("unconfigured/alef/HugeSourceFacade.java", facade_source),
    ]);
    assert!(
        success,
        "the generated doc snippet must actually compile under javac, not merely render \
         without a single overlong literal:\n{output}\n\ngenerated source:\n{body}"
    );
}
