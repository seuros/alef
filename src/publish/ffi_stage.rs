//! FFI artifact staging — copies built shared libraries into language-specific
//! directories for Go, Java, and C# packages.
//!
//! After `cargo build --release -p {name}-ffi --target {triple}`, the shared
//! library lives in `target/{triple}/release/`. This module copies it to:
//! - Go: `packages/go/.lib/{platform}/` (e.g., `macos-arm64/`, `linux-x86_64/`)
//! - Java: `packages/java/src/main/resources/natives/{rid}/`
//! - C#: `packages/csharp/{Project}/runtimes/{rid}/native/`
//!
//! For Go, the staged `.lib/{platform}/` directory only serves in-tree/dev flows —
//! e.g. `go generate` (which shells out to `cmd/setup -lib-dir .lib`) and local builds
//! against this checkout. The published Go module does not ship `.lib/` (it stays
//! gitignored); consumers instead run `cmd/setup`, which downloads the native library
//! from the GitHub release into a per-user cache at consume-time.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::publish::package::BuildProfile;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// Stage the FFI shared library for a specific language, target, and build profile.
///
/// `profile` must name the profile the caller actually wants staged -- there is no "guess the
/// right one" mode here. A caller that just ran `cargo build`/`cargo build --release` itself
/// knows exactly which profile that produced and must pass it; a caller with no such build of
/// its own to point to should use [`stage_ffi_preferring_release`] instead of picking a profile
/// arbitrarily.
pub fn stage_ffi(
    config: &ResolvedCrateConfig,
    lang: Language,
    target: &RustTarget,
    workspace_root: &Path,
    profile: BuildProfile,
) -> Result<PathBuf> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);

    let lib_path = find_built_library(workspace_root, target, &shared_lib, profile)?;

    let dest_dir = staging_dir(config, lang, target, workspace_root)?;
    fs::create_dir_all(&dest_dir).with_context(|| format!("creating {}", dest_dir.display()))?;

    let dest_path = dest_dir.join(&shared_lib);
    fs::copy(&lib_path, &dest_path)
        .with_context(|| format!("copying {} to {}", lib_path.display(), dest_path.display()))?;

    tracing::info!(
        lang = %lang,
        lib = %shared_lib,
        dest = %dest_dir.display(),
        %profile,
        "staged FFI library"
    );

    Ok(dest_path)
}

/// Stage the FFI shared library using whichever build profile is already on disk, `release`
/// preferred, falling back to `debug`.
///
/// For callers that do not themselves know which profile (if either) was most recently built --
/// `alef generate`'s post-build pass and `alef test`'s e2e FFI staging never invoke `cargo build`
/// at all, so neither can name a profile the way [`stage_ffi`]'s contract requires. Trying
/// `release` first matches what every other caller of `stage_ffi` (the `build` pipeline, `alef
/// publish`) treats as canonical; `debug` is a legitimate fallback here specifically because nothing
/// in this path just ran a build of its own to trust over what is already there. Still never
/// touches `deps/` -- that fallback is what this whole module exists to not repeat.
pub fn stage_ffi_preferring_release(
    config: &ResolvedCrateConfig,
    lang: Language,
    target: &RustTarget,
    workspace_root: &Path,
) -> Result<PathBuf> {
    match stage_ffi(config, lang, target, workspace_root, BuildProfile::Release) {
        Ok(dest) => Ok(dest),
        Err(release_error) => stage_ffi(config, lang, target, workspace_root, BuildProfile::Debug)
            .map_err(|debug_error| release_error.context(format!("debug fallback also failed: {debug_error:#}"))),
    }
}

/// Optionally stage the C header alongside the shared library.
pub fn stage_header(
    config: &ResolvedCrateConfig,
    lang: Language,
    target: &RustTarget,
    workspace_root: &Path,
) -> Result<Option<PathBuf>> {
    let header_name = config.ffi_header_name();
    let ffi_crate_dir = find_ffi_crate_dir(config, workspace_root);

    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if !header_src.exists() {
        return Ok(None);
    }

    let dest_dir = staging_dir(config, lang, target, workspace_root)?;
    let include_dir = dest_dir.join("include");
    fs::create_dir_all(&include_dir)?;

    let dest_path = include_dir.join(&header_name);
    fs::copy(&header_src, &dest_path)?;

    Ok(Some(dest_path))
}

/// Find the built shared library in the target directory for a specific build profile.
fn find_built_library(
    workspace_root: &Path,
    target: &RustTarget,
    shared_lib: &str,
    profile: BuildProfile,
) -> Result<PathBuf> {
    crate::publish::package::find_built_artifact(workspace_root, target, shared_lib, profile)
}

