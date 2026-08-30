//! C FFI distribution packaging — shared lib + static lib + header + pkg-config + cmake.

use super::PackageArtifact;
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Package C FFI artifacts into a distributable tarball.
///
/// Produces: `{name}-ffi-v{version}-{platform}.tar.gz` containing:
/// - `lib/` — shared and static libraries
/// - `include/` — C header
/// - `share/pkgconfig/` — .pc file (if `pkg_config` enabled)
/// - `lib/cmake/` — CMake find module (if `cmake_config` enabled)
/// - `LICENSE`
pub fn package_c_ffi(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let crate_name = &config.name;
    let platform = target.platform_for(crate::core::config::extras::Language::Ffi);

    let pkg_name = format!("{crate_name}-ffi-v{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let lib_dir = staging.join("lib");
    let include_dir = staging.join("include");
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&include_dir)?;

    // Packaging always ships a `--release` build -- nothing here is publishable in `debug`. ~keep
    let shared_lib = target.shared_lib_name(&lib_name);
    let shared_src = super::find_built_artifact(workspace_root, target, &shared_lib, super::BuildProfile::Release)?;
    let shared_dst = lib_dir.join(&shared_lib);
    fs::copy(&shared_src, &shared_dst)?;

    super::util::fix_macos_dylib_id(target, &shared_dst, &shared_lib)?;

    let static_lib = target.static_lib_name(&lib_name);
    let static_result = super::find_built_artifact(workspace_root, target, &static_lib, super::BuildProfile::Release);
    if let Ok(static_src) = static_result {
        fs::copy(&static_src, lib_dir.join(&static_lib))?;
    }

    let ffi_crate_dir = crate::publish::ffi_stage::find_ffi_crate_dir_pub(config, workspace_root);
    copy_required_headers(config, &ffi_crate_dir, &include_dir)?;

    let pub_config = publish_lang_config(config);
    if pub_config.pkg_config.unwrap_or(true) {
        let pkgconfig_dir = staging.join("share/pkgconfig");
        fs::create_dir_all(&pkgconfig_dir)?;
        let pc_content = generate_pc_file(crate_name, version, &lib_name, &header_name);
        fs::write(pkgconfig_dir.join(format!("{crate_name}.pc")), pc_content)?;
    }

    if pub_config.cmake_config.unwrap_or(true) {
        let cmake_dir = staging.join("lib/cmake").join(crate_name);
        fs::create_dir_all(&cmake_dir)?;
        let cmake_content = generate_cmake_config(crate_name, &lib_name);
        fs::write(cmake_dir.join(format!("{crate_name}-config.cmake")), cmake_content)?;
        let version_content = generate_cmake_version(version);
        fs::write(
            cmake_dir.join(format!("{crate_name}-config-version.cmake")),
            version_content,
        )?;
    }

    // Copy LICENSE if present.
    for name in &["LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        let license = workspace_root.join(name);
        if license.exists() {
            fs::copy(&license, staging.join(name))?;
            break;
        }
    }

    let archive_name = format!("{pkg_name}.tar.gz");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    let _ = fs::remove_dir_all(&staging);

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

fn copy_required_headers(config: &ResolvedCrateConfig, ffi_crate_dir: &Path, include_dir: &Path) -> Result<()> {
    let source_dir = ffi_crate_dir.join("include");
    for header_name in required_header_names(config) {
        let source = source_dir.join(&header_name);
        fs::copy(&source, include_dir.join(&header_name))
            .with_context(|| format!("copying required C FFI header {}", source.display()))?;
    }
    Ok(())
}

fn required_header_names(config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let mut headers = BTreeSet::from([config.ffi_header_name()]);
    let Some(e2e) = config.e2e.as_ref() else {
        return headers;
    };
    for call in std::iter::once(&e2e.call).chain(e2e.calls.values()) {
        if let Some(header) = call.overrides.get("c").and_then(|override_| override_.header.as_ref()) {
            headers.insert(header.clone());
        }
    }
    headers
}

fn publish_lang_config(config: &ResolvedCrateConfig) -> crate::core::config::publish::PublishLanguageConfig {
    if let Some(publish) = &config.publish
        && let Some(cfg) = publish.languages.get("c_ffi").or_else(|| publish.languages.get("ffi"))
    {
        return cfg.clone();
    }
    crate::core::config::publish::PublishLanguageConfig::default()
}

fn generate_pc_file(name: &str, version: &str, lib_name: &str, _header: &str) -> String {
    format!(
        "prefix=${{pcfiledir}}/../..\n\
         libdir=${{prefix}}/lib\n\
         includedir=${{prefix}}/include\n\n\
         Name: {name}\n\
         Description: {name} C FFI library\n\
         Version: {version}\n\
         Libs: -L${{libdir}} -l{lib_name}\n\
         Cflags: -I${{includedir}}\n"
    )
}

fn generate_cmake_config(name: &str, lib_name: &str) -> String {
    format!(
        "# CMake find module for {name}\n\
         get_filename_component(_dir \"${{CMAKE_CURRENT_LIST_FILE}}\" PATH)\n\
         get_filename_component(_prefix \"${{_dir}}/../..\" ABSOLUTE)\n\n\
         set({name}_INCLUDE_DIR \"${{_prefix}}/include\")\n\
         set({name}_LIBRARY \"${{_prefix}}/lib/lib{lib_name}${{CMAKE_SHARED_LIBRARY_SUFFIX}}\")\n\n\
         if(EXISTS \"${{{name}_LIBRARY}}\")\n\
         \x20\x20set({name}_FOUND TRUE)\n\
         else()\n\
         \x20\x20set({name}_FOUND FALSE)\n\
         endif()\n"
    )
}

fn generate_cmake_version(version: &str) -> String {
    format!(
        "set(PACKAGE_VERSION \"{version}\")\n\n\
         if(PACKAGE_FIND_VERSION VERSION_EQUAL PACKAGE_VERSION)\n\
         \x20\x20set(PACKAGE_VERSION_EXACT TRUE)\n\
         endif()\n\n\
         if(NOT PACKAGE_FIND_VERSION VERSION_GREATER PACKAGE_VERSION)\n\
         \x20\x20set(PACKAGE_VERSION_COMPATIBLE TRUE)\n\
         else()\n\
         \x20\x20set(PACKAGE_VERSION_UNSUITABLE TRUE)\n\
         endif()\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use tempfile::TempDir;

    #[test]
    fn package_header_stage_copies_named_call_headers() {
        let config: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
header_name = "sample.h"

[crates.e2e]
fixtures = "fixtures"

[crates.e2e.call]
function = "extract"

[crates.e2e.calls.batch]
function = "extract_batch"

[crates.e2e.calls.batch.overrides.c]
header = "sample_batch.h"
"#,
        )
        .expect("config parses");
        let resolved = config.resolve().expect("config resolves").remove(0);
        let directory = TempDir::new().expect("temporary directory");
        let ffi_dir = directory.path().join("ffi");
        let output = directory.path().join("package/include");
        fs::create_dir_all(ffi_dir.join("include")).expect("create source include");
        fs::create_dir_all(&output).expect("create package include");
        fs::write(ffi_dir.join("include/sample.h"), "canonical").expect("write canonical header");
        fs::write(ffi_dir.join("include/sample_batch.h"), "batch").expect("write named header");

        copy_required_headers(&resolved, &ffi_dir, &output).expect("stage package headers");

        assert_eq!(fs::read_to_string(output.join("sample.h")).unwrap(), "canonical");
        assert_eq!(fs::read_to_string(output.join("sample_batch.h")).unwrap(), "batch");
    }
}
