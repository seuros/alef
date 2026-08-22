//! Regression for the diagnostic-ordering defect behind a real-world liter-llm failure: a
//! Dart FRB facade (`packages/dart/rust/src/lib.rs`) self-marks and so always regenerates
//! freely, but its sibling manifest (`packages/dart/rust/Cargo.toml`) predates alef's
//! marker-stamping convention for `.toml` in an already-committed consumer tree. When a
//! *newly* `#[cfg(feature = "...")]`-gated function is added to the facade,
//! `collect_cfg_features` correctly wants to add a forwarding `[features]` entry to that
//! Cargo.toml -- but the ownership guard refuses the write because the pre-existing file
//! carries no `alef:hash:` marker. The facade gains the gated function; the manifest that
//! would activate its feature does not; a real `flutter_rust_bridge_codegen` run (or, as
//! here, a stale bridge left on disk because `skip_frb` omitted the `RunCommand` step) then
//! legitimately lacks that function, and `VerifyFrbBridgeCoverage` (alef #135) fails the
//! build -- correctly. The bug this file pins is that the one diagnostic explaining *why*
//! (`pipeline::report_refused_writes`'s "run `alef adopt <path>`") was only ever printed at
//! the very end of `handle`'s "All" arm, unreachable once the post-build `?` had already
//! returned. See the matching `~keep` comment on `complete_generated_artifacts`'s call site
//! in `all_commands.rs`. ~keep

use super::handle;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::CwdGuard;
use tracing_test::traced_test;

const WIDGET_LIB_SOURCE: &str = r#"
pub fn always_present(value: i64) -> i64 {
    value
}

/// Counts widgets in a collection. Only available when the `widgets` feature is enabled.
#[cfg(feature = "widgets")]
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(collection.len() as i64)
}
"#;

const WIDGET_LIB_CARGO_TOML: &str = "[package]\nname = \"widget-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

const WIDGET_LIB_ALEF_TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "widget-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.dart]
package_name = "widget_lib"
skip_frb = true
"#;

/// A plausible pre-existing FRB facade manifest with no `[features]` table and no
/// `alef:hash:` marker -- exactly the shape `crates/*-ffi/Cargo.toml` and
/// `packages/dart/rust/Cargo.toml` were found in on real consumer trees (see
/// `stamp_for_adoption`'s doc in `cli::pipeline::generate::write`), because both predate
/// alef ever stamping `.toml` output. Deliberately missing the `widgets` forwarding entry
/// `collect_cfg_features` would add, so the regenerated content differs from this and a
/// real write is attempted (and refused) rather than short-circuited as "unchanged".
const STALE_DART_RUST_CARGO_TOML: &str = "\
[package]
name = \"widget-lib-dart\"
version = \"0.1.0\"
edition = \"2024\"

[lib]
crate-type = [\"cdylib\", \"staticlib\"]

[dependencies]
flutter_rust_bridge = \"=2.0.0\"
";

/// A bridge as `flutter_rust_bridge_codegen` would have left it before `count_widgets` was
/// added to the facade -- covers `always_present` only.
const STALE_BRIDGE_DART: &str = "\
Future<int> alwaysPresent({required int value}) =>
    RustLib.instance.api.crateAlwaysPresentAlwaysPresent(value: value);
";

fn write_widget_lib_fixture(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), WIDGET_LIB_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), WIDGET_LIB_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), WIDGET_LIB_ALEF_TOML).expect("write fixture alef.toml");

    let rust_dir = root.join("packages/dart/rust");
    std::fs::create_dir_all(&rust_dir).expect("create dart rust crate directory");
    std::fs::write(rust_dir.join("Cargo.toml"), STALE_DART_RUST_CARGO_TOML).expect("seed stale dart Cargo.toml");

    let bridge_dir = root.join("packages/dart/lib/src/widget_lib_bridge_generated");
    std::fs::create_dir_all(&bridge_dir).expect("create bridge_generated directory");
    std::fs::write(bridge_dir.join("lib.dart"), STALE_BRIDGE_DART).expect("seed stale bridge lib.dart");
}

fn widget_lib_all_command() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
    }
}

/// Drives the real `handle` entry point (not a lower-level unit call) because the defect
/// lives in `all_commands.rs`'s own control flow -- specifically, which side of a `?` a
/// call to `pipeline::report_refused_writes` sits on. A unit test on `collect_cfg_features`
/// or `missing_bridge_functions` alone (both already covered, and both already correct)
/// cannot observe this: the divergence they each individually compute is fine in isolation,
/// and only surfaces as a *silently unexplained* build failure once `handle` wires a refused
/// write together with a post-build failure it caused. ~keep
#[test]
#[traced_test]
fn all_surfaces_the_refusal_report_before_a_post_build_coverage_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_widget_lib_fixture(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let result = handle(widget_lib_all_command(), &context);
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!(
            "a stale FRB bridge missing a cfg-gated facade function must still fail the run -- \
             VerifyFrbBridgeCoverage's detection (alef #135) must not be weakened"
        ),
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("count_widgets") && message.contains("missing 1 function"),
        "the propagated error must still be VerifyFrbBridgeCoverage's own diagnostic naming the \
         cfg-gated function: {message}"
    );

    assert!(
        logs_contain("alef adopt"),
        "the ownership guard refused packages/dart/rust/Cargo.toml (it predates alef's marker \
         convention for .toml), which is *why* the bridge coverage check failed -- \
         pipeline::report_refused_writes must run before the post-build error propagates, or an \
         operator only ever sees VerifyFrbBridgeCoverage's misleading \"install/enable \
         flutter_rust_bridge_codegen\" text and never learns the actual fix is `alef adopt`"
    );
    assert!(
        logs_contain("Cargo.toml"),
        "the surfaced refusal report must name the frozen file itself, not just announce that \
         something was refused"
    );
}
