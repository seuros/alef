//! R e2e individual test case rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_ident;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::{args, assertions, visitor};

/// Render the R `expect_error(...)`/`tryCatch` block for an `error`-asserting test.
///
/// ~keep With no declared value this returns output byte-identical to the bare
/// `expect_error(fn(args))` call. testthat's `expect_error(regexp=, class=)`
/// combine with AND semantics, so they can't express the message-or-class-name
/// disjunction other backends use (see `declared_error_value`'s doc comment);
/// a manual `tryCatch` is required instead.
fn render_r_error_check(function_name: &str, final_args: &str, declared_error: Option<&str>) -> String {
    match declared_error {
        Some(value) => {
            let pattern = crate::e2e::escape::r_regex_literal(value);
            format!(
                "  tryCatch({{\n    {function_name}({final_args})\n    fail(\"expected an error to be thrown\")\n  }}, error = function(e) {{\n    expect_true(grepl({pattern}, conditionMessage(e)) || grepl({pattern}, paste(class(e), collapse = \" \")))\n  }})"
            )
        }
        None => format!("  expect_error({function_name}({final_args}))"),
    }
}

pub(super) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    default_result_is_simple: bool,
    default_result_is_r_list: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let (ir_reachable_fields, ir_known_excluded_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
    let field_resolver = &call_field_resolver;
    // Resolve `function` via the R override when present. The default
    // `call_config.function` can be empty (e.g. trait-bridge calls like
    // `clear_document_extractors` set `function = ""` at the top level and
    // expose the real binding name only through per-language overrides);
    // emitting it verbatim produces invalid `result <- ()` calls.
    let function_name = call_config
        .overrides
        .get("r")
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    // Per-fixture call configs (e.g. `list_document_extractors`) may set
    // `result_is_simple = true` even when the default `[e2e.call]` does not.
    // Without this lookup the registry/detection wrappers (which return scalar
    // strings or character vectors directly) get wrapped in
    // `jsonlite::fromJSON(...)` and the parser fails on non-JSON output.
    let r_override = call_config.overrides.get("r");
    let result_is_simple = if fixture.call.is_some() {
        call_config.result_is_simple || r_override.is_some_and(|o| o.result_is_simple)
    } else {
        default_result_is_simple
    };
    // Per-fixture override: when the R binding already returns a native R list
    // (not a JSON string), suppress `jsonlite::fromJSON` wrapping while still
    // using field-path (`result$field`) accessors in assertions.
    let result_is_r_list = if fixture.call.is_some() {
        r_override.is_some_and(|o| o.result_is_r_list)
    } else {
        default_result_is_r_list
    };

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('"', "\\\"");

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Allow per-call R overrides to remap fixture argument names. Many calls
    // (e.g. `extract_bytes`, `batch_extract_files`) use language-neutral
    // fixture field names (`data`, `paths`) that the R extendr binding
    // exposes under different identifiers (`content`, `items`).
    let arg_name_map = r_override.map(|o| &o.arg_name_map);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("r", fixture, call_config, type_defs);
    let options_type = recipe.compatible_options_type(&["r", "csharp", "java", "go", "php", "python"]);
    // Build visitor setup and args if present
    let mut setup_lines = Vec::new();
    let mut teardown_block = String::new();
    let args_str = args::build_args_string(
        &fixture.input,
        fixture.resolved_args(call_config),
        args::RArgsContext {
            arg_name_map,
            options_type,
            fixture,
            config,
            type_defs,
            setup_lines: &mut setup_lines,
            teardown_block: &mut teardown_block,
        },
    );

    // Per-call R extra_args: positional trailing arguments appended verbatim.
    // Used when the extendr wrapper has more parameters than the fixture
    // declares (e.g. `render_pdf_page_to_png(pdf_bytes, page_index, dpi,
    // password)` where `dpi`/`password` are optional in Rust but extendr
    // surfaces them as required R parameters with no defaults).
    let r_extra_args: Vec<String> = r_override.map(|o| o.extra_args.clone()).unwrap_or_default();
    let args_with_extra = if r_extra_args.is_empty() {
        args_str
    } else {
        let extra = r_extra_args.join(", ");
        if args_str.is_empty() {
            extra
        } else {
            format!("{args_str}, {extra}")
        }
    };
    let final_args = if let Some(visitor_spec) = &fixture.visitor {
        visitor::build_r_visitor(&mut setup_lines, visitor_spec);
        // R rejects duplicated named arguments ("matched by multiple actual arguments"), so
        // strip any existing `options = ...` arg before appending the visitor-options list.
        // Handles `options = NULL` (when no default) and `options = <OptionsType>$default()`
        // (when build_args_string emits a default placeholder for an optional options arg).
        let base = args::strip_options_arg(&args_with_extra);
        let visitor_opts = "options = list(visitor = visitor)";
        let trimmed = base.trim_matches([' ', ',']);
        if trimmed.is_empty() {
            visitor_opts.to_string()
        } else {
            format!("{trimmed}, {visitor_opts}")
        }
    } else {
        args_with_extra
    };

    if expects_error {
        let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
        for line in &setup_lines {
            let _ = writeln!(out, "  {line}");
        }
        let error_check = render_r_error_check(
            &function_name,
            &final_args,
            crate::e2e::codegen::declared_error_value(fixture),
        );
        let _ = writeln!(out, "{error_check}");
        let _ = writeln!(out, "}})");
        return;
    }

    let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
    for line in &setup_lines {
        let _ = writeln!(out, "  {line}");
    }
    // The extendr extraction wrappers return JSON strings carrying the
    // serialized core result; parse into an R list so tests can use `$`
    // accessors. `result_is_simple` calls
    // already return scalar values and must be passed through verbatim.
    // `result_is_r_list` signals the binding returns a native R list (Robj),
    // not a JSON string — skip `jsonlite::fromJSON` but keep `$` accessors.
    // `returns_void` calls (trait-bridge `clear_*` wrappers that return `()`
    // in Rust → `NULL` in R) must not bind a `result` variable: the previous
    // emission of `result <- {function_name}(...)` was already correct when
    // `function_name` resolved, but parsers flag a stray `result` for void
    // calls. Use `invisible(...)` to make the void contract explicit.
    if call_config.returns_void {
        let _ = writeln!(out, "  invisible({function_name}({final_args}))");
    } else if result_is_simple || result_is_r_list {
        let _ = writeln!(out, "  {result_var} <- {function_name}({final_args})");
    } else {
        let _ = writeln!(
            out,
            "  {result_var} <- jsonlite::fromJSON({function_name}({final_args}), simplifyVector = FALSE)"
        );
    }

    let result_is_bytes = call_config.result_is_bytes || r_override.is_some_and(|o| o.result_is_bytes);
    // Resolve assert_enum_fields from the R-language override so the assertion renderer
    // can identify fields that require the `.alef_format_value` wrapper rather than
    // matching against the literal field path "metadata.format".
    static EMPTY_ASSERT_ENUM_FIELDS: std::sync::LazyLock<std::collections::HashMap<String, String>> =
        std::sync::LazyLock::new(std::collections::HashMap::new);
    let assert_enum_fields = r_override
        .map(|o| &o.assert_enum_fields)
        .unwrap_or(&EMPTY_ASSERT_ENUM_FIELDS);
    // `out` accumulates every fixture's rendered test case in this file (see the
    // caller in `test_file.rs`), so the strict-availability scan below must only
    // look at the text this fixture's own assertion loop appends — scanning the
    // whole buffer would misattribute an earlier fixture's skip comment to this
    // fixture's id. ~keep
    let assertions_start = out.len();
    for assertion in &fixture.assertions {
        let context = assertions::RAssertionContext {
            field_resolver,
            result_is_simple,
            result_is_bytes,
            assert_enum_fields,
        };
        assertions::render_assertion(out, assertion, result_var, &context);
    }
    apply_vacuous_assertion_fallback(
        out,
        assertions_start,
        !fixture.assertions.is_empty(),
        call_config.returns_void,
        result_var,
    );
    crate::e2e::codegen::fail_on_unavailable_field_markers(&out[assertions_start..], "r", &fixture.id);

    // Emit teardown for trait-bridge tests to clean up registered test backends.
    for line in teardown_block.lines() {
        let _ = writeln!(out, "{line}");
    }

    let _ = writeln!(out, "}})");
}

