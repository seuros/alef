//! Executes the generated Python visitor-context probe and checks it actually distinguishes the
//! two shapes a PyO3 visitor bridge can hand a callback.
//!
//! **Boundary.** Nothing here builds or loads the compiled extension module -- that needs a
//! `cargo`/`maturin` build these tests cannot perform. What is executed is the pure-Python
//! surface the generator emits: the `_TestVisitor` class, driven against a stand-in for the
//! generated `#[pyclass]` and against the literal `dict` the bridge's fallback arm builds. The
//! Rust half of the bridge -- that it hands over one shape rather than the other -- is covered by
//! the render assertions in `backends::pyo3::trait_bridge::visitor_bridge::tests`, not here.
//!
//! **Why the shapes are driven, not string-matched.** A probe that reads a scalar attribute off
//! the context cannot tell a class from a mapping in general: a context type whose declared names
//! happen to be dict API names (`keys`, `items`, `values`) satisfies every `getattr` and every
//! `getattr(...)()` on a plain dict. `a_dict_whose_names_collide_with_the_dict_api_is_rejected`
//! is that case, and it is the reason the probe carries a name-independent shape check. The
//! driver observes the MAP, LIST and INDEX controls on both objects before it trusts either
//! result.
//!
//! **What is executed is what ships.** The class source handed to the driver comes from the real
//! generator functions, and [`generated_visitor_class`] asserts that exact text also appears in
//! `render_test_file`'s output. A previous PyO3 guard in this repo hand-wrote the artifact it
//! claimed to check and was therefore blind to whether the generator produced it at all. ~keep

use std::io::Write as IoWrite;
use std::process::{Command, Stdio};

use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

use super::super::helpers::core_to_binding_convertible_types;
use super::super::visitor_context::{distinct_context_probes, visitor_callback_probes};
use super::super::visitors::{emit_python_visitor_context_probes, emit_python_visitor_method};
use super::render_test_file;

const DRIVER_SOURCE: &str = include_str!("visitor_context_runtime_driver.py");
const TRAIT_NAME: &str = "DocumentWalker";
const CONTEXT_TYPE: &str = "TraversalState";
const CALLBACK: &str = "visit_heading";

/// Skips rather than fails when no interpreter is on `PATH`, mirroring `ruff_available` in
/// `lint_clean_python_tests`.
fn python_available() -> bool {
    which::which("python3").is_ok()
}

fn context_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

fn context_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: TypeRef::String,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

/// A visitor trait plus its context type. `is_clone` is what puts the context on the bridge's
/// `#[pyclass]` side of `context_binding_class`; without it the generator emits no probe and
/// every assertion below would run over an empty string.
fn context_type_defs(fields: &[&str], methods: &[&str]) -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: TRAIT_NAME.to_string(),
            rust_path: format!("sample_core::{TRAIT_NAME}"),
            is_trait: true,
            methods: vec![MethodDef {
                name: CALLBACK.to_string(),
                has_default_impl: true,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: CONTEXT_TYPE.to_string(),
            rust_path: format!("sample_core::{CONTEXT_TYPE}"),
            is_clone: true,
            fields: fields.iter().map(|name| context_field(name)).collect(),
            methods: methods.iter().map(|name| context_method(name)).collect(),
            ..TypeDef::default()
        },
    ]
}

fn bridge_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: TRAIT_NAME.to_string(),
            type_alias: Some(format!("{TRAIT_NAME}Handle")),
            context_type: Some(CONTEXT_TYPE.to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn visitor_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "visits_headings",
        "description": "Visits headings",
        "visitor": {"callbacks": {"visit_heading": {"action": "skip"}}},
    }))
    .expect("fixture must parse")
}

fn e2e_config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "do_thing".to_string(),
            module: "mypackage".to_string(),
            result_var: "result".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The generated `_TestVisitor`, lifted to module level so it can be executed standalone.
///
/// The body comes from the real generator functions, and is then asserted to appear verbatim in
/// `render_test_file`'s output -- so the text under execution is the text a consumer's suite
/// receives, not a restatement of it.
fn generated_visitor_class(fields: &[&str], methods: &[&str]) -> String {
    let type_defs = context_type_defs(fields, methods);
    let config = bridge_config();
    let fixture = visitor_fixture();
    let convertible = core_to_binding_convertible_types(&type_defs, &[]);

    let callbacks = visitor_callback_probes(&config, &type_defs, &[], &convertible, &fixture);
    let distinct = distinct_context_probes(&callbacks);
    assert!(
        !distinct.is_empty(),
        "the fixture produced no context probe, so every assertion below would run over an empty \
         class body and pass without testing anything"
    );

    let mut body = String::new();
    emit_python_visitor_context_probes(&mut body, &distinct);
    for (method_name, action, probe) in &callbacks {
        emit_python_visitor_method(
            &mut body,
            method_name,
            action,
            probe.as_ref().map(|probe| probe.probe_method.as_str()),
        );
    }

    let shipped = render_test_file(
        "visitor",
        &[&fixture],
        &e2e_config(),
        &config,
        &type_defs,
        &[],
        &[],
        &[],
        false,
    );
    assert!(
        shipped.contains(&body),
        "the class body under execution is not the one the generator ships:\n--- executed ---\n{body}\n\
         --- rendered test file ---\n{shipped}"
    );

    let dedented: String = body
        .lines()
        .map(|line| format!("{}\n", line.strip_prefix("    ").unwrap_or(line)))
        .collect();
    format!("class _TestVisitor:\n{dedented}")
}

