//! Whether a built native library is actually on disk, asked the way a linker asks it.
//!
//! One module rather than one copy per validator because "the filenames this host can link
//! `-l<name>` against" is a single fact with a single reason to change (a new host triple, a
//! toolchain naming change), and two copies that disagree would make one language's artifact
//! probe silently wrong on exactly the platform the other one was fixed for. Grown out of
//! `zig::manifest`, which held the original and now calls this. ~keep

use std::path::Path;

/// A host platform this probe distinguishes native library naming for.
///
/// Narrower than every target triple zig can cross-compile to on purpose: the FFI directories
/// this probes hold libraries `cargo build` produced on the same host as the `zig build` that
/// links them, so the host toolchain's own naming is the only one that matters here. A target
/// directory can still carry a *different* platform's artifact -- a vendored prebuilt, a stale
/// copy left by a previous CI matrix leg -- and that is exactly the case this type exists to
/// reject rather than credit. ~keep
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HostPlatform {
    Windows,
    MacOs,
    Linux,
}

impl HostPlatform {
    /// The platform this build of alef is actually running on -- the one production call site,
    /// [`linkable_library_names`], goes through. Tests reach [`linkable_library_names_for`]
    /// directly with an explicit platform instead, so Windows/macOS/Linux naming can all be
    /// asserted from whichever single host runs the test suite. ~keep
    fn host() -> Self {
        if cfg!(target_os = "windows") {
            HostPlatform::Windows
        } else if cfg!(target_os = "macos") {
            HostPlatform::MacOs
        } else {
            HostPlatform::Linux
        }
    }
}

/// macOS's dynamic library extension, `lib`-prefixed like every non-Windows target. ~keep
const MACOS_DYNAMIC_EXTENSION: &str = "dylib";
/// Linux's (and every other non-macOS Unix's) dynamic library extension. ~keep
const LINUX_DYNAMIC_EXTENSION: &str = "so";
/// The static-archive fallback extension, `lib`-prefixed, shared by macOS and Linux. ~keep
const UNIX_STATIC_EXTENSION: &str = "a";

/// Windows carries no `lib` prefix on either its dynamic (`.dll`) or import (`.lib`) library.
/// zig's own "unable to find dynamic system library" diagnostic names exactly these three, in
/// this order, and nothing else. ~keep
const WINDOWS_DYNAMIC_EXTENSION: &str = "dll";
const WINDOWS_IMPORT_EXTENSION: &str = "lib";
const WINDOWS_STATIC_EXTENSION: &str = "a";

/// The filenames `-l<lib_name>` / `linkSystemLibrary("<lib_name>")` can actually resolve on
/// `platform`, in deterministic precedence order (most-preferred first): the dynamic library that
/// platform's toolchain actually emits, then its platform-specific fallback(s). This is a fixed
/// table, not a filesystem scan, so the order is defined by this function's own literal `vec!`
/// and never depends on directory-iteration order.
///
/// The list is deliberately platform-*exclusive*, not a shared "every non-Windows extension"
/// bucket: a `.dylib` is never a candidate on Linux and a `.so` is never a candidate on macOS. A
/// shared bucket let a stray macOS artifact in a mixed-platform/vendored target directory satisfy
/// a Linux probe (and vice versa) -- the exact false positive this table exists to prevent. ~keep
fn linkable_library_names_for(platform: HostPlatform, lib_name: &str) -> Vec<String> {
    match platform {
        HostPlatform::Windows => vec![
            format!("{lib_name}.{WINDOWS_DYNAMIC_EXTENSION}"),
            format!("{lib_name}.{WINDOWS_IMPORT_EXTENSION}"),
            format!("lib{lib_name}.{WINDOWS_STATIC_EXTENSION}"),
        ],
        HostPlatform::MacOs => vec![
            format!("lib{lib_name}.{MACOS_DYNAMIC_EXTENSION}"),
            format!("lib{lib_name}.{UNIX_STATIC_EXTENSION}"),
        ],
        HostPlatform::Linux => vec![
            format!("lib{lib_name}.{LINUX_DYNAMIC_EXTENSION}"),
            format!("lib{lib_name}.{UNIX_STATIC_EXTENSION}"),
        ],
    }
}

/// The filenames `-l<lib_name>` / `linkSystemLibrary("<lib_name>")` can actually resolve on this
/// host, which is the only set worth probing: a name the toolchain does not search for is a
/// library the build step will fail to find no matter what this reports. See
/// [`linkable_library_names_for`] for the precedence rule and why the table is platform-exclusive.
/// ~keep
pub(super) fn linkable_library_names(lib_name: &str) -> Vec<String> {
    linkable_library_names_for(HostPlatform::host(), lib_name)
}

