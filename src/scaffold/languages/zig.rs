use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use crate::core::template_versions::toolchain;
use crate::scaffold::{readme_language_configured, scaffold_meta};
use anyhow::Context as _;
use std::path::{Path, PathBuf};

pub(crate) fn scaffold_zig(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let ffi_lib_name = config.ffi_lib_name();
    let module_name = config.zig_module_name();
    let ffi_crate_path = config.ffi_crate_path();

    let capsule_imports_block: String = config
        .zig
        .as_ref()
        .map(|c| {
            let import_names = crate::core::config::languages::zig_capsule_import_names(&c.capsule_types);
            if import_names.is_empty() {
                return String::new();
            }
            let mut block = String::new();
            for name in &import_names {
                block.push_str(&format!(
                    "\n    const {name}_dep = b.dependency(\"{name}\", .{{\n        \
                     .target = target,\n        .optimize = optimize,\n    }});\n    \
                     module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n    \
                     test_module.addImport(\"{name}\", {name}_dep.module(\"{name}\"));\n"
                ));
            }
            block
        })
        .unwrap_or_default();

    let build_zig = format!(
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Default library/include search paths follow the conventional Cargo workspace
    // layout. `alef publish package --lang zig` rewrites this file for the
    // distributed tarball so consumers link the bundled lib/ and include/ dirs.
    // Override with -Dffi_path=... and -Dffi_include_path=... if your layout differs.
    const ffi_path = b.option(
        []const u8,
        "ffi_path",
        "Path to directory containing lib{ffi_lib}.{{dylib,so,dll,a}}"
    ) orelse "../../target/release";

    const ffi_include = b.option(
        []const u8,
        "ffi_include_path",
        "Path to directory containing the FFI C header"
    ) orelse "{ffi_crate_path}/include";

    const module = b.addModule("{module_name}", .{{
        .root_source_file = b.path("src/{module_name}.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    module.linkSystemLibrary("{ffi_lib}", .{{}});

    // Scaffold also seeds `test/{module_name}_test.zig` (create-only — never overwrites
    // a real test suite once one exists) so `zig build test` has a real target to compile
    // from day one instead of silently re-running `src/{module_name}.zig` with zero `test`
    // blocks. ~keep
    const test_module = b.createModule(.{{
        .root_source_file = b.path("test/{module_name}_test.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    test_module.addImport("{module_name}", module);
    test_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    test_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    test_module.linkSystemLibrary("{ffi_lib}", .{{}});
{capsule_imports_block}
    const tests = b.addTest(.{{
        .root_module = test_module,
    }});

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);

    const example_module = b.createModule(.{{
        .root_source_file = b.path("examples/example.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    }});
    example_module.addImport("{module_name}", module);
    example_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});
    example_module.addIncludePath(.{{ .cwd_relative = ffi_include }});
    example_module.linkSystemLibrary("{ffi_lib}", .{{}});

    const example_exe = b.addExecutable(.{{
        .name = "example",
        .root_module = example_module,
    }});
    const run_example = b.addRunArtifact(example_exe);
    const example_step = b.step("example", "Run the example");
    example_step.dependOn(&run_example.step);
}}
"#,
        module_name = module_name,
        ffi_lib = ffi_lib_name,
        ffi_crate_path = ffi_crate_path,
        capsule_imports_block = capsule_imports_block,
    );

    let fingerprint = zig_fingerprint(&module_name);

    let zig_capsule_deps: String = config
        .zig
        .as_ref()
        .map(|c| {
            let mut entries: Vec<String> = c
                .capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .filter_map(|cap| {
                    let import_name = crate::core::config::languages::zig_capsule_import_name(&cap.host_type)?;
                    let hash_field = if cap.package_version.is_empty() {
                        String::new()
                    } else {
                        format!("\n            .hash = \"{}\",", cap.package_version)
                    };
                    Some(format!(
                        "        .{import_name} = .{{\n            .url = \"{}\",{}\n        }},",
                        cap.package, hash_field
                    ))
                })
                .collect();
            entries.sort();
            entries.dedup();
            entries.join("\n")
        })
        .unwrap_or_default();
    let dependencies_block = if zig_capsule_deps.is_empty() {
        ".{}".to_string()
    } else {
        format!(".{{\n{zig_capsule_deps}\n    }}")
    };

    let build_zig_zon = format!(
        r#".{{
    .name = .{module_name},
    .version = "{version}",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = {dependencies_block},
    .paths = .{{
        "build.zig",
        "build.zig.zon",
        "src",
    }},
}}
"#,
        module_name = module_name,
        version = version,
        fingerprint = fingerprint,
        min_zig = toolchain::MIN_ZIG_VERSION,
        dependencies_block = dependencies_block,
    );

    let gitignore = "zig-cache/\nzig-out/\n.zig-cache/\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.zig]\nindent_style = space\nindent_size = 4\n";
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();

    let readme = format!(
        r#"# {module_name}

{description}

## Installation

Install Zig from [ziglang.org](https://ziglang.org/download/).

## Building

```sh
zig build
zig build test
```

## Usage

Add to your `build.zig.zon`:

```text
.dependencies = .{{
    .{module_name} = .{{
        .path = "path/to/{module_name}",
    }},
}},
```
"#,
        module_name = module_name,
        description = meta.description,
    ) + &license_section;

    let example_zig = r#"const std = @import("std");

pub fn main() !void {
    var threaded: std.Io.Threaded = .init(std.heap.smp_allocator, .{});
    defer threaded.deinit();

    var stdout_buffer: [64]u8 = undefined;
    var stdout_writer = std.Io.File.stdout().writer(threaded.io(), &stdout_buffer);
    const stdout = &stdout_writer.interface;

    try stdout.print("Example: module loaded successfully\n", .{});
    try stdout.flush();
}
"#;

    let main_zig = format!(
        "// Generated by alef. Imports the full {module_name} API.\npub const api = @import(\"{module_name}.zig\");\n",
        module_name = module_name,
    );

    let test_zig = scaffold_zig_test(api, config, &module_name);

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from("packages/zig/build.zig"),
            content: build_zig,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/build.zig.zon"),
            content: build_zig_zon,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/zig/test/{module_name}_test.zig")),
            content: test_zig,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/examples/example.zig"),
            content: example_zig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/zig/src/main.zig"),
            content: main_zig.to_string(),
            generated_header: false,
        },
    ];
    // See the matching comment in `scaffold_swift`: once `[crates.readme.languages.zig]`
    // is configured, the README module owns this path end-to-end, and scaffold must not
    // compete with it as a second writer (#555). Inserted at its original position
    // (after `.editorconfig`, before the examples) rather than appended, so file order
    // is unchanged for languages that still rely on this placeholder. ~keep
    if !readme_language_configured(config, "zig") {
        files.insert(
            4,
            GeneratedFile {
                path: PathBuf::from("packages/zig/README.md"),
                content: readme,
                generated_header: false,
            },
        );
    }
    Ok(files)
}

/// Repair a pre-existing `packages/zig/build.zig` whose `test_module` still points at the
/// generated `src/<module>.zig` (zero `test` blocks) instead of the seeded
/// `test/<module>_test.zig` — the exact defect fixed in [`scaffold_zig`] above.
///
/// `build.zig` is emitted `generated_header: false` (create-only: see
/// `write_scaffold_files_report`'s ownership guard in
/// `cli::pipeline::generate::scaffold`), and deliberately so — every consumer repo checked
/// while designing this
/// migration (liter-llm, tree-sitter-language-pack, html-to-markdown) carries direct,
/// hand-written `build.zig` commits, including at least one per repo hand-patching this
/// *exact* `test_module` defect before this migration existed, plus unrelated fixes (a
/// corrected `ffi_include_path` default after a crate rename) that the generator's own
/// template still does not know how to reproduce. A full regenerate-and-overwrite (e.g. by
/// giving the file a self-marker so the normal write path could update it) would silently
/// destroy those. So this repairs only the one known-bad shape, byte-for-byte, and leaves
/// every other line — indentation, added build steps, unrelated hand fixes — untouched.
///
/// Detection is scoped to the `const test_module = b.createModule(.{ ... });` block only
/// (bounded by its own `});`), so the *library* module's identical-looking
/// `.root_source_file = b.path("src/<module>.zig")` line is never a candidate — only the
/// `test_module` one is defective. Silently returns `Ok(false)` (no-op) when: the file does
/// not exist yet (nothing to migrate), the `test_module` block is missing (not this shape at
/// all — a from-scratch hand-authored `build.zig`), or the block's `.root_source_file` no
/// longer matches the known-bad `src/*.zig` pattern (already migrated, or hand-fixed to some
/// other shape) — idempotent by construction, so calling this on every scaffold run is safe.
///
/// Also inserts `test_module.addImport("<module>", module);` immediately after the block's
/// closing `});`, but only when that exact self-import is not already present — the pre-fix
/// template never wired it, so a freshly repointed test module would otherwise fail to
/// resolve `@import("<module>")`. The check is deliberately for the whole call and not for a
/// bare `test_module.addImport(` prefix: a consumer may already import unrelated third-party
/// modules into the test module (tree-sitter-language-pack imports `tree_sitter`), and a
/// prefix match would read those as the self-import and skip the one line that matters. ~keep
pub(crate) fn migrate_build_zig_test_target(base_dir: &Path) -> anyhow::Result<bool> {
    let path = base_dir.join("packages/zig/build.zig");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };

    let Some(migrated) = repair_build_zig_test_target(&content) else {
        return Ok(false);
    };
    if migrated == content {
        return Ok(false);
    }

    let parent = path.parent().context("build.zig path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    tracing::warn!(
        path = %path.display(),
        "repaired pre-existing build.zig: test_module now points at test/<module>_test.zig \
         instead of the generated src/<module>.zig (zero test blocks)"
    );
    Ok(true)
}

/// Pure line-based transform behind [`migrate_build_zig_test_target`]. Returns `None` when
/// `content` does not contain the known-bad shape at all (no migration candidate); returns
/// `Some(content.to_string())` unchanged when the shape was already repaired by a prior call
/// in the same content (defensive — the caller already short-circuits on this too).
fn repair_build_zig_test_target(content: &str) -> Option<String> {
    const BLOCK_ANCHOR: &str = "const test_module = b.createModule(.{";
    const BAD_PREFIX: &str = ".root_source_file = b.path(\"src/";
    const BAD_SUFFIX: &str = ".zig\"),";

    let lines: Vec<&str> = content.lines().collect();
    let block_start = lines.iter().position(|line| line.trim() == BLOCK_ANCHOR)?;
    let block_end = lines[block_start..]
        .iter()
        .position(|line| line.trim() == "});")
        .map(|offset| block_start + offset)?;

    let bad_line_index = (block_start..block_end).find(|&index| {
        let trimmed = lines[index].trim();
        trimmed.starts_with(BAD_PREFIX) && trimmed.ends_with(BAD_SUFFIX)
    })?;

    let trimmed = lines[bad_line_index].trim();
    let module_name = trimmed.strip_prefix(BAD_PREFIX)?.strip_suffix(BAD_SUFFIX)?;
    // Leading indent only (`trim_start`, not `trim`): `line.len() - line.trim().len()` would
    // count *trailing* whitespace too, and if the matched line ever carried any, that extra
    // width would be byte-sliced off the front here, corrupting the rewritten line. This
    // repair's entire purpose is exact preservation of everything outside the one field it
    // touches, so getting this slice wrong is a real correctness bug, not a style nit. ~keep
    let field_indent = &lines[bad_line_index][..lines[bad_line_index].len() - lines[bad_line_index].trim_start().len()];
    // Statement-level indent (one level shallower than the struct-literal field above),
    // taken from the block's own closing `});` line, for the sibling `test_module.*`
    // call this may insert — not `field_indent`, which belongs to a field one level deeper. ~keep
    let stmt_indent = &lines[block_end][..lines[block_end].len() - lines[block_end].trim_start().len()];

    let mut repaired: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    repaired[bad_line_index] =
        format!("{field_indent}.root_source_file = b.path(\"test/{module_name}_test.zig\"),");

    let import_call = format!("test_module.addImport(\"{module_name}\", module);");
    if !repaired.iter().any(|line| line.contains(import_call.as_str())) {
        repaired.insert(block_end + 1, format!("{stmt_indent}{import_call}"));
    }

    let mut joined = repaired.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// Build the seed content for `test/{module_name}_test.zig`.
///
/// `build.zig`'s `test_module` now points here instead of re-compiling `src/{module_name}.zig`
/// (which carries zero `test` blocks — see the Defect-1 fix in `scaffold_zig`). `write_scaffold_files_report`
/// treats `generated_header: false` as create-only, so once a real suite exists at this path alef never
/// overwrites it; this only ever seeds a fresh project.
///
/// The seed must not be vacuous — a `zig build test` that always passes regardless of what alef
/// generated reproduces the exact "0 test blocks, silently green" defect one layer down. So this
/// asserts against the *real*, currently-generated API surface (`api`), in order of how strong a
/// check is safely synthesizable without knowing arbitrary function signatures:
///
/// 1. A visible zero-parameter function returning a bare primitive is actually called end-to-end
///    (real FFI link, real invocation) — the strongest check: it fails on a broken build, a link
///    error, or a removed/renamed export, not just a missing declaration.
/// 2. Otherwise, any other visible function is *referenced without being called*
///    (`_ = &{module}.{fn};`). Calling an arbitrary function generically isn't safe (unknown
///    allocator/ownership/JSON conversion needs per parameter), but a bare reference needs no
///    knowledge of the argument contract at all, and still forces Zig to semantically analyse
///    the wrapper's body and to resolve the extern C symbol that body calls. See
///    [`symbol_reference_test`] for the measured evidence and for the one class of change this
///    provably cannot catch.
/// 3. Otherwise, a visible type or enum is checked for existence via `@hasDecl` at comptime.
///    Types and enums have no referenceable-as-value form (`&SomeType` is not valid Zig), so
///    comptime existence is the strongest fact available about them — but this tier compiles no
///    wrapper body and links nothing, so it catches only a rename or a removal.
/// 4. Only when the API surface is genuinely empty (e.g. scaffolding before any Rust code exists)
///    does this fall back to asserting the module resolves at all — there is nothing else to
///    assert against yet, and once real items exist this file is never regenerated over.
///
/// Every tier draws its candidate from `api` (the parsed Rust surface) and never from the
/// generated `.zig` file's declaration list. That is why the Zig binding generator's synthetic
/// helpers — `_last_error`, `_free_string`, `_first_error`, emitted directly as text by
/// `backends::zig::gen_bindings::helpers` with no backing `FunctionDef` — can never be picked as
/// the seed subject: they are structurally invisible here, not filtered out by name. ~keep
fn scaffold_zig_test(api: &ApiSurface, config: &ResolvedCrateConfig, module_name: &str) -> String {
    let (exclude_functions, exclude_types) = zig_binding_exclusions(api, config);
    let function_is_visible = |f: &FunctionDef| !f.binding_excluded && !exclude_functions.contains(&f.name);
    let import_line = format!("const {module_name} = @import(\"{module_name}\");\n\n");

    let trivial_call_fn = api
        .functions
        .iter()
        .find(|f| function_is_visible(f) && f.params.is_empty() && matches!(f.return_type, TypeRef::Primitive(_)));
    if let Some(f) = trivial_call_fn {
        return import_line + &trivial_call_test(module_name, f);
    }

    if let Some(f) = api.functions.iter().find(|f| function_is_visible(f)) {
        return import_line + &symbol_reference_test(module_name, &f.name);
    }

    if let Some(t) = api
        .types
        .iter()
        .find(|t| !t.binding_excluded && !t.is_trait && !exclude_types.contains(&t.name))
    {
        return import_line + &hasdecl_test(module_name, &t.name, "type");
    }

    if let Some(e) = api
        .enums
        .iter()
        .find(|e| !e.binding_excluded && !exclude_types.contains(&e.name))
    {
        return import_line + &hasdecl_test(module_name, &e.name, "enum");
    }

    import_line
        + "// No generated API surface exists yet for this crate, so there is nothing to assert\n\
           // against beyond the module resolving. Once real functions/types exist, alef never\n\
           // regenerates over this file — it is a create-only scaffold seed. ~keep\n\
           test \"module imports successfully\" {\n    _ = "
        + module_name
        + ";\n}\n"
}

/// Names excluded from Zig binding generation, mirroring the same union `ZigBackend::generate_bindings`
/// (`src/backends/zig/gen_bindings/mod.rs`) computes: `[crates.zig]`/`[crates.ffi]` exclude lists plus
/// any type explicitly marked `binding_excluded`. Kept in sync deliberately rather than shared, since
/// the generator's version also folds in transitive signature exclusion this seed-picker doesn't need
/// to replicate (it only needs *a* safe, visible name — not the exhaustive filtered set).
fn zig_binding_exclusions(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut exclude_functions: std::collections::HashSet<String> = config
        .zig
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut exclude_types: std::collections::HashSet<String> = config
        .zig
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    (exclude_functions, exclude_types)
}

/// The strongest safe check: actually call a visible zero-arg, primitive-returning function
/// end-to-end (real FFI link, real invocation), so a broken build or a removed/renamed export
/// fails `zig build test` immediately instead of shipping green with a suite that links nothing.
fn trivial_call_test(module_name: &str, f: &FunctionDef) -> String {
    let call = if f.error_type.is_some() {
        format!("try {module_name}.{}()", f.name)
    } else {
        format!("{module_name}.{}()", f.name)
    };
    format!(
        "// Calls the generated `{fn_name}` binding end-to-end (real FFI link, real call), so a\n\
         // broken build or a removed/renamed export fails `zig build test` immediately instead of\n\
         // shipping green with a suite that links nothing. Create-only scaffold seed. ~keep\n\
         test \"{module_name}.{fn_name} runs\" {{\n    const result = {call};\n    _ = result;\n}}\n",
        fn_name = f.name,
    )
}

/// Reference — but do not call — a visible function, for the (common) case where no function in
/// the surface is safe to call generically.
///
/// `_ = &{module}.{fn};` needs no knowledge of the function's per-parameter ownership contract,
/// which is exactly why it is synthesizable where a real call is not. It is strictly stronger
/// than the comptime `@hasDecl` tier below it: taking the address of a function forces Zig to
/// semantically analyse that function's body and to resolve the extern C symbol the body calls,
/// so this tier fails on a link error or a wrapper-body type error, not merely on a rename.
///
/// Measured, not assumed — the html-to-markdown owner ran all three forms on Zig 0.16.0 with
/// controls. Against a wrapper whose extern symbol did not exist in the compiled library:
/// `@hasDecl` exited 0 ("All 1 tests passed"); `_ = &m.convert;` exited 1 with
/// `undefined symbol: _htm_probe_definitely_missing_symbol ... referenced by _lib.convert`; and
/// `_ = m.convert(1);` — the positive control — exited 1 with the same error. Against a type
/// error in the wrapper body with no extern involved: `@hasDecl` exited 0, `_ = &m.convert;`
/// exited 1 with `expected type '[]const u8', found 'c_int'`.
///
/// It cannot catch a C-level ABI change that preserves the symbol name; see the emitted comment,
/// which states that limit in the generated file where a reader of a green run will actually
/// encounter it. ~keep
fn symbol_reference_test(module_name: &str, name: &str) -> String {
    format!(
        "// `{name}` isn't a zero-arg, primitive-returning function this seed can safely call\n\
         // generically — its per-parameter allocator/ownership/JSON conversion contract is not\n\
         // knowable here — so this *references* it rather than calling it. That is not a weaker\n\
         // `@hasDecl`: taking the address forces Zig to semantically analyse the wrapper's body\n\
         // and to resolve the extern C symbol that body calls, neither of which a comptime\n\
         // `@hasDecl` does. Measured on Zig 0.16.0 with a deleted extern symbol: `@hasDecl` exits\n\
         // 0 (\"All 1 tests passed\"); this line exits 1 with `undefined symbol: ... referenced\n\
         // by ...`, matching a real call (the positive control). Same split for a type error in\n\
         // the wrapper body with no extern involved.\n\
         //\n\
         // LIMIT — read this before trusting a green run. This proves the symbol EXISTS and the\n\
         // wrapper typechecks. It does NOT prove the symbol is CORRECT. A C-level ABI change that\n\
         // preserves the symbol name is invisible to it: the linker resolves by name and C\n\
         // symbols carry no type information, so if the C signature changes and the generated Zig\n\
         // `extern` declaration is regenerated to match it, both move together and nothing ever\n\
         // disagrees. This closes \"the symbol does not exist\". It leaves \"the symbol lies\" wide\n\
         // open. Create-only scaffold seed. ~keep\n\
         test \"{module_name}.{name} symbol resolves\" {{\n    _ = &{module_name}.{name};\n}}\n",
    )
}

/// Comptime `@hasDecl` existence check against `name` (a real declaration in the currently
/// generated API surface). Used for types and enums, which — unlike functions — have no
/// referenceable-as-value form (`&SomeType` is not valid Zig), so comptime existence is the
/// strongest fact synthesizable about them.
///
/// This tier is deliberately the weakest one that still asserts something falsifiable. It
/// compiles no wrapper body and links nothing, so it catches a rename or a removal and nothing
/// else — a signature or ABI change that preserves the name passes it, and the symbol need not
/// exist in the compiled library at all for the test to go green. ~keep
fn hasdecl_test(module_name: &str, name: &str, kind: &str) -> String {
    format!(
        "// `{name}` isn't a zero-arg, primitive-returning function this seed can safely call\n\
         // generically, so this checks the generated {kind} exists at comptime instead. Create-only\n\
         // scaffold seed. ~keep\n\
         test \"{module_name} exposes `{name}`\" {{\n    \
             comptime {{\n        \
                 if (!@hasDecl({module_name}, \"{name}\")) {{\n            \
                     @compileError(\"{module_name} is missing expected declaration `{name}`\");\n        \
                 }}\n    \
             }}\n}}\n",
    )
}

/// Derive a deterministic 64-bit fingerprint from the package name.
/// Zig 0.16+ requires a `.fingerprint` field in `build.zig.zon` with structure
/// `(crc32_ieee(name) << 32) | id`, where `id` is a 32-bit value not equal to
/// `0x00000000` or `0xffffffff`. We use FNV-1a over the package name as the
/// stable id so regeneration is deterministic.
fn zig_fingerprint(name: &str) -> u64 {
    let name_crc = crc32_ieee(name.as_bytes());
    let mut id: u32 = 0x811c_9dc5;
    for byte in name.as_bytes() {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    ((name_crc as u64) << 32) | (id as u64)
}

fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{EnumDef, PrimitiveType, TypeDef};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
    }

    fn trivial_function(name: &str) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            ..Default::default()
        }
    }

    /// The strongest available check: a visible zero-arg, primitive-returning function is
    /// actually called, not just checked for existence.
    #[test]
    fn calls_a_visible_trivial_function_end_to_end() {
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("const my_lib = @import(\"my_lib\");"), "got:\n{out}");
        assert!(out.contains("test \"my_lib.ping runs\""), "got:\n{out}");
        assert!(out.contains("const result = my_lib.ping();"), "got:\n{out}");
        assert!(
            !out.contains("try my_lib.ping()"),
            "non-fallible call must not use try, got:\n{out}"
        );
    }

    /// A fallible zero-arg primitive function must be called with `try` — Zig's `test` blocks
    /// tolerate a returned error, so this compiles and surfaces a real runtime error as a
    /// genuine test failure instead of a Rust-side type mismatch.
    #[test]
    fn calls_a_fallible_trivial_function_with_try() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                error_type: Some("MyError".to_string()),
                ..trivial_function("ping")
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("const result = try my_lib.ping();"), "got:\n{out}");
    }

    /// A function that isn't zero-arg-and-primitive-returning can't be called generically
    /// (unknown allocator/ownership/JSON conversion needs), so it is *referenced* instead —
    /// which still forces Zig to analyse the wrapper body and resolve its extern C symbol,
    /// unlike the comptime `@hasDecl` tier below it. Pins the emitted line exactly.
    #[test]
    fn references_without_calling_a_function_that_fails_the_trivial_call_tier() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("test \"my_lib.greet symbol resolves\" {"), "got:\n{out}");
        assert!(out.contains("\n    _ = &my_lib.greet;\n"), "got:\n{out}");
        assert!(
            !out.contains("my_lib.greet("),
            "the reference tier must never synthesize a call, got:\n{out}"
        );
        assert!(
            // `symbol_reference_test`'s own `~keep` comment legitimately discusses `@hasDecl`
            // in prose (contrasting this stronger tier against it), so a bare substring check
            // false-positives on that comment. `if (!@hasDecl(` is `hasdecl_test`'s actual
            // functional emission (see line ~586) and is what "fell through" means. ~keep
            !out.contains("if (!@hasDecl("),
            "a visible function must not fall through to the weaker comptime tier, got:\n{out}"
        );
    }

    /// The emitted comment must state the one thing this tier provably cannot catch. An
    /// overclaiming comment is precisely how a green run gets misread as ABI verification,
    /// so the honest limit is part of the contract, not decoration.
    #[test]
    fn reference_tier_documents_that_it_cannot_catch_an_abi_change() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("It does NOT prove the symbol is CORRECT."), "got:\n{out}");
        assert!(
            out.contains("A C-level ABI change that"),
            "the limit must name the uncaught change class, got:\n{out}"
        );
        assert!(out.contains("~keep"), "got:\n{out}");
    }

    /// The T1 boundary is `matches!(return_type, TypeRef::Primitive(_))` — a *bare* primitive.
    /// An optional-returning function is the real-world shape that falls through it (an
    /// optional slice is what a fallible getter binding actually returns), and until now no
    /// test pinned that boundary with anything but `TypeRef::String`. `Optional(Primitive)` is
    /// the sharp case: it contains a primitive but is not one, so a `matches!` loosened to
    /// look inside the `Box` would silently start synthesizing an uncallable call.
    #[test]
    fn an_optional_return_falls_through_the_trivial_call_tier() {
        for return_type in [
            TypeRef::Optional(Box::new(TypeRef::String)),
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
        ] {
            let api = ApiSurface {
                functions: vec![FunctionDef {
                    name: "maybe_name".to_string(),
                    return_type: return_type.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

            assert!(
                out.contains("\n    _ = &my_lib.maybe_name;\n"),
                "{return_type:?} must fall through to the reference tier, got:\n{out}"
            );
            assert!(
                !out.contains("my_lib.maybe_name()"),
                "{return_type:?} is not a bare primitive and must never be called, got:\n{out}"
            );
        }
    }

    /// The Zig binding generator emits `_last_error`, `_free_string` and `_first_error` directly
    /// as text (`backends::zig::gen_bindings::helpers`) with no backing `FunctionDef`, so they
    /// exist as public declarations in the generated module but not in `ApiSurface`. Exclusion
    /// from the seed is therefore structural — every tier draws only from `api` — rather than a
    /// name filter that could be dropped. Pinned anyway: an implementation that ever picked its
    /// subject by inspecting the generated module's declarations would seed a test that asserts
    /// only against alef's own boilerplate and links nothing of the real surface, which is the
    /// vacuous-green failure this whole seed exists to prevent. Checked on the empty surface
    /// too, since that is the case where such an implementation would have nothing else to grab.
    #[test]
    fn never_seeds_against_the_synthetic_binding_helpers() {
        let non_trivial = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        for api in [non_trivial, ApiSurface::default()] {
            let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");
            for helper in ["_last_error", "_free_string", "_first_error"] {
                assert!(
                    !out.contains(helper),
                    "synthetic helper `{helper}` must never be the seed subject, got:\n{out}"
                );
            }
        }
    }

    /// The `@hasDecl` tier's defining weakness, pinned so nobody mistakes it for a link check:
    /// it emits no call and no reference, so it compiles no wrapper body and asks the linker to
    /// resolve nothing. The subject name appears only as a comptime string argument — never as
    /// a field access on the module. Catches a rename or a removal, nothing more.
    #[test]
    fn hasdecl_tier_neither_calls_nor_references_its_subject() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Widget".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Widget\"))"), "got:\n{out}");
        assert!(out.contains("comptime {"), "got:\n{out}");
        assert!(
            !out.contains("my_lib.Widget"),
            "the comptime tier must not emit a field access, got:\n{out}"
        );
        assert!(
            !out.contains("_ = &"),
            "the comptime tier must not emit a symbol reference, got:\n{out}"
        );
        assert!(
            !out.contains("Widget()"),
            "the comptime tier must not emit a call, got:\n{out}"
        );
    }

    /// `binding_excluded` functions were never emitted into the generated `.zig` file, so the
    /// seed must skip them rather than asserting against a declaration that doesn't exist.
    #[test]
    fn skips_binding_excluded_functions() {
        let api = ApiSurface {
            functions: vec![
                FunctionDef {
                    binding_excluded: true,
                    ..trivial_function("hidden")
                },
                trivial_function("visible"),
            ],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("my_lib.visible"), "got:\n{out}");
        assert!(!out.contains("hidden"), "got:\n{out}");
    }

    /// `[crates.zig] exclude_functions` mirrors the real binding generator's own filter
    /// (`ZigBackend::generate_bindings`), so a function excluded there must also be skipped
    /// here — otherwise the seed asserts against a declaration the real generator never emits.
    #[test]
    fn skips_functions_excluded_via_zig_config() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "my-lib"
