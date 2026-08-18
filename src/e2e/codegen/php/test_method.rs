use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_php_single, php_pcre_literal};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::{args, assertions, stubs, types, visitor};

/// True when `body` contains at least one line that is not blank and not a
/// `//`-prefixed comment — i.e. an executable PHPUnit assertion statement.
/// A body made up only of "// skipped: ..." lines is not executable and
/// must not be treated as if the fixture asserted something.
fn has_executable_assertion(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with("//")
    })
}

/// When a fixture's rendered `assertions_body` has no executable statement —
/// its only assertion was `not_error`, or every field assertion resolved to a
/// "skipped" comment — inject a real assertion instead of leaving the
/// PHPUnit test vacuous. Never falls back to `expectNotToPerformAssertions()`:
/// that only quiets PHPUnit's own risky-test detector without asserting
/// anything, certifying untested behaviour as green. ~keep
fn apply_vacuous_assertion_fallback(
    assertions_body: &mut String,
    is_streaming: bool,
    expects_error: bool,
    result_var: &str,
) {
    if expects_error || has_executable_assertion(assertions_body) {
        return;
    }
    if is_streaming {
        assertions_body.push_str("        $this->assertTrue(is_array($chunks), 'expected drained chunks list');\n");
    } else {
        let _ = writeln!(assertions_body, "        $this->assertNotNull(${result_var});");
    }
}