/// Whether a built FFI shared library for `target` and `profile` exists on disk, without staging
/// it.
///
/// Callers that run staging as a post-build step must distinguish "nothing was built this run"
/// (a legitimate skip -- e.g. `alef generate`'s post-build pass reruns every backend's
/// [`crate::core::backend::PostBuildStep`]s without ever invoking `cargo build`) from a genuine
/// copy failure once staging is attempted against an artifact known to exist. ~keep
pub fn ffi_artifact_built(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    profile: BuildProfile,
) -> bool {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);
    find_built_library(workspace_root, target, &shared_lib, profile).is_ok()
}

/// [`ffi_artifact_built`] but for callers that cannot name a single profile -- true when either
/// `release` or `debug` has a trusted, uplifted artifact on disk. Mirrors
/// [`stage_ffi_preferring_release`]'s fallback order.
pub fn ffi_artifact_built_preferring_release(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
) -> bool {
    ffi_artifact_built(config, target, workspace_root, BuildProfile::Release)
        || ffi_artifact_built(config, target, workspace_root, BuildProfile::Debug)
}

/// Determine the staging directory for a language + target combination.
pub(crate) fn staging_dir(
    config: &ResolvedCrateConfig,
    lang: Language,
    target: &RustTarget,
    workspace_root: &Path,
) -> Result<PathBuf> {
    let pkg_dir = config.package_dir(lang);
    let platform = target.platform_for(lang);

    let rel = match lang {
        Language::Go => PathBuf::from(&pkg_dir).join(".lib").join(&platform),
        Language::Java => PathBuf::from(&pkg_dir)
            .join("src/main/resources/natives")
            .join(&platform),
        Language::Csharp => {
            let namespace = config.csharp_namespace();
            PathBuf::from(&pkg_dir)
                .join(&namespace)
                .join("runtimes")
                .join(&platform)
                .join("native")
        }
        other => bail!("FFI staging not supported for {other}"),
    };

    Ok(workspace_root.join(rel))
}

/// Find the FFI crate directory (for locating the header file). Public alias for use by packagers.
pub fn find_ffi_crate_dir_pub(config: &ResolvedCrateConfig, workspace_root: &Path) -> PathBuf {
    find_ffi_crate_dir(config, workspace_root)
}

