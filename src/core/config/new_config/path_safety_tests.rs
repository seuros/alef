//! RED-first coverage for the config-path-safety fix: `crate_dir` overrides (jni/node/wasm),
//! `dart.lib_name`, dotted package/namespace overrides (java/kotlin/kotlin_android/csharp), and
//! the crate `name` field itself must be rejected with a contextual `ResolveError`, not silently
//! accepted, when the value would let a generated write escape the output tree.
//!
//! Most rejection tests below panicked on current code before the fix (the checked values are
//! exactly the shapes `validate_output_segment` / `validate_output_path` used to `panic!` on
//! inside `OutputTemplate::resolve`, now surfaced instead as `Err(ResolveError::InvalidConfig)`
//! at the same config-resolution boundary for these additional fields). The `crate_name_rejects_*`
//! tests differ: the leading-dot shape was silently *accepted* on current code (a JNI-only,
//! single-crate config never embeds the crate name in any `OutputTemplate`-resolved path, so
//! nothing there ever inspected it), while the directly-absolute shape still panicked (an
//! embedded `/` is caught by `OutputTemplate::resolve`'s own unconditional
//! `validate_output_segment(crate_name, ...)` call, which runs for the `jni` language too since
//! its per-crate explicit-output lookup always returns `None`) — both are failures on current
//! code either way, just via different failure modes. The no-change control at the bottom proves
//! an ordinary value still resolves to the exact same values as before.

use super::*;

fn resolve(toml_str: &str) -> Result<Vec<ResolvedCrateConfig>, ResolveError> {
    let cfg: NewAlefConfig = toml::from_str(toml_str).expect("fixture toml must parse");
    cfg.resolve()
}

fn expect_rejected(toml_str: &str, must_contain: &[&str]) {
    let err = resolve(toml_str).expect_err("hazardous config value must be rejected");
    let message = err.to_string();
    for needle in must_contain {
        assert!(
            message.contains(needle),
            "expected error message to contain `{needle}`, got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// jni.crate_dir -- a single flat name; `/`, NUL, and a bare `..` are all rejected.
// ---------------------------------------------------------------------------------------------

#[test]
fn jni_crate_dir_rejects_a_path_separator() {
    expect_rejected(
        r#"
[workspace]
languages = ["jni", "kotlin_android"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.jni]
crate_dir = "../../etc/passwd"
"#,
        &["sample-core", "jni.crate_dir", "path separators are not allowed"],
    );
}

#[test]
fn jni_crate_dir_rejects_a_bare_parent_dir_reference() {
    expect_rejected(
        r#"
[workspace]
languages = ["jni", "kotlin_android"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.jni]
crate_dir = ".."
"#,
        &[
            "sample-core",
            "jni.crate_dir",
            "contains `..`",
            "escape the project root",
        ],
    );
}

#[test]
fn jni_crate_dir_rejects_a_nul_byte() {
    // Routed through the private helper directly rather than a TOML fixture: a NUL byte inside
    // a TOML string is legal input (escaped as `\u0000`) but awkward to spell portably in a
    // source file, and the helper is exactly what `resolve_one` calls, so this exercises the
    // same code path.
    let err = validate_path_segment_field("sample-core", Some("foo\0bar"), "jni.crate_dir")
        .expect_err("a NUL byte must be rejected");
    let message = err.to_string();
    assert!(message.contains("sample-core"));
    assert!(message.contains("jni.crate_dir"));
    assert!(message.contains("NUL byte is not allowed"));
}

// ---------------------------------------------------------------------------------------------
// node.crate_dir / wasm.crate_dir -- documented and tested to hold a full relative path (may
// contain `/` legitimately, e.g. "crates/sample-markdown-node"), so only absolute values and a
// `..` component are rejected -- not `/` itself.
// ---------------------------------------------------------------------------------------------

#[test]
fn node_crate_dir_rejects_an_absolute_value() {
    expect_rejected(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.node]
crate_dir = "/etc/passwd"
"#,
        &[
            "sample-core",
            "node.crate_dir",
            "is absolute",
            "escape the project root",
        ],
    );
}

#[test]
fn node_crate_dir_rejects_a_parent_dir_traversal() {
    expect_rejected(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.node]
crate_dir = "../../../../tmp/pwned"
"#,
        &[
            "sample-core",
            "node.crate_dir",
            "contains `..`",
            "escape the project root",
        ],
    );
}

#[test]
fn wasm_crate_dir_rejects_an_absolute_value() {
    expect_rejected(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.wasm]
crate_dir = "/etc/passwd"
"#,
        &["sample-core", "wasm.crate_dir", "is absolute"],
    );
}

#[test]
fn wasm_crate_dir_rejects_a_parent_dir_traversal() {
    expect_rejected(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.wasm]
crate_dir = ".."
"#,
        &["sample-core", "wasm.crate_dir", "contains `..`"],
    );
}