/// Render the PHP test-method body for an `error`-asserting fixture.
///
/// ~keep With no declared value this returns output byte-identical to the
/// pre-existing `expectException` idiom. PHPUnit's `expectException*` family
/// only ever inspects the thrown exception's message, so matching the
/// message-or-class-name disjunction other backends use (see
/// `declared_error_value`'s doc comment) requires a manual try/catch instead
/// of `expectExceptionMessageMatches`.
fn render_error_test_body(setup_lines: &[String], call_expr: &str, declared_error: Option<&str>) -> String {
    let mut out = String::new();
    match declared_error {
        Some(value) => {
            let pattern = php_pcre_literal(value);
            // The failure message is its own single-quoted PHP string literal, separate
            // from `pattern` (itself already a quoted `'/.../'` literal). Interpolating
            // `pattern` directly here would nest one single-quoted string inside another
            // and produce invalid PHP — escape the raw declared value for this string
            // instead of reusing the pre-quoted PCRE literal.
            let message_value = escape_php_single(value);
            out.push_str("        try {\n");
            for line in setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            let _ = writeln!(out, "            {call_expr};");
            out.push_str("            $this->fail('Expected an exception to be thrown');\n");
            out.push_str("        } catch (\\Exception $e) {\n");
            let _ = writeln!(
                out,
                "            $this->assertTrue(preg_match({pattern}, $e->getMessage()) === 1 || preg_match({pattern}, get_class($e)) === 1, 'expected exception message or class name to match {message_value}');"
            );
            out.push_str("        }");
        }
        None => {
            out.push_str("        $this->expectException(\\Exception::class);\n");
            for line in setup_lines {
                let _ = writeln!(out, "        {line}");
            }
            let _ = write!(out, "        {call_expr};");
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    lang: &str,
    namespace: &str,
    class_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
    php_enum_names: &HashSet<String>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    php_lang_rename_all: &str,
    config: &ResolvedCrateConfig,
    trait_bridge_imports: &mut Vec<String>,
) {
    // Resolve per-fixture call config: supports named calls via fixture.call field.
    let mut call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Fallback: if the resolved call has required args missing from input,
    // try to find a better-matching call from the named calls.
    call_config = crate::e2e::codegen::select_best_matching_call(call_config, e2e_config, fixture);
    // Build per-call PHP getter map and field resolver using the effective field sets.
    let per_call_getter_map = types::build_php_getter_map(
        type_defs,
        php_enum_names,
        call_config,
        e2e_config.effective_result_fields(call_config),
    );
    let (ir_reachable_fields, ir_known_excluded_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_field_resolver = FieldResolver::new_with_php_getters(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
        &HashMap::new(),
        per_call_getter_map,
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
    let field_resolver = &call_field_resolver;
    let call_overrides = call_config.overrides.get(lang);
    let has_override = call_overrides.is_some_and(|o| o.function.is_some());
    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and fall back to the per-call language override or the
    // file-level language default for backwards compatibility.
    let result_is_simple =
        call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple) || result_is_simple;
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // The PHP facade exposes async Rust methods under their bare name (no `_async`
    // suffix) — PHP has no surface-level async, so the facade picks the async
    // implementation as the default and delegates to `*Async` on the native class.
    // The `*_sync` variants stay explicit (e.g. `extract_bytes_sync` → `extractBytesSync`).
    if !has_override {
        function_name = function_name.to_lower_camel_case();
    }
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;

    let method_name = crate::e2e::escape::sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Resolve options_type for this call. Precedence: per-language call override,
    // then the call-level `options_type` (the binding-agnostic config parameter type,
    // a call-specific options type), then the global per-language call override (fallback default).
    let call_options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.options_type.as_deref())
    });

    let adapter_lookup_name = call_config.core_lookup_name(lang);
    let call_adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| adapters.iter().find(|a| a.name == name));
    let adapter_request_type: Option<String> = call_adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`$engine->streamItems($req)`), not as static facade methods.
    // Capture the owner handle variable so the call is rendered as an
    // instance-method invocation and the handle is omitted from the argument list.
    let streaming_owner_handle: Option<String> = if call_adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    }) {
        args.iter().find(|a| a.arg_type == "handle").map(|a| a.name.clone())
    } else {
        None
    };

    let (mut setup_lines, args_str, teardown_block) = args::build_args_and_setup(
        &fixture.input,
        args,
        class_name,
        enum_fields,
        fixture,
        options_via,
        call_options_type,
        adapter_request_type.as_deref(),
        namespace,
        streaming_owner_handle.is_some(),
        type_defs,
        php_lang_rename_all,
        config,
        trait_bridge_imports,
        false,
    );

    // Check for skip_languages early
    let skip_test = call_config.skip_languages.iter().any(|l| l == "php");
    if skip_test {
        let rendered = crate::e2e::template_env::render(
            "php/test_method.jinja",
            minijinja::context! {
                method_name => method_name,
                description => description,
                client_factory => String::new(),
                setup_lines => Vec::<String>::new(),
                expects_error => false,
                skip_test => true,
                call_expr => String::new(),
                result_var => result_var,
                assertions_body => String::new(),
            },
        );
        out.push_str(&rendered);
        return;
    }

    // Build visitor if present and add to setup
    let mut options_already_created = !args_str.is_empty() && args_str == "$options";
    if let Some(visitor_spec) = &fixture.visitor {
        visitor::build_php_visitor(&mut setup_lines, visitor_spec);
        if !options_already_created {
            // A visitor has no standalone PHP parameter — it is only reachable by
            // assignment onto an options object, so without a resolvable options type
            // there is nowhere to attach it. This used to re-render the template with
            // `skip_test => true` — the *same* output the sanctioned `skip_languages`
            // branch above emits — so a config failure was indistinguishable from an
            // author-declared skip, and the suite reported green either way. That
            // sanctioned branch has already returned by here, so reaching this point
            // means the fixture is genuinely required for PHP. Fail generation instead
            // of emitting a test that silently drops the visitor. ~keep
            let Some(options_type) = call_options_type.or_else(|| stubs::trait_bridge_options_type(config)) else {
                panic!(
                    "PHP e2e generator: fixture `{}` declares a `visitor`, but neither its `[e2e.call]` config nor any `[[crates.trait_bridges]]` entry provides an `options_type` to attach it to; cannot generate a PHP visitor test without a resolvable trait bridge options type",
                    fixture.id
                );
            };
            if options_via == "from_json" {
                // When options_via is "from_json", create options from JSON first,
                // then attach the visitor using with_visitor() since PHP closures can't be JSON-encoded.
                setup_lines.push(format!("$options = \\{namespace}\\{options_type}::from_json('{{}}');"));
                setup_lines.push(format!(
                    "$visitorHandle = \\{namespace}\\VisitorHandle::from_php_object($visitor);"
                ));
                // ext-php-rs camel-cases snake_case method names; the generated PHP class
                // exposes the wither as `withVisitor`, not `with_visitor`.
                setup_lines.push("$options = $options->withVisitor($visitorHandle);".to_string());
            } else {
                // Default builder pattern for other options_via modes
                setup_lines.push(format!("$builder = \\{namespace}\\{options_type}::builder();"));
                setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
            }
            options_already_created = true;
        }
    }

    let final_args = if options_already_created {
        if args_str.is_empty() || args_str == "$options" {
            "$options".to_string()
        } else {
            format!("{args_str}, $options")
        }
    } else {
        args_str
    };

    let call_expr = if php_client_factory.is_some() {
        format!("$client->{function_name}({final_args})")
    } else if let Some(ref handle_var) = streaming_owner_handle {
        // Instance-method invocation on the owner handle.
        format!("${handle_var}->{function_name}({final_args})")
    } else {
        format!("{class_name}::{function_name}({final_args})")
    };

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_factory = if let Some(factory) = php_client_factory {
        let fixture_id = &fixture.id;
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            format!(
                "$apiKey = getenv('{var}');\n        $baseUrl = ($apiKey !== false && $apiKey !== '') ? null : getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';\n        fwrite(STDERR, \"{fixture_id}: \" . ($baseUrl === null ? 'using real API ({var} is set)' : 'using mock server ({var} not set)') . \"\\n\");\n        $client = \\{namespace}\\{class_name}::{factory}($baseUrl === null ? $apiKey : 'test-key', $baseUrl);"
            )
        } else if has_mock {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!("(getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}')")
            } else {
                format!("getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}'")
            };
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key', {base_url_expr});")
        } else if let Some(var) = api_key_var {
            format!(
                "$apiKey = getenv('{var}');\n        if (!$apiKey) {{ $this->markTestSkipped('{var} not set'); return; }}\n        $client = \\{namespace}\\{class_name}::{factory}($apiKey);"
            )
        } else {
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key');")
        }
    } else {
        String::new()
    };

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());

    // For streaming fixtures, emit collect snippet after the result assignment.
    let collect_snippet = if is_streaming {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet("php", result_var, "chunks")
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Collect fields_array fields that are referenced in assertions
    // so we can emit bindings for them (e.g., $chunks = $result->getChunks();).
    //
    // Use a BTreeMap (sorted by key) so the emitted accessor extraction lines
    // appear in a stable order across regens. A HashMap here previously leaked
    // its randomized iteration order into the generated PHP source, causing
    // e.g. parser-pack's `e2e/php/tests/ProcessTest.php` to flip the relative order
    // of `$imports` vs `$structure` bindings between back-to-back
    // `alef e2e generate` invocations.
    let mut fields_array_bindings: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            // Skip enum variant accessor paths (metadata.format.excel etc.)
            let is_enum_variant_accessor = f.contains("metadata.format.") && f.matches('.').count() >= 2;
            if !f.is_empty()
                && !is_enum_variant_accessor
                && field_resolver.is_array(f)
                // Only collect bindings for fields that are valid on the result type
                && field_resolver.is_valid_for_result(f)
            {
                // Only emit binding if not already added
                if !fields_array_bindings.contains_key(f.as_str()) {
                    let accessor = field_resolver.accessor(f, "php", &format!("${result_var}"));
                    let var_name = f.to_lower_camel_case();
                    fields_array_bindings.insert(f.clone(), (var_name, accessor));
                }
            }
        }
    }

    // Generate field binding lines (e.g., $chunks = $result->getChunks();)
    // Every collected array-binding accessor needs its $var emitted; the prior
    // hardcoded allowlist ("chunks"/"imports"/"structure") silently dropped
    // bindings like $choices0MessageToolCalls and $segments, leaving
    // assertions that reference them to fail with "Undefined variable".
    // BTreeMap iteration is sorted-by-key, so this loop is deterministic.
    let mut field_bindings = String::new();
    for (var_name, accessor) in fields_array_bindings.values() {
        field_bindings.push_str(&format!("        ${} = {};\n", var_name, accessor));
    }

    // Render assertions_body
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        assertions::render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            call_config.result_is_array,
            &fields_array_bindings,
            is_streaming,
        );
    }

    // A fixture whose only assertion is `not_error` (streaming or not), or
    // whose resolvable field assertions all rendered as "skipped" comments,
    // leaves assertions_body with no executable statement. Fall back to a
    // real assertion instead of a vacuous test body.
    apply_vacuous_assertion_fallback(&mut assertions_body, is_streaming, expects_error, result_var);
    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, "php", &fixture.id, &fixture.assertions);

    let error_test_body = if expects_error {
        render_error_test_body(
            &setup_lines,
            &call_expr,
            crate::e2e::codegen::declared_error_value(fixture),
        )
    } else {
        String::new()
    };

    let rendered = crate::e2e::template_env::render(
        "php/test_method.jinja",
        minijinja::context! {
            method_name => method_name,
            description => description,
            client_factory => client_factory,
            setup_lines => setup_lines,
            expects_error => expects_error,
            error_test_body => error_test_body,
            skip_test => fixture.assertions.is_empty(),
            call_expr => call_expr,
            result_var => result_var,
            collect_snippet => collect_snippet,
            field_bindings => field_bindings,
            assertions_body => assertions_body,
            teardown_block => teardown_block,
        },
    );
    out.push_str(&rendered);
}

