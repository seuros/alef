//! Shared Dart native library staging logic for both build-time and publish-time.
//!
//! Both `cargo build` (post-build step) and `cargo publish` (packaging step) need
//! to stage prebuilt native libraries into the Dart package's `lib/src/native/<rid>/`
//! directory so that flutter_rust_bridge can find them at runtime.
//!
//! Lookup goes through [`crate::publish::package::find_built_artifact`] -- the same resolver
//! `ffi_stage` and the other publish packagers use -- rather than re-deriving `target/{triple}/
//! {profile}/` paths locally. Before this, `find_native_libraries` hardcoded `release` with no
//! profile parameter at all, so a debug-only `alef build` either silently staged nothing or, if a
//! stale `release` artifact from an earlier run was still on disk, silently staged *that* --
//! exactly the wrong-profile staging bug already fixed once for `ffi_stage::stage_ffi`. ~keep

use crate::publish::package::{BuildProfile, PREFERRING_RELEASE_ORDER, find_built_artifact};
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLibraryStageStatus {
    Staged,
    Missing,
}

/// Recursively copy a directory and all its contents.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).context(format!("creating directory: {}", dst.display()))?;
    for entry in fs::read_dir(src).context(format!("reading directory: {}", src.display()))? {
        let entry = entry.context(format!("reading entry in {}", src.display()))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);
        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path)?;
        } else {
            fs::copy(&path, &dst_path)
                .with_context(|| format!("copying file {} to {}", path.display(), dst_path.display()))?;
        }
    }
    Ok(())
}

/// Platform-specific native library filename patterns.
/// Maps from runtime identifier (RID) to expected library filenames.
#[derive(Debug, Clone)]
struct NativeLibPattern {
    rid: &'static str,
    rust_target: &'static str,
    formats: &'static [NativeLibFormat],
}

#[derive(Debug, Clone, Copy)]
enum NativeLibFormat {
    MacosDylib,
    MacosFramework,
    UnixSharedObject,
    WindowsDll,
}

impl NativeLibFormat {
    fn filename(self, stem: &str) -> String {
        match self {
            Self::MacosDylib => format!("lib{stem}.dylib"),
            Self::MacosFramework => format!("{stem}.framework/{stem}"),
            Self::UnixSharedObject => format!("lib{stem}.so"),
            Self::WindowsDll => format!("{stem}.dll"),
        }
    }
}

const MACOS_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::MacosDylib, NativeLibFormat::MacosFramework];
const LINUX_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::UnixSharedObject];
const WINDOWS_NATIVE_LIB_FORMATS: &[NativeLibFormat] = &[NativeLibFormat::WindowsDll];

const NATIVE_LIB_PATTERNS: &[NativeLibPattern] = &[
    NativeLibPattern {
        rid: "macos-x64",
        rust_target: "x86_64-apple-darwin",
        formats: MACOS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "macos-arm64",
        rust_target: "aarch64-apple-darwin",
        formats: MACOS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "linux-x64",
        rust_target: "x86_64-unknown-linux-gnu",
        formats: LINUX_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "linux-arm64",
        rust_target: "aarch64-unknown-linux-gnu",
        formats: LINUX_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "windows-x64",
        rust_target: "x86_64-pc-windows-msvc",
        formats: WINDOWS_NATIVE_LIB_FORMATS,
    },
    NativeLibPattern {
        rid: "windows-arm64",
        rust_target: "aarch64-pc-windows-msvc",
        formats: WINDOWS_NATIVE_LIB_FORMATS,
    },
];

/// Find prebuilt native libraries in the cargo target directory, searching `profiles` in order
/// and taking the first profile under which a given filename is found.
///
/// Delegates to [`find_built_artifact`] for the actual `target/{triple}/{profile}/` /
/// `target/{profile}/` lookup, so this stays in lockstep with every other staging/packaging path
/// instead of carrying its own copy of that search order. A filename simply absent under every
/// profile is not an error here -- multiple filenames may be checked (e.g. a `.dylib` and a
/// `.framework` for macOS) and it is normal for only one format to have actually been built.
fn find_native_libraries(
    workspace_root: &Path,
    rust_target: &str,
    filenames: &[String],
    profiles: &[BuildProfile],
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let target = RustTarget::parse(rust_target)
        .with_context(|| format!("parsing built-in Dart native-library target triple '{rust_target}'"))?;

    let mut found = Vec::new();
    for filename in filenames {
        for &profile in profiles {
            if let Ok(lib_path) = find_built_artifact(workspace_root, &target, filename, profile) {
                found.push((lib_path, PathBuf::from(filename)));
                break;
            }
        }
    }

    Ok(found)
}

