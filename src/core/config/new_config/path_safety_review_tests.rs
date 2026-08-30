use super::*;

fn expect_path_rejected(config: &str, field: &str) {
    let parsed: NewAlefConfig = toml::from_str(config).expect("fixture must parse");
    let error = parsed
        .resolve()
        .expect_err("escaping path must be rejected")
        .to_string();
    assert!(error.contains(field), "expected `{field}` context, got: {error}");
    assert!(error.contains("escape the project root"), "unexpected error: {error}");
}

#[test]
fn explicit_output_rejects_windows_drive_paths_on_every_host() {
    expect_path_rejected(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.output]
node = 'C:\outside'
"#,
        "output.node",
    );
}

#[test]
fn explicit_output_rejects_backslash_parent_traversal_on_every_host() {
    expect_path_rejected(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.output]
node = '..\outside'
"#,
        "output.node",
    );
}

#[test]
fn scaffold_output_rejects_parent_traversal() {
    expect_path_rejected(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.python]
scaffold_output = "../outside"
"#,
        "python.scaffold_output",
    );
}

#[test]
fn output_template_errors_are_contextual_instead_of_panicking() {
    expect_path_rejected(
        r#"
[workspace]
languages = ["node"]

[workspace.output_template]
node = "../outside/{crate}"

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
"#,
        "output.node",
    );
}