#[cfg(test)]
mod vacuous_assertion_fallback_tests {
    use super::{apply_vacuous_assertion_fallback, has_executable_assertion};

    #[test]
    fn has_executable_assertion_is_false_for_comment_only_body() {
        let body = "        // skipped: field 'foo' not available on result type\n";
        assert!(
            !has_executable_assertion(body),
            "comment-only body must not count as asserting"
        );
    }

    #[test]
    fn has_executable_assertion_is_false_for_empty_body() {
        assert!(!has_executable_assertion(""));
        assert!(!has_executable_assertion("   \n  \n"));
    }

    #[test]
    fn has_executable_assertion_is_true_when_a_real_statement_is_present() {
        let body =
            "        // skipped: field 'foo' not available on result type\n        $this->assertTrue($result->ok);\n";
        assert!(
            has_executable_assertion(body),
            "a real assertTrue line must count as asserting"
        );
    }

    /// Regression test for the not_error-only vacuous-test defect: a fixture whose
    /// only assertion is `not_error` renders an empty assertions_body. The fallback
    /// must emit a real assertion, never `expectNotToPerformAssertions()`.
    #[test]
    fn not_error_only_body_gets_a_real_assertion_not_a_framework_suppression() {
        let mut body = String::new();
        apply_vacuous_assertion_fallback(&mut body, false, false, "result");
        assert_eq!(body, "        $this->assertNotNull($result);\n");
        assert!(!body.contains("expectNotToPerformAssertions"));
    }

