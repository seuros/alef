use super::{OutputLayout, default_binding_crate_root, resolve_output_layout};
use std::path::PathBuf;

/// The one formula every scaffolder's hard-coded `crates/{crate}-<suffix>` manifest path,
/// `OutputTemplate::resolve`'s single-crate default, and `package_dir`'s no-override
/// formula for Node/Wasm must all agree on. Table-driven so a language added to one side
/// without the other shows up here as a wrong answer instead of a silent gap.
#[test]
fn default_binding_crate_root_matches_every_scaffolder_suffix() {
    let cases: &[(&str, Option<&str>)] = &[
        ("python", Some("crates/toolkit-py")),
        ("node", Some("crates/toolkit-node")),
        ("php", Some("crates/toolkit-php")),
        ("ffi", Some("crates/toolkit-ffi")),
        ("wasm", Some("crates/toolkit-wasm")),
        ("ruby", None),
        ("elixir", None),
        ("go", None),
        ("swift", None),
    ];

    for (lang, expected) in cases {
        assert_eq!(
            default_binding_crate_root("toolkit", lang),
            expected.map(str::to_string),
            "default_binding_crate_root(\"toolkit\", {lang:?})"
        );
    }
}

/// Every shape a resolved binding output path takes in the wild, and the crate root and
/// source directory each one implies.
///
/// The `src`-suffixed rows are the shape all four consumer repos spell out in
/// `[crates.output]`; the crate-root rows are the shape `OutputTemplate::resolve` produces
/// when a project configures nothing. Both must resolve, and the crate root must never
/// climb above the configured path in the second case.
#[test]
fn output_layout_derives_the_crate_root_from_the_path_shape() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "crates/toolkit-wasm/src/",
            "crates/toolkit-wasm",
            "crates/toolkit-wasm/src/",
        ),
        (
            "crates/toolkit-wasm/src",
            "crates/toolkit-wasm",
            "crates/toolkit-wasm/src",
        ),
        ("packages/wasm", "packages/wasm", "packages/wasm/src"),
        ("packages/wasm/", "packages/wasm/", "packages/wasm/src"),
        (
            "packages/ruby/ext/toolkit_rb/src",
            "packages/ruby/ext/toolkit_rb",
            "packages/ruby/ext/toolkit_rb/src",
        ),
        ("src", "", "src"),
    ];

    for (output_dir, expected_root, expected_src) in cases {
        let layout = OutputLayout::from_output_dir(output_dir);
        assert_eq!(
            layout.root,
            PathBuf::from(expected_root),
            "crate root for output dir `{output_dir}`"
        );
        assert_eq!(
            layout.src,
            PathBuf::from(expected_src),
            "source dir for output dir `{output_dir}`"
        );
    }
}

/// The crate root may never be an ancestor of the configured output path unless the
/// configured path named a `src` directory. This is the property that failed: a
/// crate-root-shaped path resolved its root one level up, so the manifest landed beside
/// its sibling packages instead of inside its own.
#[test]
fn output_layout_never_escapes_a_crate_root_shaped_path() {
    for output_dir in [
        "packages/wasm",
        "packages/ffi",
        "packages/php",
        "packages/deeply/nested/pkg",
    ] {
        let layout = OutputLayout::from_output_dir(output_dir);
        assert_eq!(layout.root, PathBuf::from(output_dir), "root for `{output_dir}`");
        assert!(
            layout.src.starts_with(&layout.root),
            "source dir `{}` must live inside crate root `{}`",
            layout.src.display(),
            layout.root.display()
        );
    }
}

/// With no `[crates.output]` entry the backend's own default string is used, and `{name}`
/// is substituted exactly as `resolve_output_dir` does.
#[test]
fn resolve_output_layout_falls_back_to_the_backend_default() {
    let layout = resolve_output_layout(None, "toolkit", "crates/{name}-wasm/src/");

    assert_eq!(layout.root, PathBuf::from("crates/toolkit-wasm"));
    assert_eq!(layout.src, PathBuf::from("crates/toolkit-wasm/src/"));
}

/// A configured path wins over the default, and `{name}` is substituted in it too.
#[test]
fn resolve_output_layout_prefers_the_configured_path() {
    let configured = PathBuf::from("bindings/{name}-wasm");
    let layout = resolve_output_layout(Some(&configured), "toolkit", "crates/{name}-wasm/src/");

    assert_eq!(layout.root, PathBuf::from("bindings/toolkit-wasm"));
    assert_eq!(layout.src, PathBuf::from("bindings/toolkit-wasm/src"));
}
