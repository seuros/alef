use super::*;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

fn make_config(crate_name: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn required_formatters_always_include_rustfmt_and_poly() {
    let tools: Vec<&str> = required_formatters(&[Language::Python])
        .iter()
        .map(|f| f.tool)
        .collect();
    assert!(tools.contains(&"rustfmt"));
    assert!(tools.contains(&"poly"));
    assert!(!tools.contains(&"cargo-sort"), "python has no cargo-sort residual");
}

#[test]
fn required_formatters_add_cargo_sort_for_residual_languages() {
    for language in [
        Language::Wasm,
        Language::Ffi,
        Language::Ruby,
        Language::Elixir,
        Language::R,
    ] {
        let tools: Vec<&str> = required_formatters(&[language]).iter().map(|f| f.tool).collect();
        assert!(tools.contains(&"cargo-sort"), "{language} runs a cargo-sort residual");
    }
}

#[test]
fn formatter_error_includes_stdout_and_stderr() {
    let err = run_formatter(
        "sh",
        &["-c", "printf 'stdout text'; printf 'stderr text' >&2; exit 7"],
        Path::new("."),
    )
    .expect_err("formatter should fail");
    let msg = err.to_string();
    assert!(msg.contains("stdout text"), "missing stdout in error: {msg}");
    assert!(msg.contains("stderr text"), "missing stderr in error: {msg}");
}

#[test]
fn wasm_residual_is_cargo_sort_n_on_the_crate_dir() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "wasm residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "crates/sample-model-wasm"],
        "cargo sort -n arg must be the wasm crate directory"
    );
    assert_eq!(
        steps[0].work_dir,
        Path::new("/repo"),
        "wasm cargo sort runs at repo root"
    );
}

#[test]
fn wasm_residual_uses_configured_output_path() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]
[crates.output]
wasm = "crates/sample-pack-core-wasm/src/"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "crates/sample-pack-core-wasm"],
        "cargo sort arg must match the crate dir derived from the configured output path"
    );
}

#[test]
fn ffi_residual_is_cargo_sort_n_workspace_wide() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ffi, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "FFI residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "-n", "-w"],
        "FFI cargo sort must be workspace-wide with -n flag"
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn ruby_residual_sorts_the_native_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ruby, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "Ruby residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert_eq!(steps[0].args[1], "-n");
    assert!(
        steps[0].args[2].starts_with("ext/") && steps[0].args[2].ends_with("/native"),
        "cargo sort arg must target ext/<gem>/native, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/ruby"));
}

#[test]
fn elixir_residual_is_cargo_sort_n_only() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Elixir, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "Elixir residual must be cargo sort only");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert_eq!(steps[0].args[1], "-n");
    assert!(
        steps[0].args[2].starts_with("native/") && steps[0].args[2].ends_with("_nif"),
        "cargo sort arg must target native/<app>_nif, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/elixir"));
}

#[test]
fn r_residual_sorts_the_extendr_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::R, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "R residual must be a single cargo sort step");
    assert_eq!(steps[0].args, vec!["sort", "-n", "packages/r/src/rust"]);
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn csharp_has_no_residual() {
    let config = make_config("sample-model");
    assert!(language_residuals(&config, Language::Csharp, Path::new("/repo")).is_empty());
}

#[test]
fn languages_without_residuals_have_none() {
    let config = make_config("sample-model");
    for lang in [
        Language::Python,
        Language::Node,
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::Csharp,
    ] {
        assert!(
            language_residuals(&config, lang, Path::new("/repo")).is_empty(),
            "{lang:?} must have no residual native pass (poly formats it)"
        );
    }
}

#[test]
fn cargo_sort_residuals_returns_fixed_set() {
    let config = make_config("sample-model");
    let steps = cargo_sort_residuals(&config, Path::new("/repo"));
    assert_eq!(steps.len(), 5, "cargo_sort_residuals must return exactly 5 steps");
    for step in &steps {
        assert_eq!(step.command, "cargo");
        assert_eq!(step.args[0], "sort");
        assert_eq!(step.args[1], "-n", "all residuals must use -n flag");
    }
}

#[test]
fn cargo_sort_residuals_includes_workspace_wide_step() {
    let config = make_config("sample-model");
    let steps = cargo_sort_residuals(&config, Path::new("/repo"));
    let has_workspace_wide = steps.iter().any(|s| s.args.contains(&"-w".to_owned()));
    assert!(
        has_workspace_wide,
        "cargo_sort_residuals must include a workspace-wide step"
    );
}

#[test]
fn poly_paths_full_regen_is_repo_root() {
    let config = make_config("sample-model");
    let paths = poly_paths(&config, Path::new("/repo"), None, &[Language::Python]);
    assert_eq!(
        paths,
        vec![PathBuf::from("/repo")],
        "full regen formats the whole repo once"
    );
}