// ---------------------------------------------------------------------------------------------
// dart.lib_name -- a single flat name (the Dart `library` declaration), same shape as
// jni.crate_dir.
// ---------------------------------------------------------------------------------------------

#[test]
fn dart_lib_name_rejects_a_path_separator() {
    expect_rejected(
        r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.dart]
lib_name = "../../etc/passwd"
"#,
        &["sample-core", "dart.lib_name", "path separators are not allowed"],
    );
}

#[test]
fn dart_lib_name_rejects_a_bare_parent_dir_reference() {
    expect_rejected(
        r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.dart]
lib_name = ".."
"#,
        &["sample-core", "dart.lib_name", "contains `..`"],
    );
}

#[test]
fn dart_lib_name_rejects_a_nul_byte() {
    let err = validate_path_segment_field("sample-core", Some("foo\0bar"), "dart.lib_name")
        .expect_err("a NUL byte must be rejected");
    let message = err.to_string();
    assert!(message.contains("sample-core"));
    assert!(message.contains("dart.lib_name"));
    assert!(message.contains("NUL byte is not allowed"));
}

// ---------------------------------------------------------------------------------------------
// java.package / kotlin.package / kotlin_android.package / csharp.namespace -- dotted values
// turned into nested path segments via `.replace('.', "/")`. A raw `/` or NUL is rejected
// directly; a value that *starts* with `.` turns into a leading `/` after the replace, which
// `PathBuf::join` would treat as an absolute override of the output directory it's joined onto.
// ---------------------------------------------------------------------------------------------

#[test]
fn java_package_rejects_a_raw_path_separator() {
    expect_rejected(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.java]
package = "com/example"
"#,
        &["sample-core", "java.package", "path separators are not allowed"],
    );
}

#[test]
fn java_package_rejects_a_nul_byte() {
    let err = validate_package_like_field("sample-core", Some("com.example\0evil"), "java.package")
        .expect_err("a NUL byte must be rejected");
    let message = err.to_string();
    assert!(message.contains("sample-core"));
    assert!(message.contains("java.package"));
    assert!(message.contains("NUL byte is not allowed"));
}

#[test]
fn java_package_rejects_a_leading_dot_that_becomes_an_absolute_path() {
    // `.hidden`.replace('.', "/") == "/hidden" -- absolute, and `PathBuf::join` discards its
    // base entirely when the joined value is absolute. See `containment_proof_*` below for the
    // underlying `Path::join` mechanics this closes.
    expect_rejected(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.java]
package = ".hidden"
"#,
        &["sample-core", "java.package", "escape the project root"],
    );
}

#[test]
fn kotlin_package_rejects_a_leading_dot_that_becomes_an_absolute_path() {
    expect_rejected(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.kotlin]
package = ".hidden"
"#,
        &["sample-core", "kotlin.package", "escape the project root"],
    );
}

#[test]
fn kotlin_android_package_rejects_a_leading_dot_that_becomes_an_absolute_path() {
    expect_rejected(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = ".hidden"
"#,
        &["sample-core", "kotlin_android.package", "escape the project root"],
    );
}

#[test]
fn csharp_namespace_rejects_a_raw_path_separator() {
    expect_rejected(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = "My/Namespace"
"#,
        &["sample-core", "csharp.namespace", "path separators are not allowed"],
    );
}

#[test]
fn csharp_namespace_rejects_a_bare_all_dots_value() {
    // "..".replace('.', "/") == "//" -- collapses to an absolute root, discarding whatever
    // output directory it would have been joined onto.
    expect_rejected(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = ".."
"#,
        &["sample-core", "csharp.namespace", "escape the project root"],
    );
}

// ---------------------------------------------------------------------------------------------
// crate `name` itself -- unconditional, independent of which languages are configured. A JNI-only
// crate (jni always requires kotlin_android, but neither has an `OutputTemplate` entry that
// embeds the crate name in a single-crate workspace like this fixture) proves the check does not
// depend on the `OutputTemplate` route the other fields incidentally ride along on.
// ---------------------------------------------------------------------------------------------

#[test]
fn crate_name_rejects_a_leading_dot_that_becomes_an_absolute_path_jni_only() {
    // ".hidden".replace('.', "/") == "/hidden" -- absolute, same shape as
    // `java_package_rejects_a_leading_dot_that_becomes_an_absolute_path` above, but here it is
    // the crate `name` itself, in a config that targets only jni + kotlin_android (no other
    // language's `OutputTemplate` entry happens to embed the crate name and catch this first).
    // Note: this failure comes from `validate_output_path` on the dot-replaced value, whose
    // message does not repeat the `label` argument (only `validate_output_segment`'s messages
    // do) -- so, unlike the directly-absolute test below, "name" itself is not asserted here.
    expect_rejected(
        r#"
[workspace]
languages = ["jni", "kotlin_android"]

[[crates]]
name = ".hidden"
sources = ["src/lib.rs"]
"#,
        &[".hidden", "escape the project root"],
    );
}

