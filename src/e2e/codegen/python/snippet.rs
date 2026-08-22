//! Documentation-snippet rendering for the Python e2e backend.
//!
//! Split out of `mod.rs` so the snippet concern — which now consults the shared
//! [`crate::e2e::codegen::snippet_error_branch`] seam — has one file of its own rather than
//! growing an already over-cap module.

use anyhow::Result;

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::helpers;
use super::test_file::render_test_file;

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    errors: &[crate::core::ir::ErrorDef],
) -> Result<String> {
    let mut call_fixture = fixture.docs_call_fixture();
    let expects_error = call_fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    call_fixture.assertions.clear();
    call_fixture.mock_response = None;
    // With `mock_response` cleared, `test_function`'s `client_factory` path falls through
    // to its "declared env var" branch whenever `fixture.env.api_key_var` is set — which
    // guards the read with `pytest.skip(...)` for the executable e2e suite's real-API-vs-mock
    // dispatch. A docs snippet is not a pytest test function, so that guard renders as an
    // undefined name. Clearing `env` here (the credential var name below still reads it from
    // the *original* `fixture`) drops straight to the same bare `api_key="test-key"` shape the
    // substitution just below already targets. ~keep
    call_fixture.env = None;
    let presentation = crate::e2e::codegen::presentation::resolve(&call_fixture, e2e_config, "python", type_defs);
    let call = e2e_config.resolve_call_for_fixture(
        call_fixture.call.as_deref(),
        &call_fixture.id,
        &call_fixture.resolved_category(),
        &call_fixture.tags,
        &call_fixture.input,
    );
    // Fixture assertions are cleared above because snippets do not render test
    // assertions, so the usual assertion-driven binding heuristic cannot see that the
    // reader-facing presentation consumes `result_var`. Bind every non-void successful
    // call: the template either prints the result itself or presents fields from it. ~keep
    let force_bind_result = !expects_error && !call.returns_void;
    let test_file = render_test_file(
        &fixture.resolved_category(),
        &[&call_fixture],
        e2e_config,
        config,
        type_defs,
        enums,
        &[],
        force_bind_result,
    );
    let (imports, body, is_async) = extract_python_snippet(&test_file)?;
    let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let body = body
        .into_iter()
        .map(|line| {
            line.replace(
                "api_key=\"test-key\"",
                &format!("api_key=os.environ[\"{api_key_var}\"]"),
            )
        })
        .collect::<Vec<_>>();
    // A `configuration/custom-base-url`-style topic documents `docs.client.base_url` so the
    // reader sees the setting the topic is about, mirroring the Java/Rust/Elixir generators'
    // `docs_client` handling. The client-construction line always ends in exactly this
    // single-argument shape immediately after the substitution above, so targeting it here
    // (rather than threading `docs_client` through `render_test_file`/`test_function`, which
    // also serves the executable e2e suite) keeps this docs-only concern out of the shared
    // test-rendering path entirely. ~keep
    let body = match fixture.docs_client().and_then(|client| client.base_url.as_deref()) {
        Some(base_url) => {
            let bare_call = format!("api_key=os.environ[\"{api_key_var}\"])");
            let with_base_url = format!(
                "api_key=os.environ[\"{api_key_var}\"], base_url=\"{}\")",
                crate::e2e::escape::escape_python(base_url)
            );
            body.into_iter()
                .map(|line| line.replace(&bare_call, &with_base_url))
                .collect::<Vec<_>>()
        }
        None => body,
    };
    let error_type = config.error_type_name();
    let mut imports = imports.into_iter().map(str::to_string).collect::<Vec<_>>();
    if body.iter().any(|line| line.contains("os.environ")) && !imports.iter().any(|line| line == "import os") {
        imports.push("import os".to_string());
    }
    // ~keep A fixture that names the variant it provokes (`{"type": "error", "value":
    // "Authentication"}`) documents a specific failure mode, and pyo3 already generates a
    // distinct exception class for it. Catching that class *before* the flat base class is the
    // difference between a snippet that teaches "handle a 401 by rotating the key" and one that
    // teaches "print the class name". The decision is not made here: `snippet_error_branch`
    // answers it once for every backend, so this branch and the e2e suite's type assertion can
    // never claim different things about the same variant. `None` — no declared variant, or a
    // message-style value — keeps the pre-existing single generic `except` unchanged.
    let typed_branch = crate::e2e::codegen::snippet_error_branch::for_fixture("python", fixture, errors)
        .filter(|branch| branch.host_type != error_type);
    if expects_error {
        let module = helpers::resolve_module(e2e_config);
        // ~keep Sorted because a published snippet is linted in the consumer repo, where ruff's
        // unsorted-imports rule fires on `from pkg import Error, AuthenticationError`.
        let mut names = vec![error_type.clone()];
        names.extend(typed_branch.as_ref().map(|branch| branch.host_type.clone()));
        names.sort();
        imports.push(format!("from {module} import {}", names.join(", ")));
    }
    Ok(crate::e2e::template_env::render(
        "python/snippet_body.py.jinja",
        minijinja::context! {
            imports => imports, body => body, is_async => is_async, presentation => presentation,
            expects_error => expects_error,
            error_type => error_type,
            typed_error_type => typed_branch.as_ref().map(|branch| branch.host_type.clone()),
            result_var => call.effective_result_var(),
            returns_void => call.returns_void,
        },
    ))
}