/// Find the FFI crate directory (for locating the header file).
fn find_ffi_crate_dir(config: &ResolvedCrateConfig, workspace_root: &Path) -> PathBuf {
    if let Some(ffi_output) = config.explicit_output.ffi.as_ref() {
        let p = Path::new(ffi_output);
        for ancestor in p.ancestors() {
            if ancestor.join("Cargo.toml").exists() || ancestor.join("include").exists() {
                return workspace_root.join(ancestor);
            }
        }
        if let Some(parent) = p.parent() {
            return workspace_root.join(parent);
        }
    }

    let crate_name = &config.name;
    workspace_root.join(format!("crates/{crate_name}-ffi"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use std::fs;
    use tempfile::TempDir;

    fn minimal_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["go", "java", "csharp"]

[[crates]]
name = "my-lib"
sources = ["crates/my-lib/src/lib.rs"]

[crates.ffi]
prefix = "mylib"
lib_name = "my_lib_ffi"
header_name = "my_lib.h"

[crates.csharp]
namespace = "MyLib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn setup_built_ffi(root: &Path, target_triple: &str) {
        setup_built_ffi_for_profile(root, target_triple, BuildProfile::Release);
    }

    fn setup_built_ffi_for_profile(root: &Path, target_triple: &str, profile: BuildProfile) {
        let target = RustTarget::parse(target_triple).unwrap();
        let lib_name = target.shared_lib_name("my_lib_ffi");
        let profile_dir = root.join("target").join(target_triple).join(profile.dir_name());
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join(lib_name), "fake-lib").unwrap();
    }

    fn setup_header(root: &Path) {
        let include_dir = root.join("crates/my-lib-ffi/include");
        fs::create_dir_all(&include_dir).unwrap();
        fs::write(include_dir.join("my_lib.h"), "#pragma once").unwrap();
    }

    #[test]
    fn stage_ffi_go() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release).unwrap();
        assert!(result.exists());
        assert!(
            result
                .to_string_lossy()
                .replace('\\', "/")
                .contains("packages/go/.lib/linux-x86_64")
        );
    }

    #[test]
    fn stage_ffi_java() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/java")).unwrap();

        let result = stage_ffi(&config, Language::Java, &target, root, BuildProfile::Release).unwrap();
        assert!(result.exists());
        assert!(
            result
                .to_string_lossy()
                .replace('\\', "/")
                .contains("natives/linux-x86_64")
        );
    }

    #[test]
    fn stage_ffi_csharp() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("aarch64-apple-darwin").unwrap();

        setup_built_ffi(root, "aarch64-apple-darwin");
        fs::create_dir_all(root.join("packages/csharp")).unwrap();

        let result = stage_ffi(&config, Language::Csharp, &target, root, BuildProfile::Release).unwrap();
        assert!(result.exists());
        assert!(
            result
                .to_string_lossy()
                .replace('\\', "/")
                .contains("runtimes/osx-arm64/native")
        );
    }

    #[test]
    fn stage_ffi_not_found() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    /// The regression this rewrite closes: a `debug`-profile artifact must not satisfy a
    /// `Release` request, even though `stage_ffi` used to search release paths only and would
    /// previously have reported "not found" the same as if nothing were built at all -- now it
    /// must say so specifically rather than fall through to an unrelated `deps/` copy.
    #[test]
    fn stage_ffi_release_does_not_use_a_debug_only_build() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release);
        assert!(
            result.is_err(),
            "a debug-only build must not satisfy a release staging request"
        );
    }

    /// Positive case for the `Debug` profile itself, proving `stage_ffi` is not silently
    /// hardcoded to `release` under the hood. Negative control is
    /// `stage_ffi_release_does_not_use_a_debug_only_build` above (the same fixture, requested
    /// under the other profile, must fail).
    #[test]
    fn stage_ffi_debug_profile_stages_a_debug_only_build() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root, BuildProfile::Debug).unwrap();
        assert!(result.exists());
    }

    #[test]
    fn ffi_artifact_built_true_when_release_artifact_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");

        assert!(ffi_artifact_built(&config, &target, root, BuildProfile::Release));
    }

    #[test]
    fn ffi_artifact_built_false_when_nothing_built() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        assert!(!ffi_artifact_built(&config, &target, root, BuildProfile::Release));
    }

    /// Negative control matching `stage_ffi_release_does_not_use_a_debug_only_build`: the
    /// existence check must agree with staging about which profile is present.
    #[test]
    fn ffi_artifact_built_false_for_release_when_only_debug_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);

        assert!(!ffi_artifact_built(&config, &target, root, BuildProfile::Release));
    }

    /// `_preferring_release` variants exist for callers with no build of their own to point to
    /// (`alef generate`'s post-build pass, `alef test`'s e2e staging). When both profiles exist,
    /// release must win -- it is what every other, profile-aware caller treats as canonical.
    #[test]
    fn stage_ffi_preferring_release_prefers_release_when_both_exist() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Release);
        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi_preferring_release(&config, Language::Go, &target, root).unwrap();
        let staged = fs::read(&result).unwrap();
        let release_dir = root.join("target/x86_64-unknown-linux-gnu/release");
        let expected = fs::read(release_dir.join(target.shared_lib_name("my_lib_ffi"))).unwrap();
        assert_eq!(
            staged, expected,
            "release must be preferred when both profiles are present"
        );
    }

    /// Negative control for the previous test and the direct regression proof for this task: when
    /// only `debug` exists (the consumer's own reported scenario -- `cargo build -p <crate>` with
    /// no `--release`), staging must still succeed by falling back to it, not fail or silently
    /// substitute an unrelated `deps/` copy.
    #[test]
    fn stage_ffi_preferring_release_falls_back_to_debug_when_release_absent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi_preferring_release(&config, Language::Go, &target, root).unwrap();
        assert!(result.exists());
    }

    /// Negative control: when neither profile exists, `_preferring_release` must fail rather than
    /// fall back to `deps/` -- proving the fallback logic still respects the "never trust deps/"
    /// contract instead of reintroducing it one layer up.
    #[test]
    fn stage_ffi_preferring_release_fails_when_only_deps_copy_exists() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let lib_name = target.shared_lib_name("my_lib_ffi");

        let deps_dir = root.join("target/x86_64-unknown-linux-gnu/release/deps");
        fs::create_dir_all(&deps_dir).unwrap();
        fs::write(deps_dir.join(&lib_name), "deps-only").unwrap();
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi_preferring_release(&config, Language::Go, &target, root);
        assert!(
            result.is_err(),
            "a deps/-only copy under either profile must not satisfy staging"
        );
    }

    #[test]
    fn ffi_artifact_built_preferring_release_true_when_only_debug_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi_for_profile(root, "x86_64-unknown-linux-gnu", BuildProfile::Debug);

        assert!(ffi_artifact_built_preferring_release(&config, &target, root));
    }

    /// Negative control for the previous test: with nothing built under either profile, the
    /// preferring-release check must still report false rather than defaulting to true.
    #[test]
    fn ffi_artifact_built_preferring_release_false_when_nothing_built() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        assert!(!ffi_artifact_built_preferring_release(&config, &target, root));
    }

    #[test]
    fn stage_header_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        setup_header(root);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release).unwrap();

        let result = stage_header(&config, Language::Go, &target, root).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().exists());
    }

    #[test]
    fn stage_header_missing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/go")).unwrap();
        stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release).unwrap();

        let result = stage_header(&config, Language::Go, &target, root).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn stage_ffi_native_build_fallback() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let lib_name = target.shared_lib_name("my_lib_ffi");

        let release_dir = root.join("target/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join(&lib_name), "fake-lib").unwrap();
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root, BuildProfile::Release).unwrap();
        assert!(result.exists());
        assert!(
            result
                .to_string_lossy()
                .replace('\\', "/")
                .contains(".lib/linux-x86_64")
        );
    }
}
