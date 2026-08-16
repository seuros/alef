use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use crate::core::template_versions::toolchain;
use crate::scaffold::{readme_language_configured, scaffold_meta};
use std::path::PathBuf;

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
/// 2. Otherwise, any other visible function is checked for existence via `@hasDecl` at comptime.
///    Calling an arbitrary function generically isn't safe (unknown allocator/ownership/JSON
///    conversion needs per parameter), but its declaration existing is still a real, falsifiable
///    fact about the generated output.
/// 3. Otherwise, a visible type or enum is checked the same way.
/// 4. Only when the API surface is genuinely empty (e.g. scaffolding before any Rust code exists)
///    does this fall back to asserting the module resolves at all — there is nothing else to
///    assert against yet, and once real items exist this file is never regenerated over. ~keep
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
        return import_line + &hasdecl_test(module_name, &f.name, "function");
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

/// Comptime `@hasDecl` existence check against `name` (a real declaration in the currently
/// generated API surface), used when no function is safe to call generically.
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
    /// (unknown allocator/ownership/JSON conversion needs), so this falls back to a comptime
    /// `@hasDecl` existence check — still a real, falsifiable fact about the generated output.
    #[test]
    fn falls_back_to_hasdecl_for_a_non_trivial_function() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "greet".to_string(),
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_zig_test(&api, &minimal_config(), "my_lib");

        assert!(out.contains("if (!@hasDecl(my_lib, \"greet\"))"), "got:\n{out}");
        assert!(out.contains("@compileError"), "got:\n{out}");
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
}
