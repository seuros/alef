//! Whole-package pipeline output invariants for generated Go bindings, split out of
//! `tests.rs`: alef marker/hash stamping (`finalize_hashes`) and cgo feature-macro
//! guard consistency between called symbols and `#cgo CFLAGS` defines.

use super::super::*;
use super::{make_config, resolved_one};

/// Replay of the write pipeline's stamping contract for a single emitted file.
///
/// `core_commands` only inserts a generated path into the set it hands
/// `finalize_hashes` when [`GeneratedFile::carries_alef_marker`] holds, and
/// `write_files_report` refuses to overwrite an existing markable file whose content
/// carries no marker. A `.go` file that fails this therefore gets neither provenance
/// nor future regeneration, silently. ~keep
fn assert_pipeline_stamps(file: &crate::core::backend::GeneratedFile) {
    use crate::core::hash;

    let path = file.path.display().to_string();
    assert!(
        file.carries_alef_marker(),
        "{path}: emitted without an alef marker and without `generated_header`, so the \
         path never reaches `finalize_hashes` and the write guard will refuse to rewrite it"
    );

    let on_disk = if hash::content_has_alef_marker(&file.content) {
        file.content.clone()
    } else {
        format!("{}\n{}", hash::header(hash::CommentStyle::DoubleSlash), file.content)
    };
    assert!(
        hash::content_has_alef_marker(&on_disk),
        "{path}: the bytes the writer puts on disk must carry the marker `finalize_hashes` \
         searches for, got:\n{on_disk}"
    );

    let inputs_hash = hash::compute_inputs_hash("sources", b"[workspace]\n");
    let body = hash::strip_hash_line(&on_disk);
    let stamped = hash::inject_hash_line(&body, &hash::compute_file_hash(&inputs_hash, &body));
    assert_eq!(
        hash::extract_hash(&stamped),
        Some(hash::compute_file_hash(&inputs_hash, &hash::strip_hash_line(&stamped))),
        "{path}: the injected alef:hash: line must re-verify the way `alef verify` derives it"
    );
}

#[test]
fn every_emitted_go_file_carries_a_hash_line_after_finalize() {
    use crate::core::ir::ApiSurface;

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.2.3".to_string(),
        ..ApiSurface::default()
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();

    let named = |name: &str| {
        files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with(name))
            .unwrap_or_else(|| {
                panic!(
                    "{name} must be emitted; got {:?}",
                    files.iter().map(|file| &file.path).collect::<Vec<_>>()
                )
            })
    };

    // Positive control: assert each file actually holds its generated payload, so the
    // stamping assertions below cannot pass over empty or missing output. ~keep
    assert!(
        named("binding.go").content.contains("package testlib"),
        "binding.go must hold real bindings, got:\n{}",
        named("binding.go").content
    );
    assert!(
        named("native_setup.go")
            .content
            .contains("RequireNativeSetup_1_2_3 = \"1.2.3\""),
        "native_setup.go must hold the version sentinel that changes on every release, got:\n{}",
        named("native_setup.go").content
    );
    assert!(
        named("embed_ffi.go").content.contains("//go:embed"),
        "embed_ffi.go must hold its embed directive, got:\n{}",
        named("embed_ffi.go").content
    );
    assert!(
        named("generate.go").content.contains("//go:generate"),
        "generate.go must hold its generate directive, got:\n{}",
        named("generate.go").content
    );
    assert!(
        named("cmd/setup/main.go").content.contains("func main()"),
        "cmd/setup/main.go must hold the setup tool, got:\n{}",
        named("cmd/setup/main.go").content
    );

    for file in &files {
        assert_pipeline_stamps(file);
    }
}