#[test]
fn poly_paths_partial_regen_scopes_to_existing_package_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    let py_dir = base.join(config.package_dir(Language::Python));
    std::fs::create_dir_all(&py_dir).unwrap();

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert_eq!(
        paths,
        vec![py_dir],
        "partial regen scopes to the changed language's package dir"
    );
}

#[test]
fn poly_paths_partial_regen_drops_nonexistent_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert!(paths.is_empty(), "nonexistent package dirs are dropped");
}

#[test]
fn poly_pass_formats_generated_python_when_poly_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let py_path = base.join("packages/python/foo.py");
    std::fs::create_dir_all(py_path.parent().unwrap()).unwrap();
    std::fs::write(&py_path, "x=1").unwrap();

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);

    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(
        Language::Python,
        vec![GeneratedFile {
            path: PathBuf::from("packages/python/foo.py"),
            content: "x=1".to_owned(),
            generated_header: false,
        }],
    )];

    format_generated(&files, &config, base, None);

    let formatted = std::fs::read_to_string(&py_path).unwrap();
    if is_tool_available("poly") {
        assert_eq!(
            formatted, "x = 1\n",
            "with poly installed, `poly fmt --fix` must reformat the generated Python file"
        );
    } else {
        assert_eq!(formatted, "x=1", "without poly the file must be left untouched");
    }
}

#[test]
fn install_poly_hooks_is_noop_outside_git_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_poly_hooks(dir.path());
}

#[test]
fn max_poly_fmt_passes_is_bounded() {
    assert_eq!(
        MAX_POLY_FMT_PASSES, 3,
        "the convergence loop must be capped at 3 passes per the full-regen contract"
    );
}

/// Writes a two-crate cargo workspace at `base` with deliberately unsorted
/// `[dependencies]` in both members. `second_crate_suffix` lets callers simulate a
/// crate that historically had no per-language cargo-sort residual (e.g. a `-py`
/// or `-node` crate) to prove the workspace-wide sort covers it too.
fn write_unsorted_workspace(base: &Path, second_crate_suffix: &str) {
    std::fs::write(
        base.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\"crates/pkg-ffi\", \"crates/pkg{second_crate_suffix}\"]\nresolver = \"2\"\n"),
    )
    .unwrap();
    for member in ["pkg-ffi", &format!("pkg{second_crate_suffix}")] {
        let crate_dir = base.join("crates").join(member);
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{member}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\nserde = \"1\"\nanyhow = \"1\"\n"
            ),
        )
        .unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), "pub fn noop() {}\n").unwrap();
    }
}

#[test]
fn run_workspace_cargo_sort_sorts_every_member_regardless_of_language() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    // "-py" simulates a crate suffix (python/pyo3) that has no entry at all in
    // `language_residuals` — the coverage gap this workspace-wide pass replaces.
    write_unsorted_workspace(base, "-py");

    run_workspace_cargo_sort(base);

    let py_toml = std::fs::read_to_string(base.join("crates/pkg-py/Cargo.toml")).unwrap();
    if is_tool_available("cargo-sort") {
        let anyhow_pos = py_toml.find("anyhow").expect("anyhow present");
        let serde_pos = py_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "cargo sort -n -w must sort deps alphabetically even for a crate with no \
             per-language residual, got: {py_toml}"
        );
        let check = std::process::Command::new("cargo")
            .args(["sort", "--check", "-w"])
            .current_dir(base)
            .output()
            .expect("cargo sort --check");
        assert!(
            check.status.success(),
            "workspace must be reported sorted after run_workspace_cargo_sort: {}",
            String::from_utf8_lossy(&check.stderr)
        );
    } else {
        assert!(
            py_toml.contains("serde = \"1\"\nanyhow = \"1\""),
            "without cargo-sort, untouched"
        );
    }
}

#[test]
fn run_workspace_cargo_sort_is_noop_without_root_cargo_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(base.join("marker.txt"), "untouched").unwrap();

    run_workspace_cargo_sort(base);

    assert!(
        !base.join("Cargo.toml").exists(),
        "must not create a Cargo.toml when there was none"
    );
}

#[test]
fn run_cargo_fmt_formats_workspace_rust_files_when_available() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    write_unsorted_workspace(base, "-node");
    let lib_path = base.join("crates/pkg-ffi/src/lib.rs");
    std::fs::write(&lib_path, "pub fn noop( ) {\nlet x=1;\nx;\n}\n").unwrap();

    run_cargo_fmt(base);

    let formatted = std::fs::read_to_string(&lib_path).unwrap();
    if is_tool_available("cargo") && is_tool_available("rustfmt") {
        assert_eq!(
            formatted, "pub fn noop() {\n    let x = 1;\n    x;\n}\n",
            "cargo fmt --all must reformat every workspace member's Rust source"
        );
    } else {
        assert_eq!(
            formatted, "pub fn noop( ) {\nlet x=1;\nx;\n}\n",
            "without cargo/rustfmt, untouched"
        );
    }
}