fn extract_python_snippet(rendered: &str) -> Result<(Vec<&str>, Vec<&str>, bool)> {
    let lines = rendered.lines().collect::<Vec<_>>();
    let signature = lines
        .iter()
        .position(|line| line.starts_with("async def test_") || line.starts_with("def test_"))
        .ok_or_else(|| anyhow::anyhow!("generated Python test did not contain a fixture function"))?;
    let imports = lines[..signature]
        .iter()
        .copied()
        .filter(|line| (line.starts_with("from ") || line.starts_with("import ")) && !line.contains("pytest"))
        .collect();
    let body = lines[signature + 1..]
        .iter()
        .copied()
        .filter_map(|line| line.strip_prefix("    "))
        .filter(|line| !line.trim_start().starts_with("\"\"\"") && !line.trim().is_empty())
        .collect();
    Ok((imports, body, lines[signature].starts_with("async def ")))
}

#[cfg(test)]
mod tests {
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::codegen::python::PythonE2eCodegen;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::Fixture;

    const CONFIG: &str = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat"
module = "example_api"
args = [{ name = "prompt", field = "prompt", type = "string" }]
"#;

    fn config() -> (E2eConfig, ResolvedCrateConfig) {
        let cfg: NewAlefConfig = toml::from_str(CONFIG).expect("config parses");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let resolved = cfg.resolve().expect("config resolves").remove(0);
        (e2e, resolved)
    }

    fn error_fixture(declared: &str) -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "auth_401",
            "description": "401 Unauthorized",
            "input": {"prompt": "Hello"},
            "assertions": [
                {"type": "error"},
                {"type": "error", "value": declared},
            ]
        }))
        .expect("fixture must parse")
    }

    fn errors_with(variant: &str) -> Vec<ErrorDef> {
        vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant.to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    /// The defect this module's `snippet_error_branch` wiring closes. A fixture that names the
    /// variant it provokes used to render a single `except Error` catch-all printing
    /// `type(error).__name__` — documentation that teaches "catch everything and print the class
    /// name" while the binding it documents exports a distinct `AuthenticationError` class. The
    /// typed branch must come FIRST: Python matches `except` clauses in order and the generated
    /// per-variant classes subclass the flat base, so a leading catch-all makes every following
    /// branch unreachable.
    #[test]
    fn a_fixture_naming_an_error_variant_catches_that_variants_exception_first() {
        let (e2e, resolved) = config();
        let rendered = PythonE2eCodegen
            .render_snippet_body_with_functions(
                &error_fixture("Authentication"),
                &e2e,
                &resolved,
                &[],
                &[],
                &[],
                &errors_with("Authentication"),
            )
            .expect("snippet renders");
        let typed = rendered
            .find("except AuthenticationError as error:")
            .unwrap_or_else(|| panic!("no typed catch branch in:\n{rendered}"));
        let generic = rendered
            .find("except Error as error:")
            .unwrap_or_else(|| panic!("no catch-all branch in:\n{rendered}"));
        assert!(typed < generic, "typed branch must precede the catch-all:\n{rendered}");
        assert!(
            rendered.contains("from example_api import AuthenticationError, Error"),
            "both exception classes must be imported:\n{rendered}"
        );
    }

    /// The deliberate fallback, tested rather than incidental: with no IR error registry the
    /// declared value cannot be resolved to a variant, so the snippet is byte-identical to what
    /// it rendered before the typed branch existed.
    #[test]
    fn without_an_error_registry_the_snippet_keeps_its_single_generic_branch() {
        let (e2e, resolved) = config();
        let rendered = PythonE2eCodegen
            .render_snippet_body(&error_fixture("Authentication"), &e2e, &resolved, &[], &[])
            .expect("snippet renders");
        assert!(rendered.contains("except Error as error:"), "{rendered}");
        assert!(!rendered.contains("except AuthenticationError"), "{rendered}");
        assert_eq!(rendered.matches("except ").count(), 1, "{rendered}");
    }

    /// A message-style declared value names no variant, so it must not invent a class name.
    #[test]
    fn a_message_style_declared_value_keeps_the_generic_branch() {
        let (e2e, resolved) = config();
        let rendered = PythonE2eCodegen
            .render_snippet_body_with_functions(
                &error_fixture("prompt must not be empty"),
                &e2e,
                &resolved,
                &[],
                &[],
                &[],
                &errors_with("Authentication"),
            )
            .expect("snippet renders");
        assert_eq!(rendered.matches("except ").count(), 1, "{rendered}");
        assert!(rendered.contains("except Error as error:"), "{rendered}");
    }
}