/// The whole-package invariant behind the cgo feature-macro defect: every FFI symbol the
/// generated Go sources call must still be *declared* after cgo runs the C preprocessor over the
/// header. cbindgen wraps each `#[cfg(feature = "x")]` export in `#if defined(PREFIX_FEATURE_X)`,
/// so a call site whose guard macro is not in the package's `#cgo CFLAGS` compiles to
/// `could not determine what C.<symbol> refers to`.
///
/// Both sides are derived, not pinned: the called set is read out of the emitted Go, the defined
/// set out of the emitted `#cgo` directives, and the required macro per symbol out of the IR gate
/// plus `c_consumer`'s symbol spelling — the same helper the FFI backend names its exports with.
/// A new gated export, a renamed macro, or a dropped `-D` all fail here.
///
/// Scope it cannot check: it models cgo's package-wide merge of `#cgo` directives (only
/// `binding.go` carries the `-D` line, as `service_file_preamble.jinja` already assumes for
/// `-I`), it only walks free functions, and it cannot see a feature the *library* was built
/// without — that is `warn_on_ffi_feature_drift`'s and the link step's job. ~keep
#[test]
fn every_gated_symbol_the_go_package_calls_has_its_guard_macro_defined() {
    use crate::codegen::c_consumer;
    use crate::core::ir::{ApiSurface, FunctionDef};
    use std::collections::BTreeSet;

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
features = ["download", "document-render"]
[crates.ffi]
prefix = "test"
extra_features = ["wasm-http"]
[crates.go]
module = "github.com/test/test-lib"
"#,
    );
    let gates: Vec<(&str, Option<&str>)> = vec![
        ("ping", None),
        ("download", Some(r#"feature = "download""#)),
        (
            "render_document",
            Some(r#"all(feature = "document-render", feature = "download")"#),
        ),
        ("fetch_wasm", Some(r#"feature = "wasm-http""#)),
    ];
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: gates
            .iter()
            .map(|(name, cfg)| FunctionDef {
                name: (*name).to_string(),
                rust_path: format!("test_lib::{name}"),
                cfg: cfg.map(str::to_string),
                ..FunctionDef::default()
            })
            .collect(),
        ..ApiSurface::default()
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let go_sources: Vec<&str> = files
        .iter()
        .filter(|file| {
            let path = file.path.to_string_lossy().into_owned();
            // `cmd/setup` is a separate `package main`; cgo does not merge its directives into
            // the binding package, so it must not count towards either set. ~keep
            path.ends_with(".go") && !path.contains("/cmd/")
        })
        .map(|file| file.content.as_str())
        .collect();
    assert!(!go_sources.is_empty(), "control: the Go backend must emit .go sources");

    let called: HashSet<String> = go_sources
        .iter()
        .flat_map(|source| {
            source.split("C.test_").skip(1).map(|tail| {
                let end = tail
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(tail.len());
                format!("test_{}", &tail[..end])
            })
        })
        .collect();

    let defined: HashSet<String> = go_sources
        .iter()
        .flat_map(|source| source.lines())
        .filter(|line| line.contains("#cgo") && line.contains("CFLAGS:"))
        .flat_map(str::split_whitespace)
        .filter_map(|token| token.strip_prefix("-D"))
        .map(|token| token.split('=').next().unwrap_or(token).to_string())
        .collect();

    let gated_symbol = c_consumer::free_function_symbol("test", "download");
    assert!(
        called.contains(&gated_symbol),
        "control: the Go package must call the gated export, otherwise this test is vacuous; called: {called:?}"
    );
    assert!(
        called.contains(&c_consumer::free_function_symbol("test", "ping")),
        "control: the Go package must also call the ungated export; called: {called:?}"
    );
    let declare_only = c_consumer::free_function_symbol("test", "fetch_wasm");
    assert!(
        !called.contains(&declare_only),
        "`extra_features` stay off, so the glue for {declare_only} must not be emitted at all"
    );
    assert!(
        !defined.contains("TEST_FEATURE_WASM_HTTP"),
        "a genuinely-disabled feature must stay genuinely invisible; defined: {defined:?}"
    );

    for func in &api.functions {
        let Some(cfg) = func.cfg.as_deref() else { continue };
        let symbol = c_consumer::free_function_symbol("test", &func.name);
        if !called.contains(&symbol) {
            continue;
        }
        let mut features = BTreeSet::new();
        crate::codegen::cfg::collect_cfg_feature_names(cfg, &mut features);
        for feature in features {
            let macro_name = crate::backends::go::cgo_features::guard_macro_name("test", &feature);
            assert!(
                defined.contains(&macro_name),
                "the Go package calls {symbol}, whose header declaration cbindgen guards with \
                 {macro_name}, but no #cgo CFLAGS defines it — cgo deletes the declaration. \
                 defined: {defined:?}"
            );
        }
    }
}
