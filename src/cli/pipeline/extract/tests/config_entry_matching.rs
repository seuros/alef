//! How a configured `[crates.include]` / `[crates.exclude]` entry is matched against the
//! extracted surface, split out of [`super`] (`tests.rs`) under the 1,000-line file-size cap.
//! Uses `super`'s private fixture helpers (`make_typedef`, `make_funcdef`, `surface_with`).

use super::*;

/// Plain (no-`::`) entries match by short name only.
#[test]
fn is_type_excluded_plain_entry_matches_by_name() {
    let exclude = vec!["OutputFormat".to_string()];

    assert!(
        is_type_excluded("OutputFormat", "sample_crate::types::OutputFormat", &exclude),
        "plain entry must match when name matches"
    );

    assert!(
        !is_type_excluded("SomethingElse", "sample_crate::types::SomethingElse", &exclude),
        "plain entry must not match when name differs"
    );
}

/// Fully-qualified entries match only the specific rust_path, not any type
/// that merely shares the same short name.
///
/// Regression: sample_core::core::config::formats::OutputFormat must be excluded
/// while sample_core::types::OutputFormat is retained.
#[test]
fn is_type_excluded_qualified_entry_matches_rust_path_not_name() {
    let exclude = vec!["sample_crate::core::config::formats::OutputFormat".to_string()];

    assert!(
        is_type_excluded(
            "OutputFormat",
            "sample_crate::core::config::formats::OutputFormat",
            &exclude
        ),
        "qualified entry must match the exact rust_path"
    );

    assert!(
        !is_type_excluded("OutputFormat", "sample_crate::types::OutputFormat", &exclude),
        "qualified entry must NOT match a different rust_path with the same short name"
    );
}

/// Hyphens in rust_path are normalised to underscores before comparison, matching
/// the convention used throughout alef's path mapping layer.
#[test]
fn is_type_excluded_normalises_hyphens_in_rust_path() {
    let exclude = vec!["my_crate::some_module::Foo".to_string()];

    assert!(
        is_type_excluded("Foo", "my-crate::some_module::Foo", &exclude),
        "hyphens in rust_path should be normalised to underscores"
    );
}

/// `include` is an allowlist, so an entry matching nothing does not fail open — it drops every
/// type and enum from the surface. Before the fix `include.types = ["Kpet"]` returned an empty
/// type list and `alef build` generated empty bindings and exited 0.
#[test]
fn apply_filters_rejects_include_types_entry_that_matches_nothing() {
    let surface = surface_with(
        vec![make_typedef("Kept"), make_typedef("Other")],
        vec![make_funcdef("do_it", TypeRef::Unit, vec![])],
    );
    let mut config = ResolvedCrateConfig::default();
    config.include.types = vec!["Kpet".to_string()];

    let error = apply_filters(surface, &config).expect_err("an include.types typo must not silently pass");

    let message = error.to_string();
    assert!(
        message.contains("`Kpet`") && message.contains("[crates.include].types"),
        "error must name the unmatched entry and the config key it came from, got: {message}"
    );
}

/// A partially-valid include list must fail too: the good entries would otherwise mask the typo,
/// which silently narrows the binding instead of emptying it.
#[test]
fn apply_filters_rejects_include_types_when_only_some_entries_match() {
    let surface = surface_with(vec![make_typedef("Kept"), make_typedef("Other")], vec![]);
    let mut config = ResolvedCrateConfig::default();
    config.include.types = vec!["Kept".to_string(), "Gone".to_string()];

    let error = apply_filters(surface, &config).expect_err("an unmatched entry must fail even alongside matched ones");

    let message = error.to_string();
    assert!(
        message.contains("`Gone`") && !message.contains("`Kept`"),
        "error must name only the unmatched entry, got: {message}"
    );
}

/// `exclude.types` accepts a qualified `crate::path::Type` entry, so `include.types` must resolve
/// the same spelling to the same type instead of comparing it against the short name and matching
/// nothing.
#[test]
fn apply_filters_include_types_accepts_qualified_path_entry() {
    let surface = surface_with(vec![make_typedef("Kept"), make_typedef("Other")], vec![]);
    let mut config = ResolvedCrateConfig::default();
    config.include.types = vec!["my_crate::Kept".to_string()];

    let result = apply_filters(surface, &config).expect("a qualified include entry must resolve");

    let names: Vec<&str> = result.types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Kept"],
        "qualified include entry must retain exactly the type it names"
    );
}

