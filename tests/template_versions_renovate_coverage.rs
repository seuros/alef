//! A version const is only "centralized" if Renovate can actually see it.
//!
//! `src/core/template_versions.rs` is the one file `renovate.json`'s `customManager`
//! scans, so moving a literal there is what keeps a generated manifest's dependency
//! current. But the marker and the const have to line up with a regex, and when they do
//! not the const is silently frozen at whatever it was written as — indistinguishable, from
//! the outside, from a const nobody has needed to bump. That is how `PYO3` sat untracked,
//! and it is the failure the `base64` downgrade surfaced: the regex's `[A-Z_]+` excluded
//! every const name containing a digit.
//!
//! These tests check the apparatus rather than the values: they assert that each marker
//! actually reaches the const beneath it.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// `depName`s the `customManager` deliberately does not reach, because their constants
/// carry a multi-line rationale between the marker and the `pub const`.
///
/// Both are compound `||` constraints spanning several majors on purpose (see their
/// rationales in `template_versions.rs`), and an auto-bump would collapse the span that is
/// the whole point of them. They are listed rather than fixed so that a *new* unreachable
/// marker still fails this test.
const DELIBERATELY_UNREACHABLE: [&str; 2] = ["phpunit/phpunit", "guzzlehttp/guzzle"];

/// Floor on how many markers the manager must reach, well under the real count so routine
/// additions do not churn it, and well over zero so a regex that matched nothing could not
/// pass every set-difference assertion in this file vacuously.
const MINIMUM_TRACKED_DEPENDENCIES: usize = 80;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn custom_manager_pattern() -> String {
    let raw = std::fs::read_to_string(repository_root().join("renovate.json")).expect("read renovate.json");
    let config: serde_json::Value = serde_json::from_str(&raw).expect("renovate.json is valid JSON");
    config["customManagers"][0]["matchStrings"][0]
        .as_str()
        .expect("the customManager declares a matchString")
        .to_string()
}

fn template_versions_source() -> String {
    std::fs::read_to_string(repository_root().join("src/core/template_versions.rs")).expect("read template_versions.rs")
}

/// The `depName`s the `customManager` regex actually captures.
fn tracked_dependency_names(pattern: &str, source: &str) -> BTreeSet<String> {
    let matcher = regex::Regex::new(pattern).expect("the customManager regex compiles");
    matcher
        .captures_iter(source)
        .filter_map(|captures| captures.name("depName").map(|name| name.as_str().to_string()))
        .collect()
}

/// The anti-vacuity control. Every assertion below is a set difference, and an empty
/// tracked set would make the "unreachable" list look complete while nothing was tracked
/// at all. Pin a floor and two concrete members.
#[test]
fn the_custom_manager_regex_matches_the_bulk_of_the_version_table() {
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &template_versions_source());

    assert!(
        tracked.len() > MINIMUM_TRACKED_DEPENDENCIES,
        "the customManager should reach nearly every marked const, reached {}: {tracked:?}",
        tracked.len()
    );
    assert!(tracked.contains("tracing"), "a known-good marker must be tracked");
}

/// The regression. `BASE64` and `PYO3` are the constants whose names carry digits; both
/// were invisible to a `[A-Z_]+` const-name class, so Renovate never proposed a bump and
/// the emitted `base64 = "0.22"` could only ever go stale.
#[test]
fn version_consts_whose_names_contain_digits_are_tracked() {
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &template_versions_source());

    for dependency in ["base64", "pyo3", "pyo3-async-runtimes"] {
        assert!(
            tracked.contains(dependency),
            "`{dependency}` must be reachable by the customManager; a const name with a digit \
             in it is exactly the case the regex used to drop. Tracked: {tracked:?}"
        );
    }
}

/// The general invariant, so a future hoist cannot repeat this quietly: a marker sitting
/// directly above a `pub const` has to reach it.
#[test]
fn every_marker_directly_above_a_const_reaches_that_const() {
    let source = template_versions_source();
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &source);
    let marker = regex::Regex::new(r"^\s*// renovate:.*\bdepName=(\S+)").expect("marker regex compiles");

    let lines: Vec<&str> = source.lines().collect();
    let mut unreachable = Vec::new();
    for pair in lines.windows(2) {
        let Some(captures) = marker.captures(pair[0]) else {
            continue;
        };
        if !pair[1].trim_start().starts_with("pub const") {
            continue;
        }
        let dependency = captures.get(1).expect("depName group").as_str();
        if !tracked.contains(dependency) {
            unreachable.push(format!("{dependency} ({})", pair[1].trim()));
        }
    }

    assert!(
        unreachable.is_empty(),
        "these markers sit directly above a `pub const` but the customManager does not reach them, \
         so the constants are frozen: {unreachable:?}"
    );
}

/// The complement, and the reason the list above is a list rather than a fix: every marker
/// the manager cannot reach must be one that is known and intended. A new one showing up
/// here means a constant went silently un-bumpable.
#[test]
fn the_only_unreachable_markers_are_the_known_compound_constraints() {
    let source = template_versions_source();
    let tracked = tracked_dependency_names(&custom_manager_pattern(), &source);
    // Anchored and applied per line so the module doc's illustrative
    // `// renovate: datasource=... depName=...` is not read as a real marker. ~keep
    let marker = regex::Regex::new(r"^\s*// renovate:.*\bdepName=(\S+)").expect("marker regex compiles");

    let declared: BTreeSet<String> = source
        .lines()
        .filter_map(|line| marker.captures(line))
        .map(|captures| captures[1].to_string())
        .collect();
    let unreachable: BTreeSet<String> = declared.difference(&tracked).cloned().collect();
    let expected: BTreeSet<String> = DELIBERATELY_UNREACHABLE
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    assert_eq!(
        unreachable, expected,
        "the set of markers the customManager cannot reach changed"
    );
}
