//! Parsing helpers for the `.cwd_relative`-bound options a scaffolded `build.zig` declares --
//! include directories, and (below) the FFI library search directory. Both read back only the
//! shape `scaffold::languages::zig::scaffold_zig` emits; any other expression is skipped rather
//! than guessed at, consistent with the rest of this validator's silence-vs-loudness choices. ~keep

use crate::snippets::error::Result;
use std::path::{Path, PathBuf};

/// Include directories the build manifest declares for its module, in declaration order. ~keep
///
/// A `build.zig` is a program rather than a manifest, so this reads back only the shape Alef's own
/// scaffold (`scaffold::languages::zig::scaffold_zig`) emits:
/// `addIncludePath(.{ .cwd_relative = <expr> })`, where `<expr>` is either a string literal or an
/// identifier [`binding_default`] can trace back to one. Any other expression is skipped rather
/// than guessed at — a wrong `-I` is worse than none.
///
/// Without this the reconstructed `build-exe` command carries no `-I` at all unless the consumer
/// also repeats the path under `include_paths`, so every snippet reaching a `@cInclude` in the
/// binding fails with `C import failed ... 'header.h' not found` while `zig build` succeeds.
/// Paths are returned verbatim, relative to the manifest's own directory — the build root the
/// scaffolded manifest rebases them onto, which the caller supplies.
pub(crate) fn zig_manifest_include_paths(manifest: &Path) -> Result<Vec<String>> {
    const DECLARATION: &str = "addIncludePath(.{ .cwd_relative = ";

    let source = std::fs::read_to_string(manifest)?;
    let mut paths: Vec<String> = Vec::new();
    for occurrence in source.split(DECLARATION).skip(1) {
        let Some(end) = occurrence.find(" })") else {
            continue;
        };
        let expression = occurrence[..end].trim();
        let Some(path) = string_literal(expression)
            .map(str::to_owned)
            .or_else(|| binding_default(&source, expression))
        else {
            continue;
        };
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn string_literal(expression: &str) -> Option<&str> {
    expression.strip_prefix('"')?.strip_suffix('"')
}

/// How many `const` hops [`resolve_binding_statement`] follows before giving up. The scaffold
/// emits exactly one (`ffi_include` → `ffi_include_option`, `ffi_path` → `ffi_path_option`); the
/// rest is headroom, and the bound is what keeps a hand-written manifest with a cyclic binding
/// from spinning here. ~keep
const MAX_BINDING_INDIRECTIONS: usize = 4;

/// Follow a chain of `const <name> = ...;` bindings down to its terminal statement, rebasing
/// through `const <name> = b.pathResolve(&.{ <build root>, <inner> });` when present.
///
/// Shared by [`binding_default`] (which only needs the terminal `orelse` literal) and
/// `zig_manifest_library_path_option` (which also needs the option *name* the same statement
/// declares) — factored out so both read exactly the same chain instead of risking the two
/// silently drifting apart. ~keep
fn resolve_binding_statement(source: &str, name: &str) -> Option<String> {
    const REBASE: &str = "b.pathResolve(&.{";

    let mut name = name.to_owned();
    for _ in 0..MAX_BINDING_INDIRECTIONS {
        let marker = format!("const {name} = ");
        let start = source.find(&marker)? + marker.len();
        let statement = source[start..]
            .split_once(';')
            .map_or(&source[start..], |(head, _)| head);
        let Some(arguments) = statement.strip_prefix(REBASE) else {
            return Some(statement.to_owned());
        };
        let end = arguments.find("})")?;
        name = arguments[..end].rsplit(',').next()?.trim().to_owned();
    }
    None
}

/// The default path a scaffolded `build.zig` binds to `name`.
///
/// Two shapes, both of which alef's own scaffold has emitted:
/// `const <name> = b.option(...) orelse "<default>";`, and the build-root-rebased pair
/// `const <name> = b.pathResolve(&.{ <build root>, <inner> });` whose `<inner>` is itself a
/// binding resolved the same way. The build-root argument is deliberately dropped rather than
/// joined in: the caller already knows that directory as the manifest's own, and it is an
/// expression here (`b.build_root.path orelse "."`) rather than a literal this could read. ~keep
fn binding_default(source: &str, name: &str) -> Option<String> {
    orelse_literal(&resolve_binding_statement(source, name)?)
}

/// The literal in `... orelse "<default>"` within a single statement.
fn orelse_literal(statement: &str) -> Option<String> {
    const ORELSE: &str = "orelse ";

    let default = statement.find(ORELSE)? + ORELSE.len();
    let literal = statement[default..].trim_start().strip_prefix('"')?;
    let end = literal.find('"')?;
    Some(literal[..end].to_owned())
}

/// The build option *name* in `b.option([]const u8, "<name>", "...") orelse "..."` within a
/// single statement — the string literal that precedes `orelse`, not the description that follows
/// the name. ~keep
fn option_name_in_statement(statement: &str) -> Option<String> {
    let orelse_at = statement.find("orelse ")?;
    let head = &statement[..orelse_at];
    let start = head.find('"')? + 1;
    let end = start + head[start..].find('"')?;
    Some(head[start..end].to_owned())
}

/// The build option name and resolved absolute default directory `addLibraryPath(.{ .cwd_relative
/// = <expr> })` binds, when `<expr>` traces back to a `b.option(...) orelse "<literal>"` — the
/// exact shape alef's own `scaffold_zig` template emits for `ffi_path`. `None` when the manifest
/// carries no such declaration, or when `<expr>` is a bare string literal with no backing option
/// (nothing to override through `-D`/dependency args, so there is nothing this can hand back).
/// ~keep
pub(crate) fn zig_manifest_library_path_option(manifest: &Path) -> Result<Option<(String, PathBuf)>> {
    const DECLARATION: &str = "addLibraryPath(.{ .cwd_relative = ";

    let source = std::fs::read_to_string(manifest)?;
    let Some(occurrence) = source.split(DECLARATION).nth(1) else {
        return Ok(None);
    };
    let Some(end) = occurrence.find(" })") else {
        return Ok(None);
    };
    let expression = occurrence[..end].trim();
    let Some(statement) = resolve_binding_statement(&source, expression) else {
        return Ok(None);
    };
    let (Some(option_name), Some(default_literal)) = (option_name_in_statement(&statement), orelse_literal(&statement))
    else {
        return Ok(None);
    };
    let build_root = manifest.parent().unwrap_or(Path::new("."));
    Ok(Some((option_name, build_root.join(default_literal))))
}

/// The library base name in the manifest's first `linkSystemLibrary("<name>", .{})` call.
fn zig_manifest_link_library_name(source: &str) -> Option<String> {
    const DECLARATION: &str = "linkSystemLibrary(\"";

    let start = source.find(DECLARATION)? + DECLARATION.len();
    let end = start + source[start..].find('"')?;
    Some(source[start..end].to_owned())
}

/// Whether `directory` directly contains a linkable artifact for `lib_name` — checked by probing
/// the common shared/static library extensions rather than trusting `directory.exists()` alone,
/// and never inside a `deps/` subdirectory: that directory carries whatever feature set some other
/// cargo invocation unified, so a copy found only there cannot be trusted. Mirrors
/// `publish::ffi_stage`'s same refusal to accept a `deps/`-only copy. ~keep
fn directory_has_ffi_library(directory: &Path, lib_name: &str) -> bool {
    const EXTENSIONS: [&str; 4] = ["dylib", "so", "a", "dll"];
    EXTENSIONS
        .iter()
        .any(|extension| directory.join(format!("lib{lib_name}.{extension}")).is_file())
}

/// The sibling `debug` directory of a `.../release` directory, or `None` when `release_dir`'s own
/// name is not literally `release` — a consumer who points the option somewhere else entirely (a
/// vendored prebuilt, a sibling repo) gets no guessed fallback rather than a wrong one. ~keep
fn sibling_profile_directory(release_dir: &Path) -> Option<PathBuf> {
    (release_dir.file_name()?.to_str()? == "release").then(|| release_dir.with_file_name("debug"))
}

/// Prefer the `release` profile's built FFI library, falling back to `debug` when only that one is
/// on disk.
///
/// `alef build` with no `--release` flag runs a plain `cargo build`, which produces
/// `target/debug/`; the scaffolded `build.zig` only ever searches `target/release/` by default
/// (`scaffold::languages::zig::scaffold_zig`), so every snippet that reaches this manifest fails
/// identically — "unable to find dynamic system library" — whenever the FFI crate was last built
/// without `--release`, even though a perfectly good, symbol-complete library sits one directory
/// over. Returns the option name to override and the absolute directory to set it to; `None` when
/// the release default already resolves (nothing to override) or when neither profile has the
/// library (let zig fail with its own message rather than pass a directory that will not help).
/// ~keep
pub(crate) fn resolve_ffi_library_override(manifest: &Path) -> Result<Option<(String, PathBuf)>> {
    let Some((option_name, release_dir)) = zig_manifest_library_path_option(manifest)? else {
        return Ok(None);
    };
    let source = std::fs::read_to_string(manifest)?;
    let Some(lib_name) = zig_manifest_link_library_name(&source) else {
        return Ok(None);
    };
    if directory_has_ffi_library(&release_dir, &lib_name) {
        return Ok(None);
    }
    let Some(debug_dir) = sibling_profile_directory(&release_dir) else {
        return Ok(None);
    };
    Ok(directory_has_ffi_library(&debug_dir, &lib_name).then_some((option_name, debug_dir)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_build_zig(with_include: bool) -> String {
        let include = if with_include {
            "module.addIncludePath(.{ .cwd_relative = ffi_include });\n"
        } else {
            ""
        };
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const ffi_include = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"vendor/include\";\n\
             \x20   const module = b.addModule(\"sample_binding\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   {include}\
             }}\n"
        )
    }

    /// The include declaration alef's scaffold emits today: the option default is rebased onto the
    /// package's own build root before it reaches `.cwd_relative`, so the literal the parser needs
    /// sits one `const` further away than it used to.
    pub(crate) fn build_root_rebased_build_zig(package_name: &str) -> String {
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const target = b.standardTargetOptions(.{{}});\n\
             \x20   const optimize = b.standardOptimizeOption(.{{}});\n\
             \x20   const build_root = b.build_root.path orelse \".\";\n\
             \x20   const ffi_include_option = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"vendor/include\";\n\
             \x20   const ffi_include = b.pathResolve(&.{{ build_root, ffi_include_option }});\n\
             \x20   const module = b.addModule(\"{package_name}\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   module.addIncludePath(.{{ .cwd_relative = ffi_include }});\n\
             }}\n"
        )
    }

    /// The `ffi_path`/`addLibraryPath` declaration alef's scaffold emits today, mirroring
    /// [`build_root_rebased_build_zig`] but for the library search directory instead of the
    /// include directory -- the shape `resolve_ffi_library_override` reads.
    pub(crate) fn build_root_rebased_ffi_path_build_zig(lib_name: &str, default_dir: &str) -> String {
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const target = b.standardTargetOptions(.{{}});\n\
             \x20   const optimize = b.standardOptimizeOption(.{{}});\n\
             \x20   const build_root = b.build_root.path orelse \".\";\n\
             \x20   const ffi_path_option = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_path\",\n\
             \x20       \"Path to directory containing lib{lib_name}.{{dylib,so,dll,a}}\"\n\
             \x20   ) orelse \"{default_dir}\";\n\
             \x20   const ffi_path = b.pathResolve(&.{{ build_root, ffi_path_option }});\n\
             \x20   const module = b.addModule(\"sample_binding\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .target = target,\n\
             \x20       .optimize = optimize,\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   module.addLibraryPath(.{{ .cwd_relative = ffi_path }});\n\
             \x20   module.linkSystemLibrary(\"{lib_name}\", .{{}});\n\
             }}\n"
        )
    }

    /// Alef's own scaffold binds the include directory through a `b.option(...) orelse`
    /// default, so reading only string literals finds nothing in the manifest Alef itself writes.
    #[test]
    fn manifest_include_paths_resolve_through_the_build_option_default() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(true)).unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["vendor/include"]);
    }

    #[test]
    fn manifest_include_paths_accept_a_direct_string_literal() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(
            &manifest,
            "const module = b.addModule(\"sample_binding\", .{\n    .root_source_file = b.path(\"src/root.zig\"),\n});\nmodule.addIncludePath(.{ .cwd_relative = \"include\" });\n",
        )
        .unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["include"]);
    }

    #[test]
    fn a_manifest_without_an_include_declaration_contributes_no_paths() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(false)).unwrap();

        assert!(zig_manifest_include_paths(&manifest).unwrap().is_empty());
    }

    /// Regression: reading only `const <name> = b.option(...) orelse "<literal>"` finds nothing in
    /// the manifest alef writes today, because the binding `.cwd_relative` names is the rebased one.
    #[test]
    fn manifest_include_paths_resolve_through_a_build_root_rebased_binding() {
        let directory = tempfile::tempdir().expect("project directory");
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, build_root_rebased_build_zig("sample_binding")).unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["vendor/include"]);
    }

    #[test]
    fn binding_default_gives_up_rather_than_looping_on_a_cyclic_binding() {
        let source = "const a = b.pathResolve(&.{ root, b_name });\nconst b_name = b.pathResolve(&.{ root, a });\n";

        assert_eq!(binding_default(source, "a"), None);
    }

    /// Regression for the defect this module exists to fix: a manifest whose `ffi_path` default
    /// names a `release` directory that was never built (`alef build` with no `--release` produces
    /// `target/debug/` instead) must resolve to the `debug` sibling that *was* built, carrying the
    /// option name the scaffold declared so the caller can thread it through `b.dependency(...)`.
    #[test]
    fn resolve_ffi_library_override_falls_back_to_debug_when_release_is_missing() {
        let directory = tempfile::tempdir().expect("project directory");
        std::fs::create_dir_all(directory.path().join("debug")).unwrap();
        std::fs::write(directory.path().join("debug/libsample_ffi.dylib"), "fake").unwrap();
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        let (option_name, resolved) = resolve_ffi_library_override(&manifest).unwrap().unwrap();

        assert_eq!(option_name, "ffi_path");
        // Lexical, not canonicalized -- `zig_manifest_library_path_option` never touches the
        // filesystem to collapse `..`, matching `relative_path`'s own no-canonicalize rationale
        // elsewhere in this module (avoiding a surprise resolution through a symlink). The
        // uncollapsed form still resolves correctly on disk, proven by
        // `a_snippet_links_against_the_debug_profile_when_release_is_missing`'s real `zig build`.
        assert_eq!(resolved, package.join("../debug"));
    }

    /// Negative control for the regression above: when the manifest's own `release` default
    /// already has the library, there is nothing to override -- an eager fix that always redirects
    /// to `debug` would break this case, so it must stay green. Both profiles carry a library here
    /// on purpose: with only `release` built, an always-redirect-to-`debug` bug would coincidentally
    /// still return `None` (nothing at the guessed `debug` sibling either), so it would not catch
    /// the over-eager shape. Building both is what makes "prefers release, does not gratuitously
    /// redirect" an observable difference. ~keep
    #[test]
    fn resolve_ffi_library_override_is_a_no_op_when_release_already_has_the_library() {
        let directory = tempfile::tempdir().expect("project directory");
        std::fs::create_dir_all(directory.path().join("release")).unwrap();
        std::fs::write(directory.path().join("release/libsample_ffi.dylib"), "fake").unwrap();
        std::fs::create_dir_all(directory.path().join("debug")).unwrap();
        std::fs::write(directory.path().join("debug/libsample_ffi.dylib"), "fake").unwrap();
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(resolve_ffi_library_override(&manifest).unwrap(), None);
    }

    /// A third control: when *neither* profile has been built, there is nothing this can offer --
    /// it must not hand back a debug directory that does not exist either.
    #[test]
    fn resolve_ffi_library_override_is_a_no_op_when_neither_profile_is_built() {
        let directory = tempfile::tempdir().expect("project directory");
        let package = directory.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let manifest = package.join("build.zig");
        std::fs::write(
            &manifest,
            build_root_rebased_ffi_path_build_zig("sample_ffi", "../release"),
        )
        .unwrap();

        assert_eq!(resolve_ffi_library_override(&manifest).unwrap(), None);
    }

    #[test]
    fn directory_has_ffi_library_never_credits_a_deps_only_copy() {
        let directory = tempfile::tempdir().expect("project directory");
        std::fs::create_dir_all(directory.path().join("deps")).unwrap();
        std::fs::write(directory.path().join("deps/libsample_ffi.dylib"), "fake").unwrap();

        assert!(!directory_has_ffi_library(directory.path(), "sample_ffi"));
    }
}
