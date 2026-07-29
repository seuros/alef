//! C# NuGet packager for the meta + per-RID native runtime split.
//!
//! The scaffold (`crate::scaffold::render_csharp_csproj` /
//! `render_csharp_runtime_csproj` / `render_csharp_runtime_json_template`) emits two
//! projects per crate:
//!
//! - `<Namespace>.csproj` — a thin meta package carrying the managed assembly plus
//!   `runtime.json` (NuGet's RID-fallback graph). No natives.
//! - `<Namespace>.Runtime.csproj` — a native-only package packed once per RID via
//!   `dotnet pack -p:PublishedRID=<rid>`, producing `<PackageId>.runtime.<rid>`.
//!
//! `package_csharp` stages the current target's native library under
//! `runtimes/{rid}/native/{libname}` (the layout both projects expect), regenerates
//! `runtime.json` from `runtime.json.template`, packs the meta project once, then packs
//! a `<PackageId>.runtime.<rid>` package for every enabled published RID that currently
//! has a staged native asset. This lets a single invocation (one target/RID staged) still
//! produce a coherent set, and lets repeated invocations that share a workspace (natives
//! accumulating across targets) produce the full RID set on the invocation that completes
//! staging.
//!
//! RID examples: `linux-x64`, `linux-arm64`, `osx-x64`, `osx-arm64`, `win-x64`,
//! `linux-musl-x64`, `linux-musl-arm64`.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::publish::platform::RustTarget;
use crate::scaffold::naming::csharp_package_id;
use crate::scaffold::{
    PUBLISHED_RUNTIME_IDENTIFIERS, render_csharp_csproj, render_csharp_runtime_csproj,
    render_csharp_runtime_json_template,
};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package the C# NuGet artifacts for the given target.
///
/// Produces the thin meta package (`{package_id}.{version}.nupkg`) plus one
/// `{package_id}.runtime.{rid}.{version}.nupkg` per enabled RID with a staged native
/// asset (at minimum, the RID for `target`).
pub fn package_csharp(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<Vec<PackageArtifact>> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);
    let rid = csharp_rid(config, target);
    let namespace = config.csharp_namespace();
    let package_id = csharp_package_id(config);

    let lib_src = crate::publish::package::find_built_artifact(workspace_root, target, &shared_lib)?;

    let pkg_dir_str = config.package_dir(Language::Csharp);
    let pkg_dir = workspace_root.join(&pkg_dir_str);
    let meta_dir = pkg_dir.join(&namespace);

    stage_native(&meta_dir, &rid, &lib_src, &shared_lib)?;

    // Regenerate the meta csproj, runtime.json, and per-RID runtime csproj from the
    // scaffold templates so the packed metadata stays in sync with alef.toml regardless
    // of what's committed on disk.
    let csproj = find_csproj(&pkg_dir, &namespace)?;
    fs::write(&csproj, render_csharp_csproj(config, version))
        .with_context(|| format!("regenerating csproj at {}", csproj.display()))?;
    tracing::debug!(path = %csproj.display(), "regenerated csproj from scaffold template");

    let runtime_json = meta_dir.join("runtime.json");
    let runtime_json_content = render_csharp_runtime_json_template(config).replace("{{VERSION}}", version);
    fs::write(&runtime_json, runtime_json_content)
        .with_context(|| format!("rendering runtime.json at {}", runtime_json.display()))?;
    tracing::debug!(path = %runtime_json.display(), "rendered runtime.json from template");

    let runtime_csproj = runtime_csproj_path(&pkg_dir, &namespace);
    fs::create_dir_all(runtime_csproj.parent().context("runtime csproj has no parent")?)
        .with_context(|| format!("creating {}", runtime_csproj.display()))?;
    fs::write(&runtime_csproj, render_csharp_runtime_csproj(config, version))
        .with_context(|| format!("regenerating runtime csproj at {}", runtime_csproj.display()))?;
    tracing::debug!(path = %runtime_csproj.display(), "regenerated runtime csproj from scaffold template");

    let abs_output_dir = output_dir.canonicalize().unwrap_or_else(|_| output_dir.to_path_buf());

    let mut artifacts = Vec::new();

    // Pack the thin meta package once: managed assembly + runtime.json RID-fallback graph.
    let meta_proj_dir = csproj.parent().context("csproj has no parent")?;
    let meta_proj_name = file_name(&csproj)?;
    let pack_meta_cmd = format!(
        "dotnet pack {proj} --configuration Release -p:Version={version} --output {out}",
        proj = meta_proj_name,
        out = abs_output_dir.display()
    );
    crate::publish::run_shell_command_in(&pack_meta_cmd, meta_proj_dir)?;
    let meta_nupkg = find_nupkg(&abs_output_dir, &package_id, version)?;
    artifacts.push(PackageArtifact {
        name: file_name(&meta_nupkg)?,
        path: meta_nupkg,
        checksum: None,
    });

    // Pack a native runtime package for every enabled published RID that currently has a
    // staged native asset (at minimum, the RID just staged above for `target`).
    let runtime_proj_dir = runtime_csproj.parent().context("runtime csproj has no parent")?;
    let runtime_proj_name = file_name(&runtime_csproj)?;
    for (rid_name, _triple) in PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .filter(|(_, t)| config.target_enabled(t))
    {
        let native_dir = meta_dir.join("runtimes").join(rid_name).join("native");
        if !has_staged_native(&native_dir) {
            continue;
        }

        let pack_runtime_cmd = format!(
            "dotnet pack {proj} --configuration Release -p:Version={version} -p:PublishedRID={rid_name} --output {out}",
            proj = runtime_proj_name,
            out = abs_output_dir.display()
        );
        crate::publish::run_shell_command_in(&pack_runtime_cmd, runtime_proj_dir)?;

        let runtime_package_id = format!("{package_id}.runtime.{rid_name}");
        let runtime_nupkg = find_nupkg(&abs_output_dir, &runtime_package_id, version)?;
        artifacts.push(PackageArtifact {
            name: file_name(&runtime_nupkg)?,
            path: runtime_nupkg,
            checksum: None,
        });
    }

    Ok(artifacts)
}