/// A type declared only in `[crates.opaque_types]` is injected into the surface after filtering
/// runs, so naming it in `include.types` must not be treated as an unmatched entry.
#[test]
fn apply_filters_include_types_accepts_declared_opaque_type_name() {
    let surface = surface_with(vec![make_typedef("Kept")], vec![]);
    let mut config = ResolvedCrateConfig::default();
    config.include.types = vec!["Kept".to_string(), "Handle".to_string()];
    config
        .opaque_types
        .insert("Handle".to_string(), "my_crate::Handle".to_string());

    apply_filters(surface, &config).expect("a declared opaque type is a valid include entry");
}

/// The `include.functions` allowlist has the same failure mode as `include.types`.
#[test]
fn apply_filters_rejects_include_functions_entry_that_matches_nothing() {
    let surface = surface_with(vec![], vec![make_funcdef("do_it", TypeRef::Unit, vec![])]);
    let mut config = ResolvedCrateConfig::default();
    config.include.functions = vec!["do_ti".to_string()];

    let error = apply_filters(surface, &config).expect_err("an include.functions typo must not silently pass");

    let message = error.to_string();
    assert!(
        message.contains("`do_ti`") && message.contains("[crates.include].functions"),
        "error must name the unmatched entry and the config key it came from, got: {message}"
    );
}

/// `exclude.fields` matched a two-segment `crate::Type` entry against any path in that crate
/// ending in `::Type`, while `exclude.types` demanded an exact path — so the same spelling
/// excluded a field but not the type owning it. One matcher now answers both.
#[test]
fn is_type_excluded_accepts_the_same_crate_qualified_shorthand_as_exclude_fields() {
    let exclude = vec!["my_crate::Foo".to_string()];

    assert!(
        is_type_excluded("Foo", "my_crate::inner::Foo", &exclude),
        "`crate::Type` shorthand must exclude the type the identical exclude.fields entry matches"
    );
    assert!(
        !is_type_excluded("Foo", "other_crate::inner::Foo", &exclude),
        "`crate::Type` shorthand must not reach into a different crate"
    );
    assert!(
        !is_type_excluded("Bar", "my_crate::inner::Bar", &exclude),
        "`crate::Type` shorthand must not match a different type name"
    );
}

/// An exclusion is only observable through what it removes, so a typo'd entry excluded nothing
/// and said nothing. Every `exclude` list must report its unmatched entries.
#[test]
fn unmatched_exclude_entries_reports_every_list_that_names_nothing() {
    let mut kept = make_typedef("Kept");
    kept.methods.push(crate::core::ir::MethodDef {
        name: "run".to_string(),
        ..Default::default()
    });
    let surface = surface_with(vec![kept], vec![make_funcdef("do_it", TypeRef::Unit, vec![])]);

    let exclude = crate::core::config::ExcludeConfig {
        types: vec!["Kpet".to_string()],
        functions: vec!["do_ti".to_string()],
        methods: vec!["Kept.walk".to_string()],
        fields: vec![],
    };

    let mut unmatched = unmatched_exclude_entries(&surface, &exclude);
    unmatched.sort();

    assert_eq!(
        unmatched,
        vec![
            ("functions", "do_ti".to_string()),
            ("methods", "Kept.walk".to_string()),
            ("types", "Kpet".to_string()),
        ],
        "each exclude list must surface the entry that matched nothing"
    );
}

/// The counterpart: entries that do match must stay silent, otherwise the diagnostic is noise
/// every consumer learns to ignore.
#[test]
fn unmatched_exclude_entries_stays_silent_when_every_entry_matches() {
    let mut kept = make_typedef("Kept");
    kept.methods.push(crate::core::ir::MethodDef {
        name: "run".to_string(),
        ..Default::default()
    });
    let surface = surface_with(vec![kept], vec![make_funcdef("do_it", TypeRef::Unit, vec![])]);

    let exclude = crate::core::config::ExcludeConfig {
        types: vec!["Kept".to_string()],
        functions: vec!["do_it".to_string()],
        methods: vec!["Kept.run".to_string()],
        fields: vec![],
    };

    assert!(
        unmatched_exclude_entries(&surface, &exclude).is_empty(),
        "matched exclude entries must not be reported"
    );
}