#[test]
fn crate_name_rejects_a_directly_absolute_value_jni_only() {
    expect_rejected(
        r#"
[workspace]
languages = ["jni", "kotlin_android"]

[[crates]]
name = "/etc/passwd"
sources = ["src/lib.rs"]
"#,
        &["/etc/passwd", "name", "path separators are not allowed"],
    );
}

// ---------------------------------------------------------------------------------------------
// Containment proof: the exact `Path::join` mechanics the fix closes, demonstrated directly
// (not through `resolve()`), so the hazard and the fix are both visible independent of the
// config-resolution plumbing above.
// ---------------------------------------------------------------------------------------------

#[test]
fn containment_proof_an_absolute_value_discards_the_base_dir_via_join() {
    use std::path::{Path, PathBuf};

    // Stands in for `PathBuf::from(&output_dir).join(&package_path)` in the Java/Kotlin/C#
    // backends, where `package_path` is a `java.package`-shaped value after
    // `.replace('.', "/")`.
    let base_dir = PathBuf::from("/safe/output_dir");
    let hostile_package_path = "/etc/passwd_dir";
    #[expect(
        clippy::join_absolute_paths,
        reason = "this regression proves that joining an absolute hostile value discards the safe base"
    )]
    let joined = base_dir.join(hostile_package_path);

    assert_eq!(
        joined,
        PathBuf::from("/etc/passwd_dir"),
        "Path::join must discard `base_dir` entirely when the joined value is absolute -- proving \
         the write would have landed outside /safe/output_dir entirely, not merely nested oddly \
         inside it"
    );

    // And the fix closes exactly this: the same value is rejected before it ever reaches `.join`.
    assert!(crate::core::config::output::validate_output_path(Path::new(hostile_package_path)).is_err());
}

#[test]
fn containment_proof_a_bare_parent_dir_value_traverses_out_via_join() {
    use std::path::{Path, PathBuf};

    // Stands in for `base_dir.join(config.package_dir(lang))` in
    // `cli::pipeline::format::poly_paths` / `cli::pipeline::generate::orphans`, where
    // `package_dir` returns a `node.crate_dir` / `wasm.crate_dir` value verbatim.
    let base_dir = PathBuf::from("/safe/output_dir");
    let hostile_crate_dir = "..";
    let joined = base_dir.join(hostile_crate_dir);

    // `Path::join` does not textually collapse `..` the way it discards an absolute value, but
    // filesystem/OS path resolution treats a trailing `..` as "go up one level" -- this
    // resolves to `/safe`, one level above the intended output tree.
    assert_eq!(joined, PathBuf::from("/safe/output_dir/.."));

    assert!(crate::core::config::output::validate_output_path(Path::new(hostile_crate_dir)).is_err());
}

// ---------------------------------------------------------------------------------------------
// No-change control: ordinary values for every field this fix touches must resolve exactly as
// they did before -- same accepted values, same derived accessors.
// ---------------------------------------------------------------------------------------------

#[test]
fn ordinary_values_for_every_touched_field_still_resolve_unchanged() {
    let resolved = resolve(
        r#"
[workspace]
languages = ["jni", "kotlin_android", "node", "wasm", "dart", "java", "kotlin", "csharp"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.jni]
crate_dir = "sample-core"

[crates.node]
crate_dir = "crates/sample-core-node"

[crates.wasm]
crate_dir = "crates/sample-core-wasm"

[crates.dart]
lib_name = "sample_core"

[crates.java]
package = "dev.sample.core"

[crates.kotlin]
package = "dev.sample.core"

[crates.kotlin_android]
package = "dev.sample.core"

[crates.csharp]
namespace = "Sample.Core"
"#,
    )
    .expect("ordinary config values must still resolve successfully");

    let config = &resolved[0];
    assert_eq!(
        config.jni.as_ref().and_then(|c| c.crate_dir.as_deref()),
        Some("sample-core")
    );
    assert_eq!(
        config.node.as_ref().and_then(|c| c.crate_dir.as_deref()),
        Some("crates/sample-core-node")
    );
    assert_eq!(
        config.wasm.as_ref().and_then(|c| c.crate_dir.as_deref()),
        Some("crates/sample-core-wasm")
    );
    assert_eq!(
        config.dart.as_ref().and_then(|c| c.lib_name.as_deref()),
        Some("sample_core")
    );
    assert_eq!(config.jni_crate_base(), "sample-core");
    assert_eq!(config.java_package(), "dev.sample.core");
    assert_eq!(config.kotlin_package(), "dev.sample.core");
    assert_eq!(config.csharp_namespace(), "Sample.Core");
}