    #[test]
    fn comment_only_body_also_gets_a_real_assertion() {
        let mut body = "        // skipped: field 'chunks' not available on result type\n".to_string();
        apply_vacuous_assertion_fallback(&mut body, false, false, "result");
        assert!(
            body.contains("$this->assertNotNull($result);"),
            "a comment-only body must still get a real fallback assertion, got: {body}"
        );
        assert!(!body.contains("expectNotToPerformAssertions"));
    }

    #[test]
    fn streaming_vacuous_body_asserts_on_drained_chunks_not_result() {
        let mut body = String::new();
        apply_vacuous_assertion_fallback(&mut body, true, false, "result");
        assert_eq!(
            body,
            "        $this->assertTrue(is_array($chunks), 'expected drained chunks list');\n"
        );
    }

    #[test]
    fn error_expecting_fixture_is_left_untouched() {
        let mut body = String::new();
        apply_vacuous_assertion_fallback(&mut body, false, true, "result");
        assert!(
            body.is_empty(),
            "error-expecting fixtures render their own error_test_body, not this fallback"
        );
    }

    #[test]
    fn body_with_a_real_assertion_is_left_untouched() {
        let mut body = "        $this->assertEquals(1, $result->count);\n".to_string();
        let original = body.clone();
        apply_vacuous_assertion_fallback(&mut body, false, false, "result");
        assert_eq!(
            body, original,
            "a fixture with a real assertion must not get an extra fallback line"
        );
    }

    /// Regression test for alef task #81: PHP's "skipped: field not available" comment
    /// text must survive as the exact marker the shared `fail_on_unavailable_field_markers`
    /// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
    /// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
    /// `unavailable_field_marker_tests`; this test only pins the marker text PHP emits.
    #[test]
    fn skip_comment_carries_the_marker_the_strict_mode_matches_on() {
        let mut body = "        // skipped: field 'nonexistent_field' not available on result type\n".to_string();
        // Confirm the comment alone doesn't get treated as a real assertion.
        assert!(!has_executable_assertion(&body));
        apply_vacuous_assertion_fallback(&mut body, false, false, "result");
        assert!(
            body.contains("field 'nonexistent_field' not available on result type"),
            "got: {body}"
        );
    }
}

#[cfg(test)]
mod error_test_body_tests {
    use super::render_error_test_body;

    #[test]
    fn no_declared_value_is_byte_identical_to_expect_exception() {
        let body = render_error_test_body(
            &["$options = new Options();".to_string()],
            "Client::create($options)",
            None,
        );
        assert_eq!(
            body,
            "        $this->expectException(\\Exception::class);\n        $options = new Options();\n        Client::create($options);"
        );
    }

