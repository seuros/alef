//! A configured *aggregate* core-crate feature must satisfy the gates of the members it enables.
//!
//! `core::ir::cfg_feature_satisfied` matches `feature = "X"` leaves literally and hard-codes
//! exactly one universal umbrella (`full`). Every other aggregate name — the shape a core crate
//! declares as `bundle-target = ["gated", ...]` — therefore satisfies no gate at all. A binding
//! configured with that aggregate compiles the gated items (cargo resolves the aggregate through
//! the core crate's own `[features]` table) but alef's cfg filter judged every
//! `#[cfg(feature = "gated")]` item unsatisfied and dropped it from the surface. The result is a
//! binding that silently exposes less API than the artifact it wraps, with no build error to
//! notice.
//!
//! `codegen::cfg::expand_configured_features` resolves the configured list through that same
//! `[features]` table. These tests pin the expansion at every site that feeds a configured
//! feature list into gate evaluation, one test per site, so a site that regresses to literal
//! matching fails here rather than in a consumer's shipped package.

use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

/// The gated function's Rust name. Every backend renames it (`GatedCall`, `gatedCall`, ...), so
/// assertions compare against [`normalize`]d text rather than this spelling verbatim.
const GATED_FN: &str = "aggregate_gated_call";

/// An ungated sibling. Its presence proves the backend generated anything at all, so a test
/// cannot pass by asserting over empty output.
const PLAIN_FN: &str = "always_present_call";

/// The core-crate aggregate a consumer configures for a binding.
const AGGREGATE: &str = "bundle-target";

/// The leaf feature `AGGREGATE` enables, and the one [`GATED_FN`]'s `#[cfg]` names.
const GATED_FEATURE: &str = "gated";

/// Lowercase and drop `_`/`-` so one needle matches `GatedCall`, `gatedCall` and
/// `aggregate_gated_call` alike.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn contains_symbol(files: &[alef::core::backend::GeneratedFile], symbol: &str) -> bool {
    let needle = normalize(symbol);
    files.iter().any(|file| normalize(&file.content).contains(&needle))
}

/// A core crate whose `[features]` table declares `AGGREGATE` as an alias for `GATED_FEATURE`.
///
/// Returned as the owning `TempDir` so the caller keeps it alive for the whole test — the
/// expansion reads this manifest off disk at generation time.
fn core_crate_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let core_dir = dir.path().join("crates").join("sample-core");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"sample-core\"\n\n[features]\ndefault = []\n\
             {AGGREGATE} = [\"{GATED_FEATURE}\"]\n{GATED_FEATURE} = []\nunrelated = []\n"
        ),
    )
    .expect("write core Cargo.toml");
    dir
}

/// Resolve `toml` and point the result at the [`core_crate_workspace`] fixture so
/// `expand_configured_features` can find the manifest.
fn config_for(toml: &str, workspace: &tempfile::TempDir) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("fixture config must parse");
    let mut resolved = cfg.resolve().expect("fixture config must resolve").remove(0);
    resolved.workspace_root = Some(workspace.path().to_path_buf());
    resolved.sources = vec![std::path::PathBuf::from("crates/sample-core/src/lib.rs")];
    resolved
}

fn free_function(name: &str, cfg: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        return_type: TypeRef::String,
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn gated_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![
            free_function(PLAIN_FN, None),
            free_function(GATED_FN, Some(&format!("feature = \"{GATED_FEATURE}\""))),
        ],
        ..Default::default()
    }
}

/// Assert `backend` emitted both functions: the ungated one (proving the run produced output)
/// and the aggregate-gated one (the behaviour under test).
fn assert_gated_function_survives(backend: &dyn Backend, config: &ResolvedCrateConfig, site: &str) {
    let api = gated_api();
    let files = backend
        .generate_bindings(&api, config)
        .unwrap_or_else(|error| panic!("{site}: generation failed: {error}"));
    assert!(
        contains_symbol(&files, PLAIN_FN),
        "{site}: the ungated function is missing, so this check examined nothing"
    );
    assert!(
        contains_symbol(&files, GATED_FN),
        "{site}: `{GATED_FN}` is gated behind `{GATED_FEATURE}`, which the configured aggregate \
         `{AGGREGATE}` enables in the core crate's [features] table — it must survive cfg filtering"
    );
}

#[test]
fn go_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.go]
module = "github.com/sample/sample-core"
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::go::GoBackend, &config, "go");
}

#[test]
fn java_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["ffi", "java"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "io.sample.core"
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::java::JavaBackend, &config, "java");
}

#[test]
fn csharp_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["ffi", "csharp"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.csharp]
namespace = "Sample.Core"
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::csharp::CsharpBackend, &config, "csharp");
}

#[test]
fn kotlin_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["ffi", "kotlin"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.kotlin]
package = "io.sample.core"
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::kotlin::KotlinBackend, &config, "kotlin");
}

#[test]
fn zig_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.zig]
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::zig::ZigBackend, &config, "zig");
}

#[test]
fn wasm_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.wasm]
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::wasm::WasmBackend, &config, "wasm");
}

/// The wasm binding crate's own `[features] default = [...]` decides whether the gates the
/// codegen just kept are actually on when cargo builds it. Keeping the item in the source while
/// leaving its passthrough row out of `default` compiles it straight back out, so the manifest
/// must expand the configured aggregate exactly as the codegen filter does.
#[test]
fn wasm_cargo_toml_defaults_the_passthrough_row_a_configured_aggregate_enables() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.wasm]
features = ["{AGGREGATE}"]
"#
        ),
        &workspace,
    );
    let files = alef::backends::wasm::WasmBackend
        .generate_bindings(&gated_api(), &config)
        .expect("wasm generation");
    let cargo_toml = files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .expect("wasm backend must emit a Cargo.toml");
    assert!(
        cargo_toml
            .content
            .contains(&format!("{GATED_FEATURE} = [\"sample-core/{GATED_FEATURE}\"]")),
        "the passthrough row must be declared:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains(&format!("default = [\"{GATED_FEATURE}\"]")),
        "`{AGGREGATE}` enables `{GATED_FEATURE}` in the core crate, so the binding crate must \
         default its own `{GATED_FEATURE}` passthrough on:\n{}",
        cargo_toml.content
    );
}

/// R resolves each gate at generation time (`extendr_module!` rejects `#[cfg]` on its entries),
/// so an unsatisfied gate removes the function outright. With `default_features = false` the
/// configured list is the whole enabled set, which is exactly when a literal aggregate erases
/// every member-gated function from the R surface.
#[test]
fn r_bindings_keep_items_gated_behind_a_configured_aggregates_member() {
    let workspace = core_crate_workspace();
    let config = config_for(
        &format!(
            r#"
[workspace]
languages = ["r"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.r]
features = ["{AGGREGATE}"]
default_features = false
"#
        ),
        &workspace,
    );
    assert_gated_function_survives(&alef::backends::extendr::ExtendrBackend, &config, "r");
}
