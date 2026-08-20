use crate::core::backend::GeneratedFile;
use crate::core::config::{FfiTargetDepOverride, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use std::path::PathBuf;

/// Render the `[target.'cfg(...)'.dependencies]` blocks for the JNI crate's
/// core-crate dependency when per-target overrides are configured.
///
/// Returns an empty string when `overrides` is empty (the core dep then lives
/// inline in `[dependencies]`). Otherwise emits a `cfg(not(any(...)))` default
/// branch carrying the full `features` set, plus one `cfg(<override.cfg>)`
/// block per override carrying that override's replacement feature set. This
/// mirrors the FFI scaffolder so the same Android/iOS/Windows gating applies to
/// the cross-compiled JNI shim. The dual-form `render_core_dep` keeps the
/// `version = "..."` requirement so the manifest still publishes cleanly.
fn render_jni_target_blocks(
    crate_name: &str,
    rel_path: &str,
    default_features: &str,
    version: &str,
    overrides: &[FfiTargetDepOverride],
) -> String {
    if overrides.is_empty() {
        return String::new();
    }

    let cfgs: Vec<&str> = overrides.iter().map(|o| o.cfg.as_str()).collect();
    let combined_cfg = if cfgs.len() == 1 {
        cfgs[0].to_owned()
    } else {
        format!("any({})", cfgs.join(", "))
    };

    let mut entries: Vec<(String, String)> = vec![(
        format!("not({combined_cfg})"),
        crate::scaffold::render_core_dep(crate_name, rel_path, default_features, version),
    )];
    for override_ in overrides {
        let features_str = if override_.features.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = override_.features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(", features = [{}]", quoted.join(", "))
        };
        entries.push((
            override_.cfg.clone(),
            crate::scaffold::render_core_dep(crate_name, rel_path, &features_str, version),
        ));
    }
    // See `crate::scaffold::join_sorted_target_dep_blocks`: cargo-sort orders
    // `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
    // predicate string, so the default `not(...)` branch is not always first. ~keep
    let joined = crate::scaffold::join_sorted_target_dep_blocks(entries);
    format!("\n{joined}")
}