/// Stage the built native library under `{meta_dir}/runtimes/{rid}/native/{shared_lib}`,
/// the layout `<Namespace>.Runtime.csproj` packs from.
fn stage_native(meta_dir: &Path, rid: &str, lib_src: &Path, shared_lib: &str) -> Result<()> {
    let runtimes_dir = meta_dir.join("runtimes").join(rid).join("native");
    fs::create_dir_all(&runtimes_dir).with_context(|| format!("creating runtimes dir {}", runtimes_dir.display()))?;

    let staged = runtimes_dir.join(shared_lib);
    fs::copy(lib_src, &staged).with_context(|| format!("staging {} to {}", lib_src.display(), staged.display()))?;
    Ok(())
}

/// Whether a RID's `runtimes/{rid}/native/` directory has at least one staged file.
fn has_staged_native(native_dir: &Path) -> bool {
    fs::read_dir(native_dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn file_name(path: &Path) -> Result<String> {
    Ok(path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy()
        .to_string())
}

/// Return the NuGet RID for this target.
fn csharp_rid(config: &ResolvedCrateConfig, target: &RustTarget) -> String {
    if let Some(publish) = &config.publish {
        if let Some(lang_cfg) = publish.languages.get("csharp") {
            if let Some(override_rid) = &lang_cfg.csharp_rid {
                return override_rid.clone();
            }
        }
    }
    target.platform_for(Language::Csharp)
}

/// Find the thin meta `<Namespace>.csproj` under the C# package directory.
fn find_csproj(pkg_dir: &Path, namespace: &str) -> Result<PathBuf> {
    let candidate = pkg_dir.join(namespace).join(format!("{namespace}.csproj"));
    if candidate.exists() {
        return Ok(candidate);
    }
    if pkg_dir.exists() {
        for entry in fs::read_dir(pkg_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                for inner in fs::read_dir(&path)? {
                    let inner = inner?;
                    let ip = inner.path();
                    if ip.extension().is_some_and(|e| e == "csproj") {
                        return Ok(ip);
                    }
                }
            }
            if path.extension().is_some_and(|e| e == "csproj") {
                return Ok(path);
            }
        }
    }
    anyhow::bail!("No .csproj found under {}", pkg_dir.display())
}

/// The canonical path of the per-RID native runtime project the scaffold emits:
/// `{pkg_dir}/{Namespace}.Runtime/{Namespace}.Runtime.csproj`.
fn runtime_csproj_path(pkg_dir: &Path, namespace: &str) -> PathBuf {
    pkg_dir
        .join(format!("{namespace}.Runtime"))
        .join(format!("{namespace}.Runtime.csproj"))
}

fn find_nupkg(output_dir: &Path, package_id: &str, version: &str) -> Result<PathBuf> {
    let expected = output_dir.join(format!("{package_id}.{version}.nupkg"));
    if expected.exists() {
        return Ok(expected);
    }
    // dotnet pack should always produce `{package_id}.{version}.nupkg` exactly; this
    // fallback only covers benign version-string normalization (e.g. build metadata).
    // With multiple nupkgs now coexisting in `output_dir` (meta + N per-RID runtime
    // packages), an ambiguous scan could silently return the wrong artifact, so the
    // fallback only fires when exactly one `.nupkg` is present.
    let candidates: Vec<PathBuf> = fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "nupkg"))
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates.into_iter().next().expect("len checked"));
    }
    anyhow::bail!(
        "no unambiguous .nupkg for {package_id}-{version} found in {} ({} candidate(s))",
        output_dir.display(),
        candidates.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn minimal_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.csharp]
