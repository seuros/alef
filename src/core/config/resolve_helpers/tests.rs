use super::{
    OutputLayout, default_binding_crate_root, java_project_root, kotlin_project_root, relative_slash_path,
    resolve_output_layout, strip_trailing_components,
};
use std::path::{Path, PathBuf};

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

/// A binding crate always sits under `crates/{crate}-<lang>` -- two path components below
/// the project root -- in both a root-flat core crate (root is the project root itself,
/// an empty path) and a workspace-shaped one (root is a `crates/` sibling). The two shapes
/// need a different number of `..` segments, which is exactly what the old hard-coded
/// `../{core_crate_dir}` formula got wrong for the root-flat case.
#[test]
fn relative_slash_path_handles_root_flat_and_workspace_core_crates() {
    let binding_root = Path::new("crates/toolkit-ffi");

    assert_eq!(
        relative_slash_path(binding_root, Path::new("")),
        "../..",
        "root-flat core crate: Cargo.toml sits at the project root"
    );
    assert_eq!(
        relative_slash_path(binding_root, Path::new("crates/toolkit-core")),
        "../toolkit-core",
        "workspace-shaped core crate: Cargo.toml sits beside the binding crate"
    );
}

#[test]
fn relative_slash_path_collapses_to_dot_for_identical_roots() {
    let same = Path::new("crates/toolkit-ffi");
    assert_eq!(relative_slash_path(same, same), ".");
}

#[test]
fn strip_trailing_components_matches_and_strips_a_tail() {
    assert_eq!(
        strip_trailing_components(
            Path::new("sdk/java/src/main/java/dev/toolkit"),
            Path::new("src/main/java/dev/toolkit")
        ),
        Some(PathBuf::from("sdk/java"))
    );
}

#[test]
fn strip_trailing_components_returns_none_on_mismatch() {
    assert_eq!(
        strip_trailing_components(Path::new("sdk/java/dev/toolkit"), Path::new("src/main/java")),
        None
    );
}

#[test]
fn strip_trailing_components_returns_dot_when_suffix_consumes_the_whole_path() {
    assert_eq!(
        strip_trailing_components(Path::new("src/main/java"), Path::new("src/main/java")),
        Some(PathBuf::from("."))
    );
}

/// The three shapes that matter for `java_project_root`: the unconfigured/bare default, the
/// full Maven `src/main/java/<pkg>/` source path (what `tslp` configures), and a package-path
/// leaf with no `src/main/java` infix at all. All three must resolve to the same project root
/// a real `pom.xml` would live in, matching exactly what `JavaBackend`'s own `ends_with`
/// disambiguation places the sources under.
#[test]
fn java_project_root_resolves_every_configured_shape() {
    let cases: &[(&str, &str, &str)] = &[
        ("packages/java", "dev/toolkit", "packages/java"),
        ("packages/java/", "dev/toolkit", "packages/java"),
        ("sdk/java/src/main/java/", "dev/toolkit", "sdk/java"),
        ("sdk/java/dev/toolkit", "dev/toolkit", "sdk/java"),
    ];
    for (output_dir, package_path, expected_root) in cases {
        assert_eq!(
            java_project_root(output_dir, package_path),
            PathBuf::from(expected_root),
            "java_project_root({output_dir:?}, {package_path:?})"
        );
    }
}

/// `kotlin_project_root` must follow the same presence-based branch the Kotlin backend's own
/// generator uses (`explicit_output.kotlin.is_some()`): unconfigured always resolves to the
/// output path itself (the project root sources are written under `src/main/kotlin/<pkg>`
/// inside), while a configured path strips the source-set suffix when present and otherwise
/// passes through unchanged.
#[test]
fn kotlin_project_root_resolves_every_configured_shape() {
    assert_eq!(
        kotlin_project_root("packages/kotlin", "dev/toolkit", false),
        PathBuf::from("packages/kotlin"),
        "unconfigured output is already the project root"
    );
    assert_eq!(
        kotlin_project_root("sdk/kotlin", "dev/toolkit", true),
        PathBuf::from("sdk/kotlin"),
        "a configured bare root with no source-set suffix passes through unchanged"
    );
    assert_eq!(
        kotlin_project_root("sdk/kotlin/src/main/kotlin/dev/toolkit", "dev/toolkit", true),
        PathBuf::from("sdk/kotlin"),
        "a configured full source-set path resolves to the project root above it"
    );
}