/// Scaffold the `<crate>-jni/Cargo.toml` for a JNI shim crate.
///
/// Emits a single `Cargo.toml` as a `cdylib` depending on `jni`, `tokio`,
/// `serde_json`, and `futures-util`.  The `<crate>` dependency path is
/// `../<core-crate-dir>` inside the same workspace; features come from
/// `[crates.kotlin_android] features` if present.
///
/// The output directory is `crates/<jni_crate_base>-jni/`, where
/// `jni_crate_base` is `[crates.jni] crate_dir` when explicitly set,
/// otherwise `config.name`.  This matches the path chosen by
/// `alef-backend-jni::gen_shims::jni_output_path` for `src/lib.rs`.
///
/// Consumers whose `config.name` carries a language suffix can set
/// `[crates.jni] crate_dir` to produce a suffix-free JNI crate — matching every other binding
/// crate — while the umbrella dep entry still uses `config.name` as the Cargo
/// package key with `path = "../<core_crate_dir>"` for the on-disk location.
///
/// When `core_crate_dir` (derived from `sources`) differs from `config.name`
/// — e.g. parser-pack's `name = "parser-language-pack"` with
/// `sources = ["crates/parser-core-core/src/lib.rs"]` — the path dependency on
/// the umbrella crate uses `core_crate_dir` (the directory) while the JNI
/// crate's own directory follows `jni_crate_base`.
pub(crate) fn scaffold_jni(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let core_crate_dir = config.core_crate_dir();
    let jni_crate_name = format!("{}-jni", config.jni_crate_base());
    let jni_lib_name = config.jni_lib_name();

    // Route through `core_dep_features` rather than reading `[crates.kotlin_android] features`
    // directly: that lookup falls back to the top-level `features` when the paired section
    // omits them, which every other binding scaffolder already gets. Reading the section alone
    // left the JNI manifest on the core crate's default features while the generated shim
    // called into feature-gated modules, so the crate could not compile. ~keep
    let features_str = crate::scaffold::core_dep_features(config, Language::KotlinAndroid);

    let umbrella_dep_name = &config.name;

    // `[target.'cfg(...)'.dependencies]` blocks. The default branch is wrapped
    // in `cfg(not(any(...)))` so exactly one variant matches on any build.
    let target_overrides = config
        .jni
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    // Same derivation as the manifest's own location, rather than assuming the core crate is a
    // `crates/` sibling: in a root-flat tree `../{core_crate_dir}` names a directory the emitted
    // tree does not contain, and cargo fails to read it before compiling anything. ~keep
    let rel_path = config.core_crate_dep_path(std::path::Path::new(&format!("crates/{jni_crate_name}")));

    let mut dep_lines: Vec<String> = vec![
        crate::scaffold::render_workspace_dep_or(config, "async-trait", &format!("\"{}\"", tv::cargo::ASYNC_TRAIT)),
        crate::scaffold::render_workspace_dep_or(config, "base64", &format!("\"{}\"", tv::cargo::BASE64)),
        crate::scaffold::render_workspace_dep_or(config, "futures-util", &format!("\"{}\"", tv::cargo::FUTURES_UTIL)),
        crate::scaffold::render_workspace_dep_or(config, "jni", &format!("\"{}\"", tv::cargo::JNI)),
        crate::scaffold::render_workspace_dep_or(config, "serde_json", &format!("\"{}\"", tv::cargo::SERDE_JSON)),
        crate::scaffold::render_workspace_dep_or(
            config,
            "tokio",
            "{ version = \"1\", features = [\"rt-multi-thread\", \"macros\", \"sync\"] }",
        ),
        crate::scaffold::render_workspace_dep_or(config, "tracing", &format!("\"{}\"", tv::cargo::TRACING)),
    ];
    if target_overrides.is_empty() {
        dep_lines.push(crate::scaffold::render_core_dep(
            umbrella_dep_name,
            &rel_path,
            &features_str,
            &api.version,
        ));
    }

    // Capsule types (e.g. tree-sitter's `Language`) make the JNI shim emit
    // `value.into_raw() as *const {into_raw_type}` casts (see
    // `method_capsule_return.rs.jinja`). When `into_raw_type` names a type from an
    // external crate, that crate must be a direct dependency of this manifest or the
    // cast is an unresolved-module compile error. `jni_capsule_types` is the exact
    // filter the JNI backend uses to decide which casts it emits, so reusing it here
    // keeps the declared deps and the emitted casts from drifting apart. ~keep
    for capsule in crate::backends::jni::jni_capsule_types(config).values() {
        let (Some(package), Some(version)) = (capsule.package.as_ref(), capsule.package_version.as_ref()) else {
            continue;
        };
        let dep_prefix = format!("{package} ");
        let dep_dot_prefix = format!("{package}.");
        if dep_lines
            .iter()
            .any(|l| l.starts_with(&dep_prefix) || l.starts_with(&dep_dot_prefix))
        {
            continue;
        }
        dep_lines.push(crate::scaffold::render_workspace_dep_or(
            config,
            package,
            &format!("\"{version}\""),
        ));
    }

    crate::scaffold::sort_dependency_lines(&mut dep_lines);
    let deps_section = dep_lines.join("\n");

    let target_blocks_section = render_jni_target_blocks(
        umbrella_dep_name,
        &rel_path,
        &features_str,
        &api.version,
        target_overrides,
    );

    let lints_section = crate::scaffold::cargo_lints_section(config);
    // Every sibling scaffolder (ffi, python, php, ruby) asks
    // `detect_workspace_inheritance_for_crate` whether a `[workspace.package]` is actually
    // reachable before writing `*.workspace = true`; this one used to assert it. In a
    // root-flat emitted tree there is no workspace root, so `edition.workspace = true` makes
    // the manifest unparseable ("failed to find a workspace root") and every downstream
    // cargo command over the JNI crate dies before compiling a line. ~keep
    let crate_dir = format!("crates/{jni_crate_name}");
    let ws = crate::scaffold::detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let package_header = crate::scaffold::cargo_package_header(
        &jni_crate_name,
        &api.version,
        "2024",
        &crate::scaffold::scaffold_meta(config),
        &ws,
    );
    let content = format!(
        r#"# Generated by alef. Do not edit by hand.

{package_header}

# `base64`, `futures-util`, `serde_json`, and `tokio` are emitted unconditionally below
# so the manifest is stable across regens (they are used when the umbrella
# crate declares binary top-level params, async fns, streaming adapters, or JSON-marshalled types),
# but for an umbrella crate that has none of those they are genuinely unused.
# List them here so `cargo machete` doesn't flag the no-async-no-streaming
# case as a real finding.
[package.metadata.cargo-machete]
ignored = ["async-trait", "base64", "futures-util", "serde_json", "tokio", "tracing"]

[lib]
name = "{jni_lib_name}"
crate-type = ["cdylib"]

[dependencies]
{deps_section}
{target_blocks_section}{lints_section}"#,
        package_header = package_header,
        jni_lib_name = jni_lib_name,
        lints_section = lints_section,
        deps_section = deps_section,
        target_blocks_section = target_blocks_section,
    );

    let _ = api;

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{jni_crate_name}/Cargo.toml")),
        content,
        generated_header: true,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::ApiSurface;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// A tree with no reachable `[workspace.package]` must get explicit `[package]` values.
    ///
    /// The JNI manifest used to hard-code `version.workspace = true` / `edition.workspace = true`
    /// / `license.workspace = true`. In a root-flat emitted tree there is no workspace root to
    /// inherit from, so cargo refused the manifest outright ("error inheriting `edition` from
    /// workspace root manifest") and every downstream command over the JNI crate -- clippy,
    /// build, test -- failed before compiling anything. Every sibling scaffolder already routed
    /// this through `detect_workspace_inheritance_for_crate`; only this one asserted. ~keep
    #[test]
    fn scaffold_jni_cargo_toml_writes_explicit_package_fields_without_a_workspace() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );

        let cargo_toml = &scaffold_jni(&ApiSurface::default(), &config).unwrap()[0].content;

        assert!(
            cargo_toml.contains("edition = \"2024\""),
            "without a workspace root the edition must be written explicitly; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains(".workspace = true"),
            "no [package] field may claim workspace inheritance when no workspace is reachable; got:\n{cargo_toml}"
        );
        // The same tree shape breaks the core dependency for the same reason: `../{core}` names
        // a `crates/<core>` sibling that a root-flat tree does not have, so cargo could not read
        // the manifest it points at. ~keep
        let expected = config.core_crate_dep_path(std::path::Path::new("crates/demo-llm-jni"));
        assert_eq!(expected, "../..", "sanity: root-flat core crate sits two levels up");
        assert!(
            cargo_toml.contains(&format!(r#"path = "{expected}""#)),
            "the core dep path must be derived from the emitted layout; got:\n{cargo_toml}"
        );
    }

    /// The JNI trait-bridge glue (`trait_bridge_method_body.rs.jinja`) logs swallowed
    /// host-callback failures via `tracing::warn!`. Since this Cargo.toml is scaffolded
    /// once (`generated_header: false`) and not regenerated once trait bridges are
    /// configured later, `tracing` must be declared unconditionally up front.
    #[test]
    fn scaffold_jni_cargo_toml_declares_tracing_dependency() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains(&format!(
                "tracing = \"{}\"",
                crate::core::template_versions::cargo::TRACING
            )),
            "JNI Cargo.toml must declare the tracing dependency; got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains("\"tracing\""),
            "JNI Cargo.toml must ignore tracing in cargo-machete since it may be unused until trait bridges are configured; got:\n{cargo_toml}"
        );
    }

    /// `[crates.cargo_lints]` must round-trip into the emitted JNI `Cargo.toml` as a
    /// `[lints.rust]` / `[lints.clippy]` block, with a configured `clippy` entry for a
    /// non-builtin key surviving alongside the builtin deny defaults. The JNI template
    /// builds the rest of its manifest from a hand-written literal, so this exercises the
    /// splice against a differently shaped template than the other binding crates.
    #[test]
    fn scaffold_jni_cargo_toml_emits_configured_cargo_lints() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.clippy]
