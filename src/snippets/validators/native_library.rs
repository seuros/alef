//! Whether a built native library is actually on disk, asked the way a linker asks it.
//!
//! One module rather than one copy per validator because "the filenames this host can link
//! `-l<name>` against" is a single fact with a single reason to change (a new host triple, a
//! toolchain naming change), and two copies that disagree would make one language's artifact
//! probe silently wrong on exactly the platform the other one was fixed for. Grown out of
//! `zig::manifest`, which held the original and now calls this. ~keep

use std::path::Path;

/// The filenames `-l<lib_name>` / `linkSystemLibrary("<lib_name>")` can actually resolve on this
/// host, which is the only set worth probing: a name the toolchain does not search for is a
/// library the build step will fail to find no matter what this reports.
///
/// Windows is not `lib`-prefixed for its dynamic and import libraries. Zig names its own search
/// there -- verbatim, from the "unable to find dynamic system library" diagnostic -- as
/// `{name}.dll`, `{name}.lib`, `lib{name}.a`, which is also what cargo emits (`{name}.dll` plus
/// `{name}.dll.lib` for a cdylib, `{name}.lib` for a staticlib). `lib{name}.dll` is a file no
/// Windows toolchain produces and zig never looks for, so prepending `lib` unconditionally made
/// this probe unable to find a real Windows FFI library at all. ~keep
pub(super) fn linkable_library_names(lib_name: &str) -> [String; 3] {
    if cfg!(windows) {
        [
            format!("{lib_name}.dll"),
            format!("{lib_name}.lib"),
            format!("lib{lib_name}.a"),
        ]
    } else {
        [
            format!("lib{lib_name}.dylib"),
            format!("lib{lib_name}.so"),
            format!("lib{lib_name}.a"),
        ]
    }
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