/// Runs the driver over `class_source`, returning whether every expectation held and the JSON
/// report it wrote.
fn run_driver(class_source: &str, attributes: &[&str], methods: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let driver_path = dir.path().join("visitor_context_runtime_driver.py");
    std::fs::write(&driver_path, DRIVER_SOURCE).expect("write driver");

    let job = serde_json::json!({
        "class_source": class_source,
        "callback": CALLBACK,
        "attributes": attributes,
        "methods": methods,
    })
    .to_string();

    let mut child = Command::new("python3")
        .arg(&driver_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");
    child
        .stdin
        .take()
        .expect("driver stdin")
        .write_all(job.as_bytes())
        .expect("write job");
    let output = child.wait_with_output().expect("run driver");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// The base case: the probe accepts the class the binding declares and rejects the dict the
/// bridge's fallback arm builds. Both directions run in one driver invocation, so an accept-all
/// or reject-all probe fails whichever half it did not intend.
#[test]
fn a_class_shaped_context_is_accepted_and_a_dict_shaped_one_is_rejected() {
    if !python_available() {
        return;
    }

    let attributes = ["node_kind", "depth"];
    let methods = ["attributes"];
    let class_source = generated_visitor_class(&attributes, &methods);
    let (ok, report) = run_driver(&class_source, &attributes, &methods);

    assert!(
        ok,
        "generated probe failed to separate the context shapes:\n{report}\n--- executed ---\n{class_source}"
    );
}

/// The case a name-based probe cannot see. Every declared name here is also a `dict` attribute,
/// so `getattr(ctx, "values")` and `getattr(ctx, "items")()` both succeed on the fallback dict
/// and the probe's `AttributeError` handlers never fire. Only the shape check rejects it.
///
/// Deleting the `isinstance(ctx, dict)` branch from `visitor_context_probe.jinja` makes this test
/// fail while `a_class_shaped_context_is_accepted_and_a_dict_shaped_one_is_rejected` still
/// passes -- that asymmetry is what this case exists to record. ~keep
#[test]
fn a_dict_whose_names_collide_with_the_dict_api_is_rejected() {
    if !python_available() {
        return;
    }

    let attributes = ["values"];
    let methods = ["items"];
    let class_source = generated_visitor_class(&attributes, &methods);
    assert!(
        class_source.contains("\"values\",") && class_source.contains("\"items\","),
        "the collision surface never reached the generated probe, so the dict below would be \
         rejected by an ordinary name probe instead:\n{class_source}"
    );

    let (ok, report) = run_driver(&class_source, &attributes, &methods);
    assert!(
        ok,
        "a dict whose keys and methods shadow the declared surface must still be rejected:\n{report}\n\
         --- executed ---\n{class_source}"
    );
}

/// Negative control for the harness itself. A `_TestVisitor` that records nothing must make the
/// driver fail; without this, a driver that silently exited 0 (bad interpreter, unread stdin, an
/// expectation loop that never ran) would render both tests above green no matter what the
/// generator emitted.
///
/// The stub below is a stand-in for the *harness*, never for the generated artifact -- the two
/// tests that judge the generator are fed the real rendered text. ~keep
#[test]
fn the_driver_fails_when_the_probe_records_nothing() {
    if !python_available() {
        return;
    }

    let inert = concat!(
        "class _TestVisitor:\n",
        "    def __init__(self):\n",
        "        self.context_errors = []\n",
        "        self.context_reads = 0\n",
        "\n",
        "    def visit_heading(self, ctx, level, text, id):\n",
        "        self.context_reads += 1\n",
        "        return \"skip\"\n",
    );

    let (ok, report) = run_driver(inert, &["values"], &["items"]);
    assert!(!ok, "the driver must be able to fail; it reported:\n{report}");
    assert!(
        report.contains("dict-shaped context was accepted"),
        "the driver must name the expectation that failed, got:\n{report}"
    );
}