/// When a fixture declares at least one assertion but the rendered body has no
/// executable statement — every field assertion resolved to a "skipped: ..."
/// comment because the field is unavailable on the result type — inject a
/// real assertion instead of leaving the test vacuous. `not_error` already
/// renders a real `expect_true(TRUE)` on its own (see
/// `assertions::render_assertion`'s `not_error` arm), so this only fires on
/// the remaining gap: declared field assertions that all turned out
/// unavailable. Fixtures that declare NO assertions at all are left
/// untouched — a deliberate "just call it" smoke test, matching every other
/// backend in this defect class (mirrors typescript's
/// `apply_vacuous_assertion_fallback`). `returns_void` calls never bind
/// `result_var` (see the `invisible(...)` branch above), so this must never
/// fire for them — referencing an unbound variable would not compile. ~keep
fn apply_vacuous_assertion_fallback(
    out: &mut String,
    assertions_start: usize,
    has_declared_assertions: bool,
    returns_void: bool,
    result_var: &str,
) {
    if returns_void || !has_declared_assertions {
        return;
    }
    let has_real_assertion = out[assertions_start..]
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with('#'));
    if has_real_assertion {
        return;
    }
    let _ = writeln!(out, "  expect_true(!is.null({result_var}))");
}

#[cfg(test)]
mod render_r_error_check_tests {
    use super::render_r_error_check;

