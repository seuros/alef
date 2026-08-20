use super::*;

fn error_fixture() -> Fixture {
    let mut fixture = Fixture {
        id: "invalid_input".into(),
        description: "Rejects invalid input".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });
    fixture
}

#[test]
fn error_path_test_fails_zig_test_on_unexpected_success_for_non_json_result() {
    let fixture = error_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    let rendered = render_test_file(
        "error",
        &[&fixture],
        &e2e,
        "parse",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    // The old shape `catch { try testing.expect(true); return; }` never fails:
    // on success the catch body simply doesn't run and the tautological
    // `expect(true)` can't fail either. Assert the actual failing construct
    // is present, not merely the absence of the old text.
    assert!(
        rendered.contains("if (sample.parse()) |_| {"),
        "expected error-union if/else on the call, got:\n{rendered}"
    );
    assert!(
        rendered.contains("return error.TestUnexpectedResult;"),
        "success arm must fail the test, got:\n{rendered}"
    );
    assert!(
        rendered.contains("} else |_| {}"),
        "error arm must be reachable and not swallow via `catch`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("expect(true)"),
        "vacuous assertion must be gone:\n{rendered}"
    );
    assert!(
        !rendered.contains(" catch {"),
        "must not fall through a swallowing catch:\n{rendered}"
    );
}

#[test]
fn error_path_test_fails_zig_test_on_unexpected_success_for_json_struct_result() {
    let fixture = error_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::core::config::e2e::CallOverride {
            result_is_json_struct: true,
            ..Default::default()
        },
    );
    let rendered = render_test_file(
        "error",
        &[&fixture],
        &e2e,
        "parse",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(
        rendered.contains("if (sample.parse()) |_| {"),
        "expected error-union if/else on the call, got:\n{rendered}"
    );
    assert!(
        rendered.contains("return error.TestUnexpectedResult;"),
        "success arm must fail the test, got:\n{rendered}"
    );
    assert!(
        rendered.contains("} else |_| {}"),
        "error arm must be reachable and not swallow via `catch`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("expect(true)"),
        "vacuous assertion must be gone:\n{rendered}"
    );
    assert!(
        !rendered.contains(" catch {"),
        "must not fall through a swallowing catch:\n{rendered}"
    );
}
