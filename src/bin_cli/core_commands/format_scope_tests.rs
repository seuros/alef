//! What a partial regen (`alef generate`) actually hands to the formatter, measured through
//! the real command rather than through `poly_paths`' arithmetic.
//!
//! `format_generated(..., Some(changed_languages))` used to format `package_dir(lang)` alone,
//! while `finalize_hashes` stamped -- and `generate_sweep_roots` swept -- both that and
//! `output_for(lang)`. For most languages the two coincide or nest, so nothing showed. For
//! Python they do not: the wheel is `packages/python`, the PyO3 glue crate is generated into
//! `crates/<name>-py/src`, and only the first was ever formatted. `alef generate --lang python`
//! therefore stamped Rust no formatter had canonicalised, and the next whole-tree pass made
//! that stamp stale.
//!
//! Elixir deliberately does NOT gain this property: `POLY_ELIXIR_EXCLUDE_GLOBS` keeps poly away
//! from `.ex`/`.exs` entirely, because poly's Elixir engine misindents mix-canonical output and
//! then reports its own output `--check`-clean. That exclusion is correct, and this fixture
//! stays on Python so it never encodes an argument against it. ~keep

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use std::path::Path;

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#;

/// Deliberately unformatted Rust, planted in the glue crate's source directory before the run.
///
/// The scope question is "was this directory handed to poly at all", and the emitter's own
/// output cannot answer it: for a fixture this small the PyO3 glue crate happens to come out
/// already rustfmt-canonical, so asserting on `lib.rs` passes with or without the fix --
/// measured, not assumed. poly formats the directories it is given, not a per-file allowlist,
/// so a file that only a widened scope can reach is what actually discriminates. ~keep
const UNFORMATTED_PROBE: &str = "pub fn probe()->i32{let x=1;x}\n";

fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// `alef generate` must format the generated PyO3 glue crate, not only the wheel package.
#[test]
fn generate_formats_the_rust_glue_crate_it_stamps() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let glue_src = root.join("crates/test-lib-py/src");
    std::fs::create_dir_all(&glue_src).expect("glue crate src directory");
    let probe = glue_src.join("scope_probe.rs");
    std::fs::write(&probe, UNFORMATTED_PROBE).expect("plant the scope probe");

    // Holds `test_support::CWD_LOCK` for the whole test: `sources_hash` and
    // `read_alef_toml_bytes` resolve against the process cwd, so a test that enters a fixture
    // directory without it passes alone and fails in the full suite. ~keep
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            strict: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");

    assert!(
        root.join("crates/test-lib-py/src/lib.rs").exists(),
        "sanity: the run must actually have generated the PyO3 glue crate, or the scope \
         assertion below proves nothing"
    );
    let formatted = std::fs::read_to_string(root.join("packages/python/test_lib/__init__.py"))
        .expect("sanity: the wheel package must also have been generated");
    assert!(
        !formatted.is_empty(),
        "sanity: the wheel package directory -- the one scope that always worked -- must be populated"
    );

    let probe_after = std::fs::read_to_string(&probe).expect("read the scope probe back");
    assert_ne!(
        probe_after, UNFORMATTED_PROBE,
        "the generated PyO3 glue crate's source directory was never handed to the formatter. \
         `finalize_hashes` stamps that directory and `generate_sweep_roots` sweeps it, so a \
         narrower formatting scope means `alef generate` emits non-canonical Rust and stamps a \
         hash over it -- and the next whole-tree format pass makes every one of those stamps stale"
    );
}
