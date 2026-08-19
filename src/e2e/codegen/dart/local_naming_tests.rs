//! Dart privacy is library-scoped, so a leading underscore on a *local* carries no meaning
//! and only trips `no_leading_underscores_for_local_identifiers`. Module-scope helpers such
//! as `_fixtureUrl`/`_fixtureUrls` are the opposite case — there the underscore is real
//! library privacy and must stay. These tests pin that split.

use super::test_case::{DartTestCaseContext, render_test_case};
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::CallOverride;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render_client_factory_case() -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "edge_batch_empty_list", "description": "Empty batch list", "input": null
    }))
    .expect("fixture");

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "create_client".into();
    e2e_config.call.overrides.insert(
        "dart".into(),
        CallOverride {
            client_factory: Some("createClient".into()),
            ..Default::default()
        },
    );

    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let functions: Vec<crate::core::ir::FunctionDef> = Vec::new();
    let first_class_map = super::values::build_dart_first_class_map(&type_defs, &enums, &e2e_config);

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        DartTestCaseContext {
            e2e_config: &e2e_config,
            lang: "dart",
            bridge_class: &config.dart_bridge_class_name(),
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            functions: &functions,
            errors: &[],
            native_typed_dtos: true,
            // The full e2e test-file path, not the snippet path: this is the branch that
            // actually derives the mock URL into a local.
            is_snippet: false,
        },
    );
    out
}

/// The mock-URL local used to be emitted as `_mockUrl`. It is a function-local, so the
/// underscore bought nothing and only produced a lint diagnostic in every generated
/// `*_test.dart` — which `dart analyze <output_dir>` (the generated package's own lint
/// command) reads, even though the snippet validator does not.
#[test]
fn derived_mock_url_local_has_no_leading_underscore() {
    let out = render_client_factory_case();

    // Precondition, not incidental: if the client_factory branch stops deriving the URL
    // inline, the negative assertions below would pass against output that never had a
    // mock-URL local at all. Fail loudly here instead of going quietly vacuous.
    assert!(
        out.contains("_fixtureUrl("),
        "fixture no longer exercises the derived-mock-URL path; got:\n{out}"
    );
    assert!(
        out.contains("baseUrl: mockUrl"),
        "expected the derived local to be threaded into the factory call; got:\n{out}"
    );
    assert!(
        out.contains("final mockUrl = "),
        "expected an underscore-free mock-URL local; got:\n{out}"
    );
    assert!(
        !out.contains("_mockUrl"),
        "emitted a leading-underscore Dart local, which trips \
         no_leading_underscores_for_local_identifiers; got:\n{out}"
    );
}

/// The control for the test above: `_fixtureUrl` is declared at module scope by the
/// test-file emitter, where the underscore *is* meaningful Dart library privacy. A blanket
/// underscore-stripping pass would have broken this, so pin that it survives.
#[test]
fn module_scope_fixture_url_helper_keeps_its_privacy_underscore() {
    let out = render_client_factory_case();

    assert!(
        out.contains("_fixtureUrl("),
        "module-scope `_fixtureUrl` must keep its library-privacy underscore; got:\n{out}"
    );
}