sources = []

[crates.zig]
exclude_functions = ["ping"]
"#,
        );
        let api = ApiSurface {
            functions: vec![trivial_function("ping")],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &config, "my_lib");

        assert!(
            !out.contains("ping"),
            "excluded function must not be referenced, got:\n{out}"
        );
        assert!(out.contains("_ = my_lib;"), "got:\n{out}");
    }

    /// With no visible function at all, a visible type is checked for existence instead.
    #[test]
    fn falls_back_to_hasdecl_for_a_type_when_no_functions_exist() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Widget".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Widget\"))"), "got:\n{out}");
    }

    /// With no visible function or type, a visible enum is checked for existence instead.
    #[test]
    fn falls_back_to_hasdecl_for_an_enum_when_no_functions_or_types_exist() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("if (!@hasDecl(my_lib, \"Color\"))"), "got:\n{out}");
    }

    /// A genuinely empty API surface (no Rust code written yet) has nothing to assert
    /// against beyond the module resolving — the only honest seed content.
    #[test]
    fn falls_back_to_import_only_when_api_surface_is_empty() {
        let out = scaffold_zig_test(&ApiSurface::default(), &minimal_config(), "my_lib");

        assert!(out.contains("const my_lib = @import(\"my_lib\");"), "got:\n{out}");
        assert!(out.contains("test \"module imports successfully\""), "got:\n{out}");
        assert!(out.contains("_ = my_lib;"), "got:\n{out}");
        assert!(!out.contains("@hasDecl"), "got:\n{out}");
    }

    fn build_zig_of(config: &ResolvedCrateConfig) -> String {
        scaffold_zig(&ApiSurface::default(), config)
            .expect("scaffold")
            .into_iter()
            .find(|f| f.path == PathBuf::from("packages/zig/build.zig"))
            .expect("build.zig must be scaffolded")
            .content
    }

    /// Regression for a real defect found in html-to-markdown: the `ffi_include_path` default
    /// must come from `[crates.output] ffi`, never from a `{crate name}-ffi` template.
    ///
    /// This template *was* the derivation — `scaffold_zig` used a local
    /// `format!("{}-ffi", config.name)` until it was switched to `config.ffi_crate_path()`,
    /// which consults `[crates.output] ffi` first. The two agree only when the alef crate name
    /// happens to equal the FFI crate's directory stem, and in two of the three consumer repos
    /// it does not: html-to-markdown's crate is named `html-to-markdown-rs` while its FFI crate
    /// is `crates/html-to-markdown-ffi`, and tree-sitter-language-pack's crate is named
    /// `tree-sitter-language-pack` while its FFI crate is `crates/ts-pack-core-ffi`. Both repos
    /// carry a hand commit fixing the emitted default, and html-to-markdown additionally carries
    /// a `crates/html-to-markdown-rs-ffi/` directory holding nothing but a README — the tombstone
    /// the bad default pointed at. This pins the fix against the html-to-markdown shape verbatim,
    /// because the failure is silent: the path is only a default, so a build that overrides
    /// `-Dffi_include_path=` never notices it is wrong. ~keep
    #[test]
    fn ffi_include_default_follows_configured_output_path_not_the_crate_name() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "html-to-markdown-rs"