/// Stage prebuilt native libraries into a Dart package's lib/src/native/ directory, searching
/// only the caller-named `profile` -- for callers that just ran `cargo build`/`cargo build
/// --release` themselves and know exactly which profile that produced. A stale artifact from the
/// *other* profile on disk from an earlier, unrelated run must never silently satisfy this call.
/// See [`stage_dart_native_libraries_preferring_release`] for callers with no such build of their
/// own to point to.
///
/// Creates `{package_root}/lib/src/native/{rid}/` and copies native libraries there.
/// If no native libraries are found, this is a no-op (development builds may lack them).
///
/// Arguments:
/// - `workspace_root`: Root of the workspace (where `target/` and Cargo.toml are)
/// - `package_root`: Root of the Dart package (where `pubspec.yaml` is; often `{workspace_root}/packages/dart`)
/// - `stem`: The library name stem (e.g., `sample_lib_dart` for a `libsample_lib_dart.dylib`)
pub fn stage_dart_native_libraries(
    workspace_root: &Path,
    package_root: &Path,
    stem: &str,
    profile: BuildProfile,
) -> Result<NativeLibraryStageStatus> {
    stage_dart_native_libraries_for_profiles(workspace_root, package_root, stem, std::slice::from_ref(&profile))
}

/// As [`stage_dart_native_libraries`], but searches [`PREFERRING_RELEASE_ORDER`] (`release` first,
/// `debug` fallback) instead of a single caller-named profile -- for callers that never invoke
/// `cargo build` themselves and so cannot name which profile (if either) was actually built:
/// `alef generate`'s post-build pass, `alef test`'s e2e staging, and `alef publish`'s Dart
/// packager. Mirrors [`crate::publish::ffi_stage::stage_ffi_preferring_release`]'s fallback order.
pub fn stage_dart_native_libraries_preferring_release(
    workspace_root: &Path,
    package_root: &Path,
    stem: &str,
) -> Result<NativeLibraryStageStatus> {
    stage_dart_native_libraries_for_profiles(workspace_root, package_root, stem, &PREFERRING_RELEASE_ORDER)
}