#[test]
fn run_cargo_fmt_is_noop_without_root_cargo_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let file_path = base.join("orphan.rs");
    std::fs::write(&file_path, "fn noop( ) {}\n").unwrap();

    run_cargo_fmt(base);

    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "fn noop( ) {}\n",
        "a directory with no root Cargo.toml is not a cargo workspace; must be left untouched"
    );
}

#[test]
fn poly_fmt_is_clean_reflects_check_state() {
    if !is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let py_path = base.join("dirty.py");
    std::fs::write(&py_path, "x=1").unwrap();

    assert!(!poly_fmt_is_clean(base), "an unformatted file must report not-clean");

    poly_format(&[base.to_path_buf()], base);

    assert!(
        poly_fmt_is_clean(base),
        "after `poly fmt --fix`, the tree must report clean"
    );
}

/// poly rewrites changed files via atomic rename, which resets the mode to `0644`.
/// `poly_format` must put the executable bit back, or every regen strips it from
/// the generated shebang scripts and poly's own `file-safety` lint rejects the
/// next commit.
#[cfg(unix)]
#[test]
fn poly_format_restores_executable_bit_on_reformatted_shebang_script() {
    use std::os::unix::fs::PermissionsExt as _;

    if !is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let script = base.join("run_tests.php");
    std::fs::write(&script, "#!/usr/bin/env php\n<?php\n$x   =   1;\necho $x;\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    poly_format(&[base.to_path_buf()], base);

    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o755,
        "poly_format must restore the pre-format mode, got {mode:#o}"
    );
}

/// The snapshot must not walk dependency-cache and build directories: on a
/// repo-root pass they dwarf the tree being formatted and hold no generated
/// scripts.
#[cfg(unix)]
#[test]
fn executable_mode_snapshot_skips_dependency_and_build_directories() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let tracked = base.join("scripts/run.sh");
    let ignored = base.join("node_modules/.bin/tool");
    for path in [&tracked, &ignored] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let snapshot = snapshot_executable_modes(&[base.to_path_buf()]);

    let recorded: Vec<&PathBuf> = snapshot.iter().map(|(path, _)| path).collect();
    assert_eq!(recorded, vec![&tracked], "only the non-skipped script may be recorded");
}

#[test]
fn converge_full_regen_formatting_leaves_workspace_sorted_and_poly_fmt_check_clean() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    // "-swift" simulates another crate suffix with no per-language residual today.
    write_unsorted_workspace(base, "-swift");
    std::fs::write(
        base.join("crates/pkg-swift/src/lib.rs"),
        "pub fn noop( ) {\nlet x=1;\nx;\n}\n",
    )
    .unwrap();

    converge_full_regen_formatting(base);

    if is_tool_available("cargo-sort") {
        let swift_toml = std::fs::read_to_string(base.join("crates/pkg-swift/Cargo.toml")).unwrap();
        let anyhow_pos = swift_toml.find("anyhow").expect("anyhow present");
        let serde_pos = swift_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "workspace-wide sort must cover every crate on a full regen"
        );
    }
    if is_tool_available("cargo") && is_tool_available("rustfmt") {
        let formatted = std::fs::read_to_string(base.join("crates/pkg-swift/src/lib.rs")).unwrap();
        assert_eq!(formatted, "pub fn noop() {\n    let x = 1;\n    x;\n}\n");
    }
    if is_tool_available("poly") {
        assert!(
            poly_fmt_is_clean(base),
            "the convergence loop must leave `poly fmt --check` clean, satisfying the \
             zero-drift full-regen contract"
        );
    }
}

#[test]
fn format_generated_full_regen_routes_through_convergence_loop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    write_unsorted_workspace(base, "-py");

    let config = make_config("sample-model");
    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(Language::Ffi, vec![])];

    // A full regen (`only_languages = None`) must go through
    // `converge_full_regen_formatting`, not the single-pass + per-language-residual
    // branch, so it also sorts the "-py" crate that has no residual entry.
    format_generated(&files, &config, base, None);

    if is_tool_available("cargo-sort") {
        let py_toml = std::fs::read_to_string(base.join("crates/pkg-py/Cargo.toml")).unwrap();
        let anyhow_pos = py_toml.find("anyhow").expect("anyhow present");
        let serde_pos = py_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "format_generated(..., None) must workspace-sort crates with no per-language residual"
        );
    }
}