sources = []

[crates.output]
ffi = "crates/html-to-markdown-ffi/src/"
"#,
        );
        let build_zig = build_zig_of(&config);

        assert!(
            build_zig.contains(") orelse \"../../crates/html-to-markdown-ffi/include\";"),
            "got:\n{build_zig}"
        );
        assert!(
            !build_zig.contains("html-to-markdown-rs-ffi"),
            "the crate-name template must not leak back in, got:\n{build_zig}"
        );
    }

    /// With no `[crates.output] ffi` configured there is nothing better to go on, so the
    /// `crates/{name}-ffi` convention is the honest fallback — pinned so the fix above is
    /// understood as a precedence change, not as removing the convention.
    #[test]
    fn ffi_include_default_falls_back_to_the_crate_name_convention_when_unconfigured() {
        let build_zig = build_zig_of(&minimal_config());

        assert!(
            build_zig.contains(") orelse \"../../crates/my-lib-ffi/include\";"),
            "got:\n{build_zig}"
        );
    }

    /// A representative pre-fix `build.zig`: the library `module` and the `test_module`
    /// both point at `src/my_lib.zig` (the known-bad shape), and the FFI include default was
    /// hand-fixed after a crate rename — exactly the kind of hand edit found in every real
    /// consumer repo (liter-llm, tree-sitter-language-pack, html-to-markdown) this migration
    /// was designed against.
    fn known_bad_build_zig() -> String {
        r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const ffi_path = b.option(
        []const u8,
        "ffi_path",
        "Path to directory containing libmy_lib_ffi.{dylib,so,dll,a}"
    ) orelse "../../target/release";

    const ffi_include = b.option(
        []const u8,
        "ffi_include_path",
        "Path to directory containing the FFI C header"
    ) orelse "../../crates/my-lib-ffi/include"; // hand-fixed after a crate rename

    const module = b.addModule("my_lib", .{
        .root_source_file = b.path("src/my_lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    module.addLibraryPath(.{ .cwd_relative = ffi_path });
    module.addIncludePath(.{ .cwd_relative = ffi_include });
    module.linkSystemLibrary("my_lib_ffi", .{});

    const test_module = b.createModule(.{
        .root_source_file = b.path("src/my_lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    test_module.addLibraryPath(.{ .cwd_relative = ffi_path });
    test_module.addIncludePath(.{ .cwd_relative = ffi_include });
    test_module.linkSystemLibrary("my_lib_ffi", .{});

    const tests = b.addTest(.{
        .root_module = test_module,
    });

    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
}
"#
        .to_string()
    }

    /// The core repair: only the `test_module` block's `.root_source_file` is repointed at
    /// `test/my_lib_test.zig`, and the missing `addImport` wiring is inserted right after
    /// that block — while the identical-looking line in the *library* `module` block, and
    /// every hand-written line (the crate-rename `ffi_include_path` fix), survive verbatim.
    #[test]
    fn repairs_only_the_test_module_target() {
        let original = known_bad_build_zig();
        let repaired = repair_build_zig_test_target(&original).expect("known-bad shape must match");

        assert!(
            repaired.contains(".root_source_file = b.path(\"test/my_lib_test.zig\"),"),
            "got:\n{repaired}"
        );
        assert!(
            repaired.contains("test_module.addImport(\"my_lib\", module);"),
            "got:\n{repaired}"
        );
        // The library module's own identical-looking line must be untouched.
        assert!(
            repaired.contains("const module = b.addModule(\"my_lib\", .{\n        .root_source_file = b.path(\"src/my_lib.zig\"),"),
            "library module's root_source_file must not be touched, got:\n{repaired}"
        );
        // The hand-fixed ffi_include_path default must survive byte-for-byte.
        assert!(
            repaired.contains("orelse \"../../crates/my-lib-ffi/include\"; // hand-fixed after a crate rename"),
            "hand edit must survive untouched, got:\n{repaired}"
        );
        // Only one line changed in the test_module block itself, plus one inserted line —
        // every other line of the 50-odd-line file is byte-identical.
        let original_lines: std::collections::HashSet<&str> = original.lines().collect();
        let unexpected_new_lines: Vec<&str> = repaired
            .lines()
            .filter(|line| !original_lines.contains(line))
            .collect();
        assert_eq!(
            unexpected_new_lines,
            vec![
                "        .root_source_file = b.path(\"test/my_lib_test.zig\"),",
                "    test_module.addImport(\"my_lib\", module);",
            ],
            "no line beyond the two expected repairs should differ, got:\n{repaired}"
        );
    }

    /// Idempotent: running the repair against its own output finds nothing left to fix.
    #[test]
    fn is_a_no_op_once_already_repaired() {
        let repaired_once = repair_build_zig_test_target(&known_bad_build_zig()).expect("first repair applies");
        assert!(
            repair_build_zig_test_target(&repaired_once).is_none(),
            "a second pass over already-repaired content must be a no-op"
        );
    }

    /// A `build.zig` with no recognizable `test_module` block at all (a from-scratch,
    /// hand-authored file) is not a migration candidate and must be left alone.
    #[test]
    fn ignores_a_build_zig_without_a_test_module_block() {
        let custom = "const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n    _ = b;\n}\n";
        assert!(repair_build_zig_test_target(custom).is_none());
    }

    /// The current (already-fixed) generator output already points `test_module` at
    /// `test/<module>_test.zig` — this must not match the known-bad pattern and must be
    /// left alone, since re-touching an already-correct file on every run would defeat the
    /// whole point of the guard being idempotent.
    #[test]
    fn ignores_a_build_zig_already_pointing_at_the_test_dir() {
        let already_fixed = known_bad_build_zig().replacen(
            ".root_source_file = b.path(\"src/my_lib.zig\"),\n        .target = target,\n        .optimize = optimize,\n        .link_libc = true,\n    });\n    test_module.addLibraryPath",
            ".root_source_file = b.path(\"test/my_lib_test.zig\"),\n        .target = target,\n        .optimize = optimize,\n        .link_libc = true,\n    });\n    test_module.addImport(\"my_lib\", module);\n    test_module.addLibraryPath",
            1,
        );
        assert!(repair_build_zig_test_target(&already_fixed).is_none());
    }

    /// End-to-end control via [`migrate_build_zig_test_target`]: writes a known-bad
    /// `build.zig` carrying a genuine hand edit (the crate-rename `ffi_include_path` fix) to
    /// a tempdir, runs the migration, and proves the hand edit is still present afterward —
    /// the exact guarantee this migration exists to provide instead of a blind overwrite.
    #[test]
    fn migrates_on_disk_and_preserves_a_hand_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let zig_dir = dir.path().join("packages/zig");
        std::fs::create_dir_all(&zig_dir).expect("create packages/zig");
        std::fs::write(zig_dir.join("build.zig"), known_bad_build_zig()).expect("write build.zig");

        let changed = migrate_build_zig_test_target(dir.path()).expect("migration must not error");
        assert!(changed, "known-bad file must be reported as changed");

        let on_disk = std::fs::read_to_string(zig_dir.join("build.zig")).expect("read migrated build.zig");
        assert!(on_disk.contains(".root_source_file = b.path(\"test/my_lib_test.zig\"),"));
        assert!(
            on_disk.contains("orelse \"../../crates/my-lib-ffi/include\"; // hand-fixed after a crate rename"),
            "hand-fixed ffi_include_path default must survive the on-disk migration, got:\n{on_disk}"
        );

        // Running it again against the now-repaired file must be a no-op.
        let changed_again = migrate_build_zig_test_target(dir.path()).expect("second pass must not error");
        assert!(!changed_again, "second pass over an already-repaired file must be a no-op");
    }

    /// A `build.zig` that does not exist yet (nothing scaffolded so far) must not be
    /// created or error — there is nothing to migrate.
    #[test]
    fn is_a_no_op_when_build_zig_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let changed = migrate_build_zig_test_target(dir.path()).expect("must not error on a missing file");
        assert!(!changed);
        assert!(!dir.path().join("packages/zig/build.zig").exists());
    }
}
