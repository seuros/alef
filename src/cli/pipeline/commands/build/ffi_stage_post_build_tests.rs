//! Regression coverage for `PostBuildStep::StageFfiLibrary` (alef #456): `alef build` used to
//! build the Go/Java/C# `-ffi` crate's cdylib without ever (re)staging it into the binding
//! package's native-library directory, so a stale copy from a previous build (or none at all)
//! silently shipped instead while `alef build` still reported success.

use super::*;
use crate::core::backend::{BuildConfig, BuildDependency, PostBuildStep};

fn go_config() -> crate::core::config::ResolvedCrateConfig {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["go"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "samplelib"
lib_name = "sample_lib_ffi"
"#,
    )
    .unwrap();
    alef_cfg.resolve().unwrap().remove(0)
}

fn go_build_config() -> BuildConfig {
    BuildConfig {
        tool: "go",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: vec![PostBuildStep::StageFfiLibrary],
    }
}

/// Seeds the destination with a stale, differently-sized file *before* calling
/// `run_post_build`, so this test fails the moment `StageFfiLibrary` stops actually
/// overwriting an existing destination file -- e.g. a future "skip if the destination already
/// exists" shortcut would leave the stale bytes in place and still report success, exactly the
/// silent-rot bug this step exists to close.
#[test]
fn run_post_build_stages_ffi_library_overwriting_a_stale_copy() {
    let base_dir = tempfile::tempdir().expect("failed to create temp dir");
    let config = go_config();
    let target = crate::publish::platform::host_target().expect("host target must resolve on the test machine");

    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);

    let release_dir = base_dir.path().join("target/release");
    std::fs::create_dir_all(&release_dir).expect("failed to create target/release");
    let fresh_bytes = b"fresh-build-bytes";
    std::fs::write(release_dir.join(&shared_lib), fresh_bytes).expect("failed to write fresh artifact");

    let platform = target.platform_for(Language::Go);
    let dest_dir = base_dir.path().join("packages/go/.lib").join(&platform);
    std::fs::create_dir_all(&dest_dir).expect("failed to create stale dest dir");
    std::fs::write(dest_dir.join(&shared_lib), b"STALE-PREVIOUS-BUILD").expect("failed to seed stale artifact");

    let just_built_release = StagingProfile::JustBuilt(crate::publish::package::BuildProfile::Release);
    run_post_build(
        Language::Go,
        &go_build_config(),
        &config,
        base_dir.path(),
        just_built_release,
    )
    .expect("post-build must succeed");

    let staged = std::fs::read(dest_dir.join(&shared_lib)).expect("failed to read staged artifact");
    assert_eq!(
        staged, fresh_bytes,
        "staged FFI library must be overwritten with the freshly built bytes, not left stale"
    );
}

/// Negative control: `alef generate`'s post-build pass runs every configured `PostBuildStep`,
/// including `StageFfiLibrary`, without ever invoking `cargo build` first -- so on a fresh
/// checkout with nothing under `target/` yet, staging must be a no-op that only warns, never a
/// hard failure. Without this, adding `StageFfiLibrary` to Go/Java/C#'s `post_build` would break
/// `alef generate --lang go` on every checkout that hasn't already run a build.
#[test]
fn run_post_build_skips_ffi_staging_without_error_when_artifact_is_absent() {
    let base_dir = tempfile::tempdir().expect("failed to create temp dir");
    let config = go_config();

    let outcome = run_post_build(
        Language::Go,
        &go_build_config(),
        &config,
        base_dir.path(),
        StagingProfile::PreferOnDisk,
    );
    assert!(
        outcome.is_ok(),
        "a missing build artifact must be a warning, not a post-build failure: {outcome:?}"
    );

    assert!(
        !base_dir.path().join("packages/go/.lib").exists(),
        "no destination directory should be created when nothing was staged"
    );
}

/// Negative control: a backend with no C FFI dependency (`BuildDependency::None`) must never
/// carry `PostBuildStep::StageFfiLibrary` -- staging is meaningless for it (there is no `-ffi`
/// crate to stage), and `ffi_stage::staging_dir` only recognizes Go/Java/C#, so attaching this
/// step to any other backend would turn every one of its builds into a hard failure.
#[test]
fn non_ffi_backend_build_config_has_no_ffi_staging_step() {
    let backend = crate::cli::registry::try_get_backend(Language::Python).expect("pyo3 backend must be registered");
    let build_config = backend.build_config().expect("pyo3 must have a build config");
    assert_eq!(build_config.build_dep, BuildDependency::None);
    assert!(
        !build_config
            .post_build
            .iter()
            .any(|step| matches!(step, PostBuildStep::StageFfiLibrary)),
        "a non-FFI-dependent backend must never carry the FFI staging post-build step"
    );
}