/// Whether `directory` directly contains a linkable artifact for `lib_name` — checked by probing
/// the names this host actually searches rather than trusting `directory.exists()` alone, and
/// never inside a `deps/` subdirectory: that directory carries whatever feature set some other
/// cargo invocation unified, so a copy found only there cannot be trusted. Mirrors
/// `publish::ffi_stage`'s same refusal to accept a `deps/`-only copy. ~keep
pub(super) fn directory_has_ffi_library(directory: &Path, lib_name: &str) -> bool {
    linkable_library_names(lib_name)
        .iter()
        .any(|name| directory.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Precedence tables in `linkable_library_names_for`, keyed by platform, verbatim -- so a
    /// change to one platform's list shows up as a one-line diff against a name a reader can
    /// check by eye instead of against a recomputed value that could drift in step with a
    /// regression. ~keep
    fn expected_names(platform: HostPlatform, lib_name: &str) -> Vec<String> {
        match platform {
            HostPlatform::Windows => vec![
                format!("{lib_name}.dll"),
                format!("{lib_name}.lib"),
                format!("lib{lib_name}.a"),
            ],
            HostPlatform::MacOs => vec![format!("lib{lib_name}.dylib"), format!("lib{lib_name}.a")],
            HostPlatform::Linux => vec![format!("lib{lib_name}.so"), format!("lib{lib_name}.a")],
        }
    }

    #[test]
    fn windows_candidates_match_the_toolchain_exactly() {
        assert_eq!(
            linkable_library_names_for(HostPlatform::Windows, "sample_ffi"),
            expected_names(HostPlatform::Windows, "sample_ffi")
        );
    }

    #[test]
    fn macos_candidates_match_the_toolchain_exactly() {
        assert_eq!(
            linkable_library_names_for(HostPlatform::MacOs, "sample_ffi"),
            expected_names(HostPlatform::MacOs, "sample_ffi")
        );
    }

    #[test]
    fn linux_candidates_match_the_toolchain_exactly() {
        assert_eq!(
            linkable_library_names_for(HostPlatform::Linux, "sample_ffi"),
            expected_names(HostPlatform::Linux, "sample_ffi")
        );
    }

    /// The regression this whole table exists for: a `.dylib` is never a Linux candidate, so it
    /// can never be the *first* name `unresolvable_ffi_library` reports either. Before the fix,
    /// `linkable_library_names` returned the same `[dylib, so, a]` list for both macOS and Linux,
    /// so this assertion would have failed on Linux by reporting a `.dylib` path. ~keep
    #[test]
    fn linux_never_offers_the_macos_dynamic_extension() {
        let names = linkable_library_names_for(HostPlatform::Linux, "sample_ffi");

        assert!(!names.iter().any(|name| name.ends_with(".dylib")));
        assert_eq!(names.first(), Some(&"libsample_ffi.so".to_owned()));
    }

    /// The mirror regression: a `.so` is never a macOS candidate, and `.dylib` stays first.
    #[test]
    fn macos_never_offers_the_linux_dynamic_extension() {
        let names = linkable_library_names_for(HostPlatform::MacOs, "sample_ffi");

        assert!(!names.iter().any(|name| name.ends_with(".so")));
        assert_eq!(names.first(), Some(&"libsample_ffi.dylib".to_owned()));
    }

    /// The ambiguous-artifact control: a target directory holding *every* platform's dynamic
    /// library at once (a plausible shared/vendored `target/release` populated by a multi-OS CI
    /// matrix). Each platform must still resolve to only its own artifact, deterministically --
    /// not to whichever file a directory scan happened to visit first. This is the scenario the
    /// reported defect actually reproduces under: with more than one native artifact present,
    /// Linux must not credit the macOS one. ~keep
    #[test]
    fn an_ambiguous_directory_resolves_per_platform_and_not_by_iteration_order() {
        let directory = tempfile::tempdir().expect("scratch directory");
        for extension in ["dylib", "so", "dll", "lib", "a"] {
            std::fs::write(directory.path().join(format!("libsample_ffi.{extension}")), "fake").unwrap();
        }
        // Windows' unprefixed names are also present, matching a real Windows build output.
        std::fs::write(directory.path().join("sample_ffi.dll"), "fake").unwrap();
        std::fs::write(directory.path().join("sample_ffi.lib"), "fake").unwrap();

        for platform in [HostPlatform::Windows, HostPlatform::MacOs, HostPlatform::Linux] {
            let names = linkable_library_names_for(platform, "sample_ffi");
            assert_eq!(
                names,
                expected_names(platform, "sample_ffi"),
                "{platform:?} must resolve its own fixed candidate list regardless of what else is on disk"
            );
            assert!(
                names.iter().any(|name| directory.path().join(name).is_file()),
                "{platform:?} must still find its own artifact in the mixed directory"
            );
        }
    }

    #[test]
    fn directory_has_ffi_library_never_credits_a_deps_only_copy() {
        let directory = tempfile::tempdir().expect("scratch directory");
        std::fs::create_dir_all(directory.path().join("deps")).unwrap();
        for name in linkable_library_names_for(HostPlatform::host(), "sample_ffi") {
            std::fs::write(directory.path().join("deps").join(name), "fake").unwrap();
        }

        assert!(!directory_has_ffi_library(directory.path(), "sample_ffi"));
    }
}