unwrap_used = "warn"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains("[lints.rust]\nunused_must_use = \"deny\""),
            "expected [lints.rust] block, got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(
                "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\n\
                 print_stdout = \"deny\"\nunwrap_used = \"warn\""
            ),
            "expected the configured clippy entry to merge with the builtin deny defaults, got:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(cargo_toml).expect("generated Cargo.toml with cargo_lints must be valid TOML");
    }

    /// Absence of `[crates.cargo_lints]` must still emit the built-in `[lints.clippy]` deny
    /// block; no `[lints.rust]` table is emitted since nothing configures it.
    #[test]
    fn scaffold_jni_cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            !cargo_toml.contains("[lints.rust]"),
            "no [lints.rust] table should be emitted when cargo_lints.rust is unset, got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml
                .contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
            "the builtin [lints.clippy] deny block must survive even when cargo_lints is unset, got:\n{cargo_toml}"
        );
    }

    #[test]
    fn scaffold_jni_inherits_declared_workspace_dependencies() {
        let workspace = tempfile::tempdir().expect("create workspace");
        std::fs::write(
            workspace.path().join("Cargo.toml"),
            r#"
[workspace]
members = []

[workspace.dependencies]
base64 = "0.22"
jni = "0.22"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
"#,
        )
        .expect("write workspace manifest");
        let mut config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );
        config.workspace_root = Some(workspace.path().to_path_buf());

        let files = scaffold_jni(&ApiSurface::default(), &config).expect("scaffold JNI");
        let cargo_toml = &files[0].content;

        assert!(cargo_toml.contains("base64.workspace = true"));
        assert!(cargo_toml.contains("jni.workspace = true"));
        assert!(cargo_toml.contains("tokio.workspace = true"));
        assert!(cargo_toml.contains("async-trait = \"0.1\""));
        assert!(!cargo_toml.contains("base64 = \"0.22\""));
    }

    /// The scaffolded `[lib] name` must match what the Kotlin Bridge emits in
    /// `System.loadLibrary(...)`.  When `[ffi] prefix` is set, both must use the
    /// prefix-derived name rather than the snake-cased package name.
    #[test]
    fn scaffold_jni_lib_name_uses_ffi_prefix() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demoffi"

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains("name = \"demoffi_jni\""),
            "expected `name = \"demoffi_jni\"` but got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("name = \"demo_llm_jni\""),
            "cdylib name must not fall back to snake-cased crate name when prefix is set; got:\n{cargo_toml}"
        );
    }

    /// When `config.name` differs from `core_crate_dir()` (e.g. parser-pack's
    /// `name = "parser-language-pack"` with sources under
    /// `crates/parser-core-core/`), the JNI scaffold must place its output at
    /// `crates/<config.name>-jni/Cargo.toml` to match the path that
    /// `alef-backend-jni::gen_shims` uses for `src/lib.rs`, and the umbrella
    /// dep entry must use the cargo package name as the dep key while the
    /// `path = "../..."` value references the on-disk directory.
    #[test]
    fn scaffold_jni_path_uses_config_name_not_core_crate_dir() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.sample_language_pack.android"
