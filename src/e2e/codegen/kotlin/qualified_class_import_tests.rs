//! Regression coverage for double-qualified imports in the Kotlin e2e test files.
//!
//! `[e2e.call.overrides.kotlin_android] class` may be spelled fully qualified
//! (`dev.sample.bindings.SampleClient`). Two emitters in `test_file.rs` read that one config
//! value and used to disagree about it: the binding-class import split it into an import path
//! plus a simple name, while the trait-bridge import prefixed the binding package onto it
//! unconditionally, producing `dev.sample.bindings.dev.sample.bindings.SampleClient` — an
//! unresolved reference that fails the Kotlin compile. Both now qualify through
//! `naming::qualified_type_path`, so a name that already carries a package is left alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::CallOverride;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

const BINDING_PACKAGE: &str = "dev.sample.bindings";
const SIMPLE_CLASS: &str = "SampleClient";

fn call_fixture() -> Fixture {
    Fixture {
        id: "smoke_basic".to_string(),
        description: "smoke test".to_string(),
        input: serde_json::json!({}),
        ..Fixture::default()
    }
}

/// `class` carries the package already, exactly as a consumer's `alef.toml` may spell it.
fn e2e_config_with_class(class: &str) -> E2eConfig {
    let mut overrides = HashMap::new();
    overrides.insert(
        "kotlin_android".to_string(),
        CallOverride {
            class: Some(class.to_string()),
            ..CallOverride::default()
        },
    );
    E2eConfig {
        call: CallConfig {
            function: "convert".to_string(),
            overrides,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

fn render(class: &str) -> String {
    super::test_file::render_test_file_inner(
        "smoke",
        &[&call_fixture()],
        class,
        "convert",
        BINDING_PACKAGE,
        "result",
        &[],
        None,
        false,
        &e2e_config_with_class(class),
        &HashMap::new(),
        true,
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("kotlin_android smoke test file renders")
}

fn import_lines(rendered: &str) -> Vec<&str> {
    rendered.lines().filter(|line| line.starts_with("import ")).collect()
}

/// The bug: the binding package was concatenated onto a name that already carried it.
#[test]
fn fully_qualified_call_class_is_never_double_qualified() {
    let fully_qualified = format!("{BINDING_PACKAGE}.{SIMPLE_CLASS}");
    let rendered = render(&fully_qualified);
    let doubled = format!("import {BINDING_PACKAGE}.{fully_qualified}");
    assert!(
        !rendered.contains(&doubled),
        "the binding package must not be prefixed onto an already-qualified class; got:\n{rendered}"
    );
    assert!(
        import_lines(&rendered).contains(&format!("import {fully_qualified}").as_str()),
        "the class must still be imported at its own package; got:\n{rendered}"
    );
}

/// The correct import was emitted alongside the bogus one, so an assertion that only checks for
/// the correct spelling would pass over the bug. Every import must be spelled exactly once.
#[test]
fn call_class_is_imported_exactly_once() {
    let fully_qualified = format!("{BINDING_PACKAGE}.{SIMPLE_CLASS}");
    let rendered = render(&fully_qualified);
    let imports = import_lines(&rendered);
    let mut sorted = imports.clone();
    sorted.sort_unstable();
    let before_dedup = sorted.len();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        before_dedup,
        "no import may be emitted twice; got:\n{imports:#?}"
    );
}

/// Control: a BARE `class` still has to be qualified with the binding package, because the test
/// file lives in the `<binding_package>.e2e` child package and Kotlin child packages do not see
/// their parent's symbols. A "fix" that simply stopped qualifying would break this.
#[test]
fn bare_call_class_is_qualified_with_the_binding_package() {
    let rendered = render(SIMPLE_CLASS);
    assert!(
        import_lines(&rendered).contains(&format!("import {BINDING_PACKAGE}.{SIMPLE_CLASS}").as_str()),
        "a bare class name must be qualified with the binding package; got:\n{rendered}"
    );
}