    #[test]
    fn declared_value_adds_message_or_class_name_check() {
        let body = render_error_test_body(&[], "Client::create()", Some("BadRequest"));
        assert_eq!(
            body,
            "        try {\n            Client::create();\n            $this->fail('Expected an exception to be thrown');\n        } catch (\\Exception $e) {\n            $this->assertTrue(preg_match('/BadRequest/', $e->getMessage()) === 1 || preg_match('/BadRequest/', get_class($e)) === 1, 'expected exception message or class name to match BadRequest');\n        }"
        );
    }

    #[test]
    fn declared_value_with_regex_metacharacters_is_escaped() {
        let body = render_error_test_body(&[], "Client::create()", Some("field.name[0]"));
        assert!(
            body.contains("'/field\\\\.name\\\\[0\\\\]/'"),
            "expected escaped PCRE literal, got: {body}"
        );
    }

    /// Regression test for a real bug: embedding the already-quoted PCRE literal
    /// (`'/max_depth/'`) directly inside the message's own single-quoted string
    /// closes that string early and leaves a bare `/max_depth/` followed by `''`
    /// — a PHP syntax error. Assert the message argument is exactly one balanced
    /// single-quoted PHP string, not merely that it contains the right substrings
    /// (string-content assertions alone did not catch this).
    #[test]
    fn declared_value_message_argument_is_a_well_formed_php_single_quoted_literal() {
        let body = render_error_test_body(&[], "Client::create()", Some("max_depth"));
        let message_start = body
            .find("'expected exception message or class name to match")
            .expect("message argument must be present");
        let message = &body[message_start..];
        // The message argument runs up to the closing `)` of assertTrue(...); — find
        // the terminating `'` that precedes it.
        let message_end = message
            .find("');")
            .expect("message argument must be closed and followed by ');'");
        let literal = &message[..=message_end];
        // Strip PHP's `\'` escape sequences before counting quote boundaries, so an
        // escaped quote inside the literal doesn't look like a premature terminator.
        let quote_count = literal.replace("\\'", "").matches('\'').count();
        assert_eq!(
            quote_count, 2,
            "message argument must be a single balanced single-quoted PHP string, got: {literal}"
        );
        assert!(
            literal.contains("max_depth"),
            "failure message should still name the expected pattern: {literal}"
        );
        assert!(
            !literal.contains("'/max_depth/'"),
            "must not embed the pre-quoted PCRE literal inside the message string: {literal}"
        );
    }

    #[test]
    fn declared_value_with_single_quote_is_escaped_in_message() {
        let body = render_error_test_body(&[], "Client::create()", Some("it's bad"));
        assert!(
            body.contains("match it\\'s bad')"),
            "expected escaped single quote in message, got: {body}"
        );
    }
}

#[cfg(test)]
mod visitor_options_type_tests {
    use super::render_test_method;
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{CallbackAction, Fixture, VisitorSpec};
    use std::collections::{HashMap, HashSet};

    /// Regression test for alef task #86: a `visitor` fixture whose options type resolves
    /// from neither `[e2e.call]` nor any `[[crates.trait_bridges]]` entry used to re-render
    /// `php/test_method.jinja` with `skip_test => true` — byte-identical to the output the
    /// sanctioned `skip_languages` branch emits. A config failure was therefore
    /// indistinguishable from an author-declared skip, and the emitted PHPUnit suite went
    /// green while exercising none of the visitor behavior it claimed. It must now fail at
    /// generation time, naming the fixture and the missing options type — mirroring
    /// `c/assertions.rs` and `kotlin/args.rs`, which already refuse to emit for an
    /// unresolvable trait bridge.
    #[test]
    #[should_panic(expected = "PHP e2e generator: fixture `visitor_smoke` declares a `visitor`")]
    fn visitor_fixture_without_trait_bridge_options_type_fails_loudly_instead_of_emitting_a_skip() {
        let fixture = Fixture {
            id: "visitor_smoke".into(),
            description: "Visitor smoke".into(),
            visitor: Some(VisitorSpec {
                callbacks: [("visit_element".to_string(), CallbackAction::Skip)]
                    .into_iter()
                    .collect(),
            }),
            ..Fixture::default()
        };
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "convert".into();
        e2e_config.call.result_var = "result".into();

        // No `[[crates.trait_bridges]]` entries declared — nothing supplies an `options_type`.
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        let mut trait_bridge_imports: Vec<String> = Vec::new();
        render_test_method(
            &mut out,
            &fixture,
            &e2e_config,
            "php",
            "Sample",
            "SampleClient",
            &[],
            &HashSet::new(),
            &HashMap::new(),
            false,
            None,
            "",
            &[],
            "",
            &config,
            &mut trait_bridge_imports,
        );
    }
}