namespace = "dev.sample_crate.sample_language_pack.android"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        assert_eq!(files.len(), 1);
        let path = files[0].path.to_string_lossy();
        let cargo_toml = &files[0].content;

        assert!(files[0].generated_header, "generated JNI manifest must be Alef-owned");
        assert_eq!(
            path, "crates/sample-language-pack-jni/Cargo.toml",
            "JNI scaffold path must follow config.name, not core_crate_dir; got: {path}"
        );
        assert!(
            cargo_toml.contains("name = \"sample-language-pack-jni\""),
            "[package] name must follow config.name; got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains("sample-language-pack = { path = \"../sample-pack-core\""),
            "umbrella dep key must be cargo package name with path = ../<core_crate_dir>; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("sample-pack-core = { path = \"../sample-pack-core\""),
            "umbrella dep key must NOT be the directory name; got:\n{cargo_toml}"
        );
    }

    /// Without an explicit `[ffi] prefix`, the lib name must still be the
    /// snake-cased crate name (regression guard for the default case).
    #[test]
    fn scaffold_jni_lib_name_defaults_to_snake_case_crate_name() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "plain-pkg"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.plain"
namespace = "dev.sample_crate.plain"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains("name = \"plain_pkg_jni\""),
            "expected `name = \"plain_pkg_jni\"` for default case; got:\n{cargo_toml}"
        );
    }

    /// When `[crates.jni] crate_dir` is set, the JNI scaffold uses the
    /// override for both the crate directory and `[package] name`, while the
    /// umbrella dep key remains `config.name` (the Cargo package name) with
    /// `path = "../<core_crate_dir>"`.
    ///
    /// This covers a suffixed package name with a suffix-free core crate directory:
    /// the JNI crate lands at the configured `crate_dir` path rather than keeping
    /// the package suffix in its own crate name.
    #[test]
    fn scaffold_jni_crate_dir_override_controls_output_path() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-render-rs"