    #[test]
    fn no_declared_value_is_byte_identical_to_bare_expect_error() {
        assert_eq!(
            render_r_error_check("extract_bytes", "data", None),
            "  expect_error(extract_bytes(data))"
        );
    }

    #[test]
    fn declared_value_adds_message_or_class_name_check() {
        let check = render_r_error_check("extract_bytes", "data", Some("BadRequest"));
        assert_eq!(
            check,
            "  tryCatch({\n    extract_bytes(data)\n    fail(\"expected an error to be thrown\")\n  }, error = function(e) {\n    expect_true(grepl(\"BadRequest\", conditionMessage(e)) || grepl(\"BadRequest\", paste(class(e), collapse = \" \")))\n  })"
        );
    }

    #[test]
    fn declared_value_with_regex_metacharacters_is_escaped() {
        let check = render_r_error_check("extract_bytes", "data", Some("field.name[0]"));
        assert!(
            check.contains("\"field\\\\.name\\\\[0\\\\]\""),
            "expected escaped regex literal, got: {check}"
        );
    }
}

#[cfg(test)]
mod vacuous_assertion_fallback_tests {
    use super::render_test_case;
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    /// A fixture whose sole assertion targets a field absent from
    /// `result_fields` renders only a "skipped" comment for that assertion.
    /// `apply_vacuous_assertion_fallback` must inject a real
    /// `expect_true(!is.null(result))` so the generated `testthat` block is
    /// never vacuously passing.
    #[test]
    fn dropped_field_assertion_still_gets_a_real_fallback_assertion() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "process".to_string();
        e2e_config.call.result_var = "result".to_string();
        e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);

        let fixture = Fixture {
            id: "process_smoke".to_string(),
            description: "test".to_string(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("nonexistent_field".to_string()),
                value: Some(serde_json::json!("x")),
                ..Assertion::default()
            }],
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[]);

        assert!(
            out.contains("# skipped: field 'nonexistent_field' not available on result type"),
            "expected the dropped assertion's skip comment, got:\n{out}"
        );
        assert!(
            out.contains("expect_true(!is.null(result))"),
            "expected a real fallback assertion on the discarded result, got:\n{out}"
        );
    }

    /// Positive control for the same fix: a fixture with genuinely zero
    /// declared assertions is left untouched (deliberate "just call it"
    /// smoke test), matching every other backend in this defect class.
    #[test]
    fn zero_declared_assertions_are_left_untouched() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "process".to_string();
        e2e_config.call.result_var = "result".to_string();

        let fixture = Fixture {
            id: "process_smoke".to_string(),
            description: "test".to_string(),
            assertions: Vec::new(),
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[]);

        assert!(
            !out.contains("expect_true(!is.null(result))"),
            "a fixture with zero declared assertions must stay vacuous, got:\n{out}"
        );
    }
}