namespace = "MyLib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn rid_linux_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-x64");
    }

    #[test]
    fn rid_osx_arm64() {
        let config = minimal_config();
        let t = RustTarget::parse("aarch64-apple-darwin").unwrap();
        assert_eq!(csharp_rid(&config, &t), "osx-arm64");
    }

    #[test]
    fn rid_win_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(csharp_rid(&config, &t), "win-x64");
    }

    #[test]
    fn rid_linux_musl_x64() {
        let config = minimal_config();
        let t = RustTarget::parse("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-musl-x64");
    }

    #[test]
    fn rid_config_override() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.publish.languages.csharp]
csharp_rid = "linux-x64-custom"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let t = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(csharp_rid(&config, &t), "linux-x64-custom");
    }

    #[test]
    fn find_nupkg_expected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg = tmp.path().join("MyLib.1.0.0.nupkg");
        std::fs::write(&pkg, b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert_eq!(result, pkg);
    }

    #[test]
    fn find_nupkg_fallback_scan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pkg = tmp.path().join("SomeOtherName.1.0.0.nupkg");
        std::fs::write(&pkg, b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert!(result.extension().unwrap() == "nupkg");
    }

    #[test]
    fn find_nupkg_ambiguous_scan_errors() {
        // With multiple nupkgs coexisting (meta + per-RID runtime packages), a missing
        // exact match must error rather than silently pick one of several candidates.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.linux-x64.1.0.0.nupkg"), b"fake").unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.osx-arm64.1.0.0.nupkg"), b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn find_nupkg_exact_match_ignores_sibling_runtime_packages() {
        // The meta package_id ("MyLib") is a prefix of the runtime package ids
        // ("MyLib.runtime.<rid>"); the exact-match path must not be confused by siblings.
        let tmp = tempfile::TempDir::new().unwrap();
        let meta = tmp.path().join("MyLib.1.0.0.nupkg");
        std::fs::write(&meta, b"fake").unwrap();
        std::fs::write(tmp.path().join("MyLib.runtime.linux-x64.1.0.0.nupkg"), b"fake").unwrap();

        let result = find_nupkg(tmp.path(), "MyLib", "1.0.0").unwrap();
        assert_eq!(result, meta);
    }

    #[test]
    fn runtime_csproj_path_matches_scaffold_layout() {
        let pkg_dir = Path::new("/repo/packages/csharp");
        let path = runtime_csproj_path(pkg_dir, "MyLib");
        assert_eq!(
            path,
            Path::new("/repo/packages/csharp/MyLib.Runtime/MyLib.Runtime.csproj")
        );
    }

    #[test]
    fn has_staged_native_false_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!has_staged_native(&tmp.path().join("runtimes/linux-x64/native")));
    }

    #[test]
    fn has_staged_native_true_when_file_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let native_dir = tmp.path().join("runtimes/linux-x64/native");
        std::fs::create_dir_all(&native_dir).unwrap();
        std::fs::write(native_dir.join("libmylib.so"), b"fake").unwrap();
        assert!(has_staged_native(&native_dir));
    }

    #[test]
    fn stage_native_writes_expected_layout() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lib_src = tmp.path().join("libmylib.so");
        std::fs::write(&lib_src, b"fake-native").unwrap();

        let meta_dir = tmp.path().join("meta");
        stage_native(&meta_dir, "linux-x64", &lib_src, "libmylib.so").unwrap();

        let staged = meta_dir.join("runtimes/linux-x64/native/libmylib.so");
        assert!(staged.exists());
        assert_eq!(std::fs::read(staged).unwrap(), b"fake-native");
    }

    /// The RID loop must only pack RIDs that are both enabled in config and have a
    /// staged native — proving the loop is config-driven (not target-specific) while
    /// remaining safe against unstaged RIDs from a single-target invocation.
    #[test]
    fn published_runtime_identifiers_filtered_by_target_enabled_and_staged() {
        let config = minimal_config();
        let enabled: Vec<&str> = PUBLISHED_RUNTIME_IDENTIFIERS
            .iter()
            .filter(|(_, t)| config.target_enabled(t))
            .map(|(rid, _)| *rid)
            .collect();
        // Default config enables every published target.
        assert_eq!(enabled.len(), PUBLISHED_RUNTIME_IDENTIFIERS.len());
        assert!(enabled.contains(&"linux-x64"));
        assert!(enabled.contains(&"osx-arm64"));
        assert!(enabled.contains(&"win-x64"));
    }
}