sources = ["crates/demo-render/src/lib.rs"]

[crates.jni]
crate_dir = "demo-render"

[crates.kotlin_android]
package = "dev.example.demo_render.android"
namespace = "dev.example.demo_render.android"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        assert_eq!(files.len(), 1);
        let path = files[0].path.to_string_lossy();
        let cargo_toml = &files[0].content;

        assert_eq!(
            path, "crates/demo-render-jni/Cargo.toml",
            "JNI scaffold path must follow [crates.jni] crate_dir override; got: {path}"
        );
        assert!(
            cargo_toml.contains("name = \"demo-render-jni\""),
            "[package] name must follow crate_dir override; got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains("demo-render-rs = { path = \"../demo-render\""),
            "umbrella dep key must be cargo package name, path must be core_crate_dir; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("demo-render = { path = \"../demo-render\""),
            "umbrella dep key must NOT be the crate_dir override; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("demo-render-rs-jni"),
            "crate name must NOT contain the -rs suffix; got:\n{cargo_toml}"
        );
    }

    /// Regression guard: the JNI `Cargo.toml` `[dependencies]` table must be
    /// emitted in the order `cargo sort --check` requires, so the `cargo-sort`
    /// prek hook does not rewrite the file on every regen. The umbrella dep is
    /// named after `config.name`, so its placement depends on the consumer crate
    /// name.
    ///
    /// The check goes through `assert_dependency_keys_sorted`, which re-parses the
    /// manifest with `toml_edit` and so runs cargo-sort's own comparison. An
    /// earlier version of this test derived the key itself with
    /// `line.split('=').next()`, which yields the DOTTED key `tracing.workspace` --
    /// the same key the emitter was sorting on -- making the assertion a tautology
    /// that kept passing while consumers failed. ~keep
    #[test]
    fn scaffold_jni_dependency_keys_are_in_cargo_sort_order() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "sample_stream"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.sample_stream"
