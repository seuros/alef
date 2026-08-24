//! Coverage for [`warn_on_ffi_feature_drift`].
//!
//! Split out of the parent test module so both files stay under the
//! `file-modularization` cap; the drift warning is a self-contained concern.

use super::{api_with_gated_functions, resolved_config};
use crate::codegen::cfg::warn_on_ffi_feature_drift;
use crate::core::config::Language;
use tracing_test::traced_test;

/// The exact scenario this repo's FFI feature drift warning was blind to (issue #257): a
/// binding's configured feature set matches `[crates.ffi]`'s configured feature set byte for
/// byte, so a comparison of the two CONFIGURED lists finds nothing. But the FFI crate's
/// EFFECTIVE default set is a strict superset -- `collect_cfg_features` discovers an emitted
/// gate neither list mentions -- so the linked cdylib actually ships more than this binding
/// declares. A drift check that only compares the two configured lists passes silently here;
/// one that compares against the effective set must fire.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_fires_when_effective_set_diverges_even_though_configured_sets_match() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["shared"]
[crates.go]
features = ["shared"]
"#,
    );
    let api = api_with_gated_functions(&[("discovered_gate", Some(r#"feature = "sample-gate""#))]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        logs_contain("coverage gap"),
        "the FFI crate's effective default set includes `sample-gate`, which Go's configured \
         list does not -- this is precisely the drift the two CONFIGURED lists agree on and \
         hide, so the warning must still fire"
    );
    assert!(
        logs_contain(r#"unsatisfied_gates={"feature = \"sample-gate\""}"#),
        "the warning must name the gate it evaluated, not merely a feature name, so a reader can \
         check the claim against the source item"
    );
    assert!(
        !logs_contain("unsafe and can produce glue"),
        "Go's configured set is a subset of the effective set here, not a superset, so this \
         must not be reported as the unsafe host-only direction"
    );
}

/// A binding's configured feature set matching the FFI crate's EFFECTIVE default set exactly
/// (not just its configured list) must not warn -- a check that always fires is as useless as
/// one that never does.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_silent_when_lang_features_equal_effective_set() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["shared"]
[crates.go]
features = ["shared"]
"#,
    );
    let api = api_with_gated_functions(&[("configured_only", None)]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        !logs_contain("coverage gap") && !logs_contain("unsafe and can produce glue"),
        "Go's configured set equals the FFI crate's effective default set, so neither warning \
         must fire"
    );
}

/// The other direction of drift: the binding satisfies a gate the FFI cdylib was NOT built
/// with, so `with_cfg_filtered_deep` keeps glue for a symbol the shipped library never exported.
///
/// The reachable shape of that direction is a declare-only feature: `[crates.ffi].extra_features`
/// names are deliberately excluded from `effective_ffi_default_features` (mutually-exclusive
/// alternatives a consumer selects at build time), so a gate on one of them is not compiled into
/// the cdylib's default build -- while a binding that lists the same name in its own configured
/// features does satisfy the gate and emits the glue. Any OTHER gate name is unreachable here by
/// construction: `effective_ffi_default_features` unions in every feature name an emitted gate
/// references, so the cdylib defaults it on.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_fires_when_the_binding_satisfies_a_gate_the_cdylib_lacks() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["shared"]
extra_features = ["alt-backend"]
[crates.go]
features = ["shared", "alt-backend"]
"#,
    );
    let api = api_with_gated_functions(&[("alt_backend_entry", Some(r#"feature = "alt-backend""#))]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        logs_contain("unsafe and can produce glue"),
        "`alt-backend` is declare-only on the FFI side, so the cdylib's default build omits \
         `alt_backend_entry`; Go configures the feature and keeps the glue -- the unsafe \
         direction must fire"
    );
    assert!(
        !logs_contain("coverage gap"),
        "no gate is satisfied by the cdylib and unsatisfied by Go here, so the safe \
         coverage-gap warning must not also fire"
    );
}

/// The regression this module exists for: with the conventional umbrella `features = ["full"]`
/// configured on BOTH sides, every gate is satisfied on both sides, `with_cfg_filtered_deep`
/// drops nothing, and there is no coverage gap of any kind to report.
///
/// A set difference over literal feature names cannot see that. `full` is a hard-coded universal
/// satisfier inside `cfg_feature_satisfied`, so `{"full"}` satisfies `feature = "alpha"` while
/// containing no name in common with it -- and the old implementation duly reported every
/// cfg-discovered name as "safely omitted by with_cfg_filtered_deep", on every regeneration of
/// every such project, with text that was simply untrue. Warnings that are routinely false get
/// filtered out by the reader, taking the true ones with them.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_reports_nothing_when_full_is_configured_on_both_sides() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["full"]
[crates.go]
features = ["full"]
"#,
    );
    let api = api_with_gated_functions(&[
        ("ungated", None),
        ("alpha_entry", Some(r#"feature = "alpha""#)),
        ("beta_entry", Some(r#"feature = "beta""#)),
    ]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        !logs_contain("coverage gap"),
        "`full` satisfies every feature gate for both the binding filter and the cdylib, so no \
         item is omitted from the Go surface and no coverage gap exists to report"
    );
    assert!(
        !logs_contain("unsafe and can produce glue"),
        "both sides satisfy every gate, so nothing is kept that the cdylib lacks either"
    );
}

/// The same false-positive shape without `full`: one configured feature satisfying an `any(...)`
/// gate keeps the item, so differencing the gate's feature NAMES against the configured list
/// invents a gap that the filter never opened.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_reports_nothing_when_an_any_gate_is_already_satisfied() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["alpha"]
[crates.go]
features = ["alpha"]
"#,
    );
    let api = api_with_gated_functions(&[("either_entry", Some(r#"any(feature = "alpha", feature = "beta")"#))]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        !logs_contain("coverage gap"),
        "`alpha` alone satisfies `any(alpha, beta)`, so `either_entry` survives the Go filter \
         exactly as it survives into the cdylib -- `beta` is not a missing feature"
    );
}

/// The case the warning exists for: a configured feature list that satisfies no gate in the
/// surface at all. Every gated item is dropped from this binding while the cdylib -- whose
/// effective defaults union in every discovered gate name -- exports all of them, so the Go
/// surface is silently smaller than the artifact it links against.
#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_reports_a_configured_list_that_satisfies_no_gate() {
    let config = resolved_config(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["full"]
[crates.go]
features = ["unrelated"]
"#,
    );
    let api = api_with_gated_functions(&[
        ("ungated", None),
        ("alpha_entry", Some(r#"feature = "alpha""#)),
        ("beta_entry", Some(r#"feature = "beta""#)),
    ]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);

    assert!(
        logs_contain("coverage gap"),
        "`unrelated` satisfies neither gate, so both gated functions vanish from the Go surface \
         while the cdylib exports them -- the underexposure must be reported"
    );
    assert!(
        logs_contain(r#"missing_features={"alpha", "beta"}"#),
        "the remedy must name the features to add, and both dropped gates must be accounted for"
    );
}

/// Shared fixture data for the three aggregate-expansion shapes below: how a core-crate
/// aggregate interacts with the FFI cdylib's effective defaults once BOTH sides of the drift
/// check expand configured aggregates the same way `backends::go`/`java`/`csharp`/`kotlin`/
/// `zig`/`wasm` now resolve their own `enabled_features` (see `fix(backends): expand configured
/// aggregate features before cfg filtering`). `core_manifest` is written to a temp `sample-core`
/// crate on disk so `expand_configured_features` has a real `[features]` table to walk; a case
/// with no aggregate involved (case c) still gets one, to prove expansion is a no-op for a plain
/// feature rather than merely untested. ~keep
struct AggregateDriftCase {
    name: &'static str,
    core_manifest: &'static str,
    workspace_toml: &'static str,
    gate: &'static str,
    expect_coverage_gap: bool,
    expect_unsafe: bool,
}

const AGGREGATE_DRIFT_CASES: &[AggregateDriftCase] = &[
    // (a) The aggregate Go configures (`mobile-target`) is the SAME aggregate the FFI side
    // configures, so expanding both sides through the core manifest lands on the identical set
    // `{"mobile-target", "alt-backend"}`. Every gate `alt-backend` touches is satisfied on both
    // sides -- this is the negative case: a fully-covered aggregate must stay silent. Before the
    // fix, `binding_enabled` was never expanded, so Go's literal `{"mobile-target"}` did not
    // satisfy `feature = "alt-backend"` while the (already-expanded) cdylib side did -- a false
    // "coverage gap" on every member, on every regeneration.
    AggregateDriftCase {
        name: "fully_covered_aggregate",
        core_manifest: "[package]\nname = \"sample-core\"\n\n[features]\ndefault = []\n\
                         mobile-target = [\"alt-backend\"]\nalt-backend = []\n",
        workspace_toml: r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["mobile-target"]
extra_features = ["alt-backend"]
[crates.go]
features = ["mobile-target"]
"#,
        gate: r#"feature = "alt-backend""#,
        expect_coverage_gap: false,
        expect_unsafe: false,
    },
    // (b) Go configures `mobile-target`, but the FFI side configures an unrelated `shared`
    // feature and only declares `alt-backend` as `extra_features` (declare-only, never
    // defaulted). No FFI-configured aggregate reaches `alt-backend`, so the cdylib's effective
    // set stays `{"shared"}` even after expansion -- the member genuinely never ships in the
    // linked library. Go's real (also-expanded) filter keeps `alt_backend_entry` regardless, so
    // this is real, actionable UNSAFE drift. Before the fix this gate was invisible to the
    // warning: literal `{"mobile-target"}` never matched `"alt-backend"`, so neither branch
    // fired and the drift went unreported.
    AggregateDriftCase {
        name: "aggregate_member_outside_ffi_reach",
        core_manifest: "[package]\nname = \"sample-core\"\n\n[features]\ndefault = []\n\
                         mobile-target = [\"alt-backend\"]\nalt-backend = []\n",
        workspace_toml: r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-core"
sources = []
[crates.ffi]
features = ["shared"]
extra_features = ["alt-backend"]
[crates.go]
features = ["mobile-target"]
"#,
        gate: r#"feature = "alt-backend""#,
        expect_coverage_gap: false,
        expect_unsafe: true,
    },
    // A third row -- a genuinely host-only literal feature configured with no aggregate
    // relationship at all -- was deliberately removed here. `expand_configured_features` is a
    // no-op for a name that is not an aggregate key (it only ever ADDS resolved members to a
    // requested name, never renames or drops one), so that shape is mathematically incapable of
    // distinguishing this fix from its absence: reverting the `expand_configured_features` call
    // in `warn_on_ffi_feature_drift` cannot turn such a case red. A test that cannot fail when
    // the code it guards is wrong is not verifying anything; see
    // `warn_on_ffi_feature_drift_fires_when_the_binding_satisfies_a_gate_the_cdylib_lacks` above
    // for the (still-valid, pre-existing) coverage of a plain declare-only feature -- that test
    // predates aggregate expansion entirely and needs no aggregate-specific counterpart. ~keep
];

/// Writes `case.core_manifest` to a temp `sample-core` crate, resolves `case.workspace_toml`
/// against it, builds a single-function API surface gated on `case.gate`, and runs the drift
/// check. Returns the tempdir so it outlives the caller's assertions (unused after the call, but
/// dropping it early would delete the manifest `expand_configured_features` reads mid-call on
/// some platforms).
fn run_aggregate_drift_case(case: &AggregateDriftCase) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let core_dir = dir.path().join("crates").join("sample-core");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(core_dir.join("Cargo.toml"), case.core_manifest).expect("write core Cargo.toml");

    let mut config = resolved_config(case.workspace_toml);
    config.workspace_root = Some(dir.path().to_path_buf());
    config.sources = vec![std::path::PathBuf::from("crates/sample-core/src/lib.rs")];
    let api = api_with_gated_functions(&[("gated_entry", Some(case.gate))]);

    warn_on_ffi_feature_drift(&api, &config, Language::Go);
    dir
}

#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_case_a_fully_covered_aggregate_stays_silent() {
    let case = &AGGREGATE_DRIFT_CASES[0];
    assert_eq!(case.name, "fully_covered_aggregate");
    let _dir = run_aggregate_drift_case(case);

    assert_eq!(
        logs_contain("coverage gap"),
        case.expect_coverage_gap,
        "case `{}`: coverage-gap direction should not fire -- both sides expand `mobile-target` \
         to the same `{{mobile-target, alt-backend}}` set",
        case.name
    );
    assert_eq!(
        logs_contain("unsafe and can produce glue"),
        case.expect_unsafe,
        "case `{}`: unsafe direction should not fire either -- nothing is kept that the cdylib \
         lacks",
        case.name
    );
}

#[traced_test]
#[test]
fn warn_on_ffi_feature_drift_case_b_aggregate_member_outside_ffi_reach_is_unsafe() {
    let case = &AGGREGATE_DRIFT_CASES[1];
    assert_eq!(case.name, "aggregate_member_outside_ffi_reach");
    let _dir = run_aggregate_drift_case(case);

    assert_eq!(
        logs_contain("unsafe and can produce glue"),
        case.expect_unsafe,
        "case `{}`: Go's expanded filter keeps `alt_backend_entry`, but no FFI-configured \
         aggregate reaches `alt-backend`, so the cdylib never ships it -- this must fire unsafe",
        case.name
    );
    assert_eq!(
        logs_contain("coverage gap"),
        case.expect_coverage_gap,
        "case `{}`: the same gate cannot be both a coverage gap and unsafe drift",
        case.name
    );
}