fn stage_dart_native_libraries_for_profiles(
    workspace_root: &Path,
    package_root: &Path,
    stem: &str,
    profiles: &[BuildProfile],
) -> Result<NativeLibraryStageStatus> {
    let native_base = package_root.join("lib/src/native");
    let mut staged_any = false;

    for pattern in NATIVE_LIB_PATTERNS {
        let filenames = pattern
            .formats
            .iter()
            .map(|format| format.filename(stem))
            .collect::<Vec<_>>();
        let libs = find_native_libraries(workspace_root, pattern.rust_target, &filenames, profiles)?;
        if libs.is_empty() {
            continue;
        }

        let rid_dir = native_base.join(pattern.rid);
        fs::create_dir_all(&rid_dir).context(format!("creating native library directory: {}", rid_dir.display()))?;

        for (lib_path, relative_path) in libs {
            let dest = rid_dir.join(relative_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating native library parent directory: {}", parent.display()))?;
            }
            if lib_path.is_dir() {
                copy_dir_recursive(&lib_path, &dest).with_context(|| {
                    format!(
                        "copying native library directory {} to {}",
                        lib_path.display(),
                        dest.display()
                    )
                })?;
            } else {
                fs::copy(&lib_path, &dest)
                    .with_context(|| format!("copying native library {} to {}", lib_path.display(), dest.display()))?;
            }
            staged_any = true;
        }
    }

    Ok(if staged_any {
        NativeLibraryStageStatus::Staged
    } else {
        NativeLibraryStageStatus::Missing
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_native_libraries_uses_package_stem() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp.path().join("target/aarch64-apple-darwin/release");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("libmy_lib_dart.dylib"), "native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status =
            stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Release).unwrap();

        assert_eq!(status, NativeLibraryStageStatus::Staged);
        assert!(
            package_root
                .join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")
                .exists()
        );
    }

    #[test]
    fn stage_native_libraries_preserves_framework_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp
            .path()
            .join("target/aarch64-apple-darwin/release/my_lib_dart.framework");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("my_lib_dart"), "native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status =
            stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Release).unwrap();

        assert_eq!(status, NativeLibraryStageStatus::Staged);
        assert!(
            package_root
                .join("lib/src/native/macos-arm64/my_lib_dart.framework/my_lib_dart")
                .exists()
        );
    }

    /// The regression this rewrite closes: `find_native_libraries` used to hardcode `release`
    /// with no profile parameter, so a debug-only build's libraries could never be found by an
    /// explicit-profile caller. Proves the `profile` parameter actually selects
    /// `target/{triple}/{profile}/`, not just a hardcoded `release`. Negative control:
    /// `debug_only_build_is_missing_for_a_release_request` below proves the same fixture does NOT
    /// satisfy a `Release` request.
    #[test]
    fn stage_native_libraries_debug_profile_finds_a_debug_only_build() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp.path().join("target/aarch64-apple-darwin/debug");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("libmy_lib_dart.dylib"), "debug-native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status =
            stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Debug).unwrap();

        assert_eq!(status, NativeLibraryStageStatus::Staged);
        let staged = fs::read(package_root.join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")).unwrap();
        assert_eq!(
            staged, b"debug-native",
            "must stage the debug-profile bytes, not substitute anything else"
        );
    }

    /// Negative control for the previous test, and the direct proof that an explicit-profile
    /// request never silently falls back to the other profile's artifact: a debug-only build must
    /// report `Missing` (not error, not silently stage the debug copy) for a `Release` request,
    /// exactly the "nothing was built this run" contract `PostBuildStep::StageFfiLibrary`'s sibling
    /// already relies on.
    #[test]
    fn debug_only_build_is_missing_for_a_release_request() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target_dir = tmp.path().join("target/aarch64-apple-darwin/debug");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("libmy_lib_dart.dylib"), "debug-native").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status =
            stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Release).unwrap();

        assert_eq!(
            status,
            NativeLibraryStageStatus::Missing,
            "a debug-only build must not satisfy a release staging request"
        );
        assert!(
            !package_root.join("lib/src/native").exists(),
            "no destination directory should be created when nothing matched the requested profile"
        );
    }

    /// The critical never-silently-stale property, stated directly: when a *stale* release
    /// artifact sits on disk alongside a *fresh* debug artifact (the shape of the bug this whole
    /// rewrite closes -- an old `alef build --release` run left bytes behind, then a plain `alef
    /// build` produced a new debug artifact), an explicit `Debug` request must stage the fresh
    /// debug bytes, never the stale release ones.
    #[test]
    fn explicit_debug_request_never_prefers_a_stale_release_artifact() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let release_dir = tmp.path().join("target/aarch64-apple-darwin/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(
            release_dir.join("libmy_lib_dart.dylib"),
            "STALE-RELEASE-FROM-EARLIER-RUN",
        )
        .unwrap();

        let debug_dir = tmp.path().join("target/aarch64-apple-darwin/debug");
        fs::create_dir_all(&debug_dir).unwrap();
        fs::write(debug_dir.join("libmy_lib_dart.dylib"), "fresh-debug-bytes").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Debug).unwrap();

        let staged = fs::read(package_root.join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")).unwrap();
        assert_eq!(
            staged, b"fresh-debug-bytes",
            "expected the fresh debug bytes, got the stale release copy instead"
        );
    }

    /// `_preferring_release` (used by `alef generate`'s post-build pass, `alef test`'s e2e
    /// staging, and `alef publish`'s Dart packager -- none of which invoke `cargo build`
    /// themselves) must prefer `release` when both profiles exist, matching
    /// `ffi_stage::stage_ffi_preferring_release`'s order.
    #[test]
    fn preferring_release_prefers_release_when_both_profiles_exist() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let release_dir = tmp.path().join("target/aarch64-apple-darwin/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("libmy_lib_dart.dylib"), "release-bytes").unwrap();

        let debug_dir = tmp.path().join("target/aarch64-apple-darwin/debug");
        fs::create_dir_all(&debug_dir).unwrap();
        fs::write(debug_dir.join("libmy_lib_dart.dylib"), "debug-bytes").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        stage_dart_native_libraries_preferring_release(tmp.path(), &package_root, "my_lib_dart").unwrap();

        let staged = fs::read(package_root.join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")).unwrap();
        assert_eq!(
            staged, b"release-bytes",
            "release must win when both profiles are present"
        );
    }

    /// Negative control / fallback proof for the previous test: with only `debug` on disk,
    /// `_preferring_release` must still succeed by falling back to it rather than reporting
    /// `Missing`.
    #[test]
    fn preferring_release_falls_back_to_debug_when_release_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let debug_dir = tmp.path().join("target/aarch64-apple-darwin/debug");
        fs::create_dir_all(&debug_dir).unwrap();
        fs::write(debug_dir.join("libmy_lib_dart.dylib"), "debug-bytes").unwrap();

        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status = stage_dart_native_libraries_preferring_release(tmp.path(), &package_root, "my_lib_dart").unwrap();

        assert_eq!(status, NativeLibraryStageStatus::Staged);
        let staged = fs::read(package_root.join("lib/src/native/macos-arm64/libmy_lib_dart.dylib")).unwrap();
        assert_eq!(staged, b"debug-bytes");
    }

    #[test]
    fn missing_native_libraries_are_a_development_no_op() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let package_root = tmp.path().join("packages/dart");
        fs::create_dir_all(&package_root).unwrap();

        let status =
            stage_dart_native_libraries(tmp.path(), &package_root, "my_lib_dart", BuildProfile::Release).unwrap();

        assert_eq!(status, NativeLibraryStageStatus::Missing);
        assert!(!package_root.join("lib/src/native").exists());
    }
}