namespace = "dev.example.sample_stream"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            crate::test_support::cargo_sort_order::assert_dependency_keys_sorted("jni Cargo.toml", cargo_toml) > 0,
            "the JNI manifest must carry dependency keys to compare:\n{cargo_toml}"
        );
    }

    /// The field failure, end to end. `render_workspace_dep_or` is the only code path in
    /// alef that puts a DOTTED key into a `[dependencies]` table, so this emitter is where
    /// the raw-line-text sort first disagreed with cargo-sort: the inherited capsule package
    /// `alpha-parser` is emitted as `alpha-parser.workspace = true` while the core crate,
    /// whose name extends it with a `-` suffix, is a plain path dependency. `-` (0x2D) sorts
    /// before `.` (0x2E), so byte-wise line comparison emits the core crate first, while
    /// cargo-sort compares `alpha-parser` against `alpha-parser-pack` and wants the shorter
    /// name first. One dependency crossing that boundary fails
    /// `cargo sort --check --workspace` for the whole crate. ~keep
    #[test]
    fn scaffold_jni_orders_an_inherited_capsule_package_before_its_hyphen_extended_core_crate() {
        let workspace = tempfile::tempdir().expect("create workspace root");
        std::fs::write(
            workspace.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.dependencies]\nalpha-parser = \"1.0\"\n",
        )
        .expect("write workspace manifest");

        let mut config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "alpha-parser-pack"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.alpha"
namespace = "dev.example.alpha"

[crates.kotlin_android.capsule_types.Grammar]
host_type = "org.example.Grammar"

[crates.ffi.capsule_types.Grammar]
into_raw_type = "alpha_parser::ffi::Grammar"
c_return_type = "Grammar"
package = "alpha-parser"
package_version = "1.0"
"#,
        );
        config.workspace_root = Some(workspace.path().to_path_buf());

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        let inherited = cargo_toml.find("alpha-parser.workspace = true").unwrap_or_else(|| {
            panic!("fixture must inherit the capsule package as a dotted key, or it proves nothing:\n{cargo_toml}")
        });
        let core = cargo_toml.find("alpha-parser-pack = ").unwrap_or_else(|| {
            panic!("fixture must emit the core crate as a plain key, or it proves nothing:\n{cargo_toml}")
        });
        assert!(
            inherited < core,
            "`alpha-parser.workspace` must precede `alpha-parser-pack`:\n{cargo_toml}"
        );
        crate::test_support::cargo_sort_order::assert_dependency_keys_sorted("jni Cargo.toml", cargo_toml);
    }

    /// When `[crates.jni] target_dep_overrides` are configured, the core-crate
    /// dependency must move out of the inline `[dependencies]` table into per-cfg
    /// `[target.'cfg(...)'.dependencies]` blocks: a `cfg(not(any(...)))` default
    /// branch carrying the full feature set, plus one block per override carrying
    /// its replacement features. Mirrors the FFI gating so the cross-compiled JNI
    /// shim drops ORT / native-C features on Android, iOS, and Windows.
    #[test]
    fn scaffold_jni_emits_target_dep_overrides() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "ios"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        let deps_start = cargo_toml.find("[dependencies]").expect("missing [dependencies]");
        let first_target = cargo_toml
            .find("[target.")
            .expect("expected at least one [target.*] block");
        let inline_deps = &cargo_toml[deps_start..first_target];
        assert!(
            !inline_deps.contains("demo-doc ="),
            "core-crate dep must not appear inline when overrides are present; got:\n{cargo_toml}"
        );

        // Default branch is gated on cfg(not(any(...))) and carries `full`.
        assert!(
            cargo_toml.contains(
                r#"[target.'cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))'.dependencies]"#
            ),
            "default branch must be gated on cfg(not(any(...))); got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(r#"[target.'cfg(target_os = "android")'.dependencies]"#)
                && cargo_toml.contains(r#"features = ["android-target"]"#),
            "android override block must carry android-target; got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(r#"[target.'cfg(target_os = "windows")'.dependencies]"#)
                && cargo_toml.contains(r#"features = ["windows-target"]"#),
            "windows override block must carry windows-target; got:\n{cargo_toml}"
        );
        let core_dep_lines = cargo_toml.matches("demo-doc = {").count();
        assert_eq!(
            core_dep_lines, 4,
            "expected one core-dep line per target branch (default + 3 overrides); got {core_dep_lines}:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
    }

    /// Regression test: `cargo-sort` (and hence `poly lint`) orders
    /// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
    /// predicate string (plain byte-wise comparison), NOT with the default
    /// `cfg(not(any(...)))` branch always first. An `all(...)` override (e.g.
    /// the macOS-Intel target a downstream consumer actually configures) sorts
    /// *before* `not(...)` — `'a'` < `'n'` — so emitting the default branch
    /// unconditionally first produces an unsorted manifest that
    /// `cargo sort --check` rejects. This reproduces a downstream consumer's
    /// real `crates/acme-jni/Cargo.toml` override set (android/ios/windows plus a
    /// macOS-Intel `all(...)` override) and asserts the `all(...)` block comes
    /// before the `not(...)` default block in the emitted text.
    #[test]
    fn scaffold_jni_target_dep_overrides_sort_all_before_not() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "ios"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'all(target_os = "macos", target_arch = "x86_64")'
features = ["macos-intel-target"]
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        let all_pos = cargo_toml
            .find(r#"[target.'cfg(all(target_os = "macos", target_arch = "x86_64"))'.dependencies]"#)
            .expect("expected the macOS-Intel `all(...)` override block");
        let not_pos = cargo_toml
            .find("[target.'cfg(not(any(")
            .expect("expected the default `not(any(...))` block");
        let android_pos = cargo_toml
            .find(r#"[target.'cfg(target_os = "android")'.dependencies]"#)
            .expect("expected the android override block");

        assert!(
            all_pos < not_pos,
            "the `all(...)` override must sort BEFORE the `not(...)` default branch \
             (cargo-sort compares raw cfg predicate strings byte-wise: 'a' < 'n'); got:\n{cargo_toml}"
        );
        assert!(
            not_pos < android_pos,
            "the `not(...)` default branch must sort before `target_os = \"android\"` \
             ('n' < 't'); got:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
    }

    /// Without `target_dep_overrides`, the core-crate dep stays inline in
    /// `[dependencies]` and no `[target.*]` blocks are emitted (regression guard
    /// for the default path).
    #[test]
    fn scaffold_jni_no_target_blocks_without_overrides() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;
        assert!(
            !cargo_toml.contains("[target."),
            "no [target.*] blocks without overrides; got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains("demo-doc = {") && cargo_toml.contains(r#"features = ["full"]"#),
            "core-crate dep must stay inline with full features; got:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
    }

    /// Regression guard: cargo-sort orders sub-tables of `[package]`
    /// (`[package.metadata.cargo-machete]`) directly after the `[package]`
    /// section and before `[lib]` / `[dependencies]`. Emitting `[lib]` or
    /// `[dependencies]` before the metadata sub-table causes cargo-sort to
    /// rewrite the file on every regen.
    #[test]
    fn scaffold_jni_section_order_matches_cargo_sort() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );
        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        let pkg = cargo_toml.find("[package]").expect("missing [package]");
        let meta = cargo_toml
            .find("[package.metadata.cargo-machete]")
            .expect("missing [package.metadata.cargo-machete]");
        let lib = cargo_toml.find("[lib]").expect("missing [lib]");
        let deps = cargo_toml.find("[dependencies]").expect("missing [dependencies]");
        assert!(
            pkg < meta && meta < lib && lib < deps,
            "section order must be [package] < [package.metadata.cargo-machete] < [lib] < [dependencies]; got:\n{cargo_toml}"
        );
    }

    /// The generated JNI shim calls whatever the core crate's configured feature set exposes,
    /// so its manifest must request that same set. Reading `[crates.kotlin_android] features`
    /// alone left the dependency on the core crate's defaults whenever the paired section
    /// omitted the key, and the shim then failed to compile against feature-gated modules.
    #[test]
    fn scaffold_jni_core_dep_inherits_top_level_features_when_kotlin_android_omits_them() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]
features = ["native-http", "full"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains(r#"features = ["native-http", "full"]"#),
            "core dep must inherit the top-level feature set; got:\n{cargo_toml}"
        );
    }

    /// An explicit `[crates.kotlin_android] features` still wins over the top-level set, so
    /// the inheritance above is a fallback rather than an override.
    #[test]
    fn scaffold_jni_core_dep_prefers_explicit_kotlin_android_features() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]
features = ["native-http", "full"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
features = ["android-http"]
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains(r#"features = ["android-http"]"#),
            "explicit kotlin_android features must win; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("native-http"),
            "top-level features must not leak in when overridden; got:\n{cargo_toml}"
        );
    }

    /// A capsule type configured in BOTH `[crates.ffi.capsule_types]` and
    /// `[crates.kotlin_android.capsule_types]` makes the JNI shim emit a
    /// `value.into_raw() as *const {into_raw_type}` cast (see
    /// `jni_capsule_types`/`method_capsule_return.rs.jinja`), so the crate providing
    /// `into_raw_type` must be a direct dependency or the cast is an unresolved-module
    /// compile error. With no workspace-level entry for the package, the manifest must
    /// fall back to a literal pinned version rather than an unresolved `.workspace = true`.
    #[test]
    fn scaffold_jni_declares_capsule_package_dependency_when_cast_is_emitted() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-lang-pack"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"

[crates.kotlin_android.capsule_types.Language]
host_type = "org.example.TSLanguage"

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
package = "tree-sitter"
package_version = "0.26"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains("tree-sitter = \"0.26\""),
            "JNI Cargo.toml must declare the capsule package dependency backing \
             `tree_sitter::ffi::TSLanguage`; got:\n{cargo_toml}"
        );
        toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
    }

    /// When the consumer's root `Cargo.toml` already declares the capsule package under
    /// `[workspace.dependencies]`, the JNI manifest must inherit it (`pkg.workspace = true`)
    /// like every other JNI dependency, rather than pinning a second, possibly divergent
    /// version literal.
    #[test]
    fn scaffold_jni_capsule_package_dependency_inherits_workspace_entry() {
        let workspace = tempfile::tempdir().expect("create workspace");
        std::fs::write(
            workspace.path().join("Cargo.toml"),
            r#"
[workspace]
members = []

[workspace.dependencies]
tree-sitter = "0.26"
"#,
        )
        .expect("write workspace manifest");
        let mut config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-lang-pack"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"

[crates.kotlin_android.capsule_types.Language]
host_type = "org.example.TSLanguage"

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
package = "tree-sitter"
package_version = "0.26"
"#,
        );
        config.workspace_root = Some(workspace.path().to_path_buf());

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            cargo_toml.contains("tree-sitter.workspace = true"),
            "JNI Cargo.toml must inherit the workspace-declared capsule package; got:\n{cargo_toml}"
        );
        assert!(
            !cargo_toml.contains("tree-sitter = \"0.26\""),
            "must not also pin a literal version when a workspace entry exists; got:\n{cargo_toml}"
        );
    }

    /// A capsule type declared only in `[crates.ffi.capsule_types]` — with no matching
    /// entry in `[crates.kotlin_android.capsule_types]` — is one the JNI backend never
    /// emits a cast for (see `jni_capsule_types`'s intersection filter). The manifest
    /// must not declare the package dependency in that case: an unused dep is exactly
    /// the false-positive `cargo machete` would need a fresh `ignored` entry to suppress.
    #[test]
    fn scaffold_jni_omits_capsule_package_dependency_when_kotlin_android_lacks_the_type() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-lang-pack"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
package = "tree-sitter"
package_version = "0.26"
"#,
        );

        let api = ApiSurface::default();
        let files = scaffold_jni(&api, &config).unwrap();
        let cargo_toml = &files[0].content;

        assert!(
            !cargo_toml.contains("tree-sitter"),
            "capsule package dep must not be emitted when kotlin_android has no matching \
             capsule_types entry (the JNI backend never emits the cast in that case); got:\n{cargo_toml}"
        );
    }
}
