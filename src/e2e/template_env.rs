use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
    (
        "kotlin/not_empty_assertion.kt.jinja",
        include_str!("templates/kotlin/not_empty_assertion.kt.jinja"),
    ),
    (
        "swift/not_empty_assertion.swift.jinja",
        include_str!("templates/swift/not_empty_assertion.swift.jinja"),
    ),
    (
        "swift/void_not_error_sync.jinja",
        include_str!("templates/swift/void_not_error_sync.jinja"),
    ),
    (
        "swift/void_not_error_async.jinja",
        include_str!("templates/swift/void_not_error_async.jinja"),
    ),
    (
        "kotlin_android/excluded_fixtures.kt.jinja",
        include_str!("templates/kotlin_android/excluded_fixtures.kt.jinja"),
    ),
    (
        "python/snippet_body.py.jinja",
        include_str!("templates/python/snippet_body.py.jinja"),
    ),
    (
        "python/docs_file_expression.py.jinja",
        include_str!("templates/python/docs_file_expression.py.jinja"),
    ),
    (
        "python/call_statement.py.jinja",
        include_str!("templates/python/call_statement.py.jinja"),
    ),
    (
        "rust/snippet_body.rs.jinja",
        include_str!("templates/rust/snippet_body.rs.jinja"),
    ),
    (
        "rust/docs_file_read.rs.jinja",
        include_str!("templates/rust/docs_file_read.rs.jinja"),
    ),
    (
        "rust/docs_json_file.rs.jinja",
        include_str!("templates/rust/docs_json_file.rs.jinja"),
    ),
    (
        "rust/mock_server_fixture.rs.jinja",
        include_str!("templates/rust/mock_server_fixture.rs.jinja"),
    ),
    (
        "typescript/typed_binding.jinja",
        include_str!("templates/typescript/typed_binding.jinja"),
    ),
    (
        "typescript/docs_file_expression.jinja",
        include_str!("templates/typescript/docs_file_expression.jinja"),
    ),
    (
        "typescript/docs_file_assignment.jinja",
        include_str!("templates/typescript/docs_file_assignment.jinja"),
    ),
    (
        "typescript/builder_iife.jinja",
        include_str!("templates/typescript/builder_iife.jinja"),
    ),
    (
        "snippets/file.md.jinja",
        include_str!("templates/snippets/file.md.jinja"),
    ),
    ("c/snippet_body.jinja", include_str!("templates/c/snippet_body.jinja")),
    (
        "c/trait_bridge_snippet.jinja",
        include_str!("templates/c/trait_bridge_snippet.jinja"),
    ),
    (
        "c/test_skip_if_env_missing.jinja",
        include_str!("templates/c/test_skip_if_env_missing.jinja"),
    ),
    (
        "c/test_pass_if_null.jinja",
        include_str!("templates/c/test_pass_if_null.jinja"),
    ),
    ("c/typed_handle.jinja", include_str!("templates/c/typed_handle.jinja")),
    (
        "c/managed_handle_create.jinja",
        include_str!("templates/c/managed_handle_create.jinja"),
    ),
    (
        "c/typed_handle_free.jinja",
        include_str!("templates/c/typed_handle_free.jinja"),
    ),
    (
        "c/snippet_void_call.jinja",
        include_str!("templates/c/snippet_void_call.jinja"),
    ),
    (
        "c/snippet_status_call.jinja",
        include_str!("templates/c/snippet_status_call.jinja"),
    ),
    (
        "python/pyproject.toml.jinja",
        include_str!("templates/python/pyproject.toml.jinja"),
    ),
    (
        "python/conftest.py.jinja",
        include_str!("templates/python/conftest.py.jinja"),
    ),
    (
        "python/assertion.jinja",
        include_str!("templates/python/assertion.jinja"),
    ),
    (
        "python/visitor_method.jinja",
        include_str!("templates/python/visitor_method.jinja"),
    ),
    (
        "python/test_file.jinja",
        include_str!("templates/python/test_file.jinja"),
    ),
    (
        "python/test_function.jinja",
        include_str!("templates/python/test_function.jinja"),
    ),
    (
        "python/test_smoke.py.jinja",
        include_str!("templates/python/test_smoke.py.jinja"),
    ),
    ("ruby/Gemfile.jinja", include_str!("templates/ruby/Gemfile.jinja")),
    (
        "ruby/snippet_body.jinja",
        include_str!("templates/ruby/snippet_body.jinja"),
    ),
    (
        "ruby/http_snippet.jinja",
        include_str!("templates/ruby/http_snippet.jinja"),
    ),
    (
        "ruby/rubocop.yml.jinja",
        include_str!("templates/ruby/rubocop.yml.jinja"),
    ),
    ("ruby/assertion.jinja", include_str!("templates/ruby/assertion.jinja")),
    (
        "ruby/visitor_method.jinja",
        include_str!("templates/ruby/visitor_method.jinja"),
    ),
    ("ruby/test_file.jinja", include_str!("templates/ruby/test_file.jinja")),
    (
        "ruby/test_function.jinja",
        include_str!("templates/ruby/test_function.jinja"),
    ),
    ("ruby/http_test.jinja", include_str!("templates/ruby/http_test.jinja")),
    (
        "ruby/http_test_close.jinja",
        include_str!("templates/ruby/http_test_close.jinja"),
    ),
    (
        "ruby/http_request.jinja",
        include_str!("templates/ruby/http_request.jinja"),
    ),
    (
        "ruby/http_test_body.jinja",
        include_str!("templates/ruby/http_test_body.jinja"),
    ),
    (
        "ruby/http_101_skip.jinja",
        include_str!("templates/ruby/http_101_skip.jinja"),
    ),
    (
        "ruby/app_harness.rb.jinja",
        include_str!("templates/ruby/app_harness.rb.jinja"),
    ),
    (
        "ruby/http_test_sut.jinja",
        include_str!("templates/ruby/http_test_sut.jinja"),
    ),
    (
        "ruby/spec_helper_mock_server.rb.jinja",
        include_str!("templates/ruby/spec_helper_mock_server.rb.jinja"),
    ),
    ("r/description.jinja", include_str!("templates/r/description.jinja")),
    ("r/snippet_body.jinja", include_str!("templates/r/snippet_body.jinja")),
    (
        "r/visitor_method.jinja",
        include_str!("templates/r/visitor_method.jinja"),
    ),
    ("r/test_runner.jinja", include_str!("templates/r/test_runner.jinja")),
    ("r/test_case.jinja", include_str!("templates/r/test_case.jinja")),
    ("r/test_file.jinja", include_str!("templates/r/test_file.jinja")),
    (
        "r/setup_fixtures.jinja",
        include_str!("templates/r/setup_fixtures.jinja"),
    ),
    (
        "r/synthetic_assertion.jinja",
        include_str!("templates/r/synthetic_assertion.jinja"),
    ),
    (
        "php/composer.json.jinja",
        include_str!("templates/php/composer.json.jinja"),
    ),
    ("php/phpunit.xml.jinja", include_str!("templates/php/phpunit.xml.jinja")),
    (
        "php/http_snippet.jinja",
        include_str!("templates/php/http_snippet.jinja"),
    ),
    (
        "php/bootstrap.php.jinja",
        include_str!("templates/php/bootstrap.php.jinja"),
    ),
    ("php/assertion.jinja", include_str!("templates/php/assertion.jinja")),
    ("php/test_file.jinja", include_str!("templates/php/test_file.jinja")),
    (
        "php/snippet_body.jinja",
        include_str!("templates/php/snippet_body.jinja"),
    ),
    ("php/typed_dto.jinja", include_str!("templates/php/typed_dto.jinja")),
    ("php/test_method.jinja", include_str!("templates/php/test_method.jinja")),
    (
        "php/visitor_method.jinja",
        include_str!("templates/php/visitor_method.jinja"),
    ),
    (
        "php/http_test_open.jinja",
        include_str!("templates/php/http_test_open.jinja"),
    ),
    (
        "php/http_test_close.jinja",
        include_str!("templates/php/http_test_close.jinja"),
    ),
    (
        "php/http_request.jinja",
        include_str!("templates/php/http_request.jinja"),
    ),
    (
        "php/http_assertions.jinja",
        include_str!("templates/php/http_assertions.jinja"),
    ),
    (
        "php/http_test_skip_101.jinja",
        include_str!("templates/php/http_test_skip_101.jinja"),
    ),
    (
        "php/synthetic_assertion.jinja",
        include_str!("templates/php/synthetic_assertion.jinja"),
    ),
    (
        "php/app_harness.php.jinja",
        include_str!("templates/php/app_harness.php.jinja"),
    ),
    ("csharp/csproj.jinja", include_str!("templates/csharp/csproj.jinja")),
    (
        "csharp/assertion.jinja",
        include_str!("templates/csharp/assertion.jinja"),
    ),
    (
        "csharp/visitor_method.jinja",
        include_str!("templates/csharp/visitor_method.jinja"),
    ),
    (
        "csharp/test_file.jinja",
        include_str!("templates/csharp/test_file.jinja"),
    ),
    (
        "csharp/test_method.jinja",
        include_str!("templates/csharp/test_method.jinja"),
    ),
    (
        "csharp/http_test_open.jinja",
        include_str!("templates/csharp/http_test_open.jinja"),
    ),
    (
        "csharp/http_test_close.jinja",
        include_str!("templates/csharp/http_test_close.jinja"),
    ),
    (
        "csharp/http_request.jinja",
        include_str!("templates/csharp/http_request.jinja"),
    ),
    (
        "csharp/http_test_body.jinja",
        include_str!("templates/csharp/http_test_body.jinja"),
    ),
    (
        "csharp/app_harness.cs.jinja",
        include_str!("templates/csharp/app_harness.cs.jinja"),
    ),
    (
        "csharp/test_setup_mock_server.cs.jinja",
        include_str!("templates/csharp/test_setup_mock_server.cs.jinja"),
    ),
    (
        "csharp/snippet_body.jinja",
        include_str!("templates/csharp/snippet_body.jinja"),
    ),
    (
        "kotlin/snippet_body.jinja",
        include_str!("templates/kotlin/snippet_body.jinja"),
    ),
    (
        "kotlin/docs_file_read.jinja",
        include_str!("templates/kotlin/docs_file_read.jinja"),
    ),
    (
        "kotlin/snippet_json_object_setup.jinja",
        include_str!("templates/kotlin/snippet_json_object_setup.jinja"),
    ),
    (
        "ruby/docs_file_read.jinja",
        include_str!("templates/ruby/docs_file_read.jinja"),
    ),
    (
        "elixir/docs_file_read.jinja",
        include_str!("templates/elixir/docs_file_read.jinja"),
    ),
    (
        "r/docs_file_read.jinja",
        include_str!("templates/r/docs_file_read.jinja"),
    ),
    (
        "swift/docs_file_read.jinja",
        include_str!("templates/swift/docs_file_read.jinja"),
    ),
    (
        "swift/docs_file_json.jinja",
        include_str!("templates/swift/docs_file_json.jinja"),
    ),
    (
        "zig/docs_file_read.jinja",
        include_str!("templates/zig/docs_file_read.jinja"),
    ),
    (
        "zig/docs_file_json.jinja",
        include_str!("templates/zig/docs_file_json.jinja"),
    ),
    (
        "zig/docs_json_replace.jinja",
        include_str!("templates/zig/docs_json_replace.jinja"),
    ),
    (
        "c/docs_json_base.jinja",
        include_str!("templates/c/docs_json_base.jinja"),
    ),
    (
        "c/docs_file_replace.jinja",
        include_str!("templates/c/docs_file_replace.jinja"),
    ),
    (
        "c/docs_json_cleanup.jinja",
        include_str!("templates/c/docs_json_cleanup.jinja"),
    ),
    ("java/pom.xml.jinja", include_str!("templates/java/pom.xml.jinja")),
    ("java/test_file.jinja", include_str!("templates/java/test_file.jinja")),
    (
        "java/test_method.jinja",
        include_str!("templates/java/test_method.jinja"),
    ),
    ("java/assertion.jinja", include_str!("templates/java/assertion.jinja")),
    (
        "java/visitor_method.jinja",
        include_str!("templates/java/visitor_method.jinja"),
    ),
    (
        "java/http_test_open.jinja",
        include_str!("templates/java/http_test_open.jinja"),
    ),
    (
        "java/http_test_close.jinja",
        include_str!("templates/java/http_test_close.jinja"),
    ),
    (
        "java/http_request.jinja",
        include_str!("templates/java/http_request.jinja"),
    ),
    (
        "java/http_assertions.jinja",
        include_str!("templates/java/http_assertions.jinja"),
    ),
    (
        "java/http_test_skip_101.jinja",
        include_str!("templates/java/http_test_skip_101.jinja"),
    ),
    (
        "java/synthetic_assertion.jinja",
        include_str!("templates/java/synthetic_assertion.jinja"),
    ),
    (
        "java/harness_main.jinja",
        include_str!("templates/java/harness_main.jinja"),
    ),
    (
        "java/snippet_body.jinja",
        include_str!("templates/java/snippet_body.jinja"),
    ),
    (
        "java/snippet_json_object_setup.jinja",
        include_str!("templates/java/snippet_json_object_setup.jinja"),
    ),
    (
        "java/MockServerListener.java.jinja",
        include_str!("templates/java/MockServerListener.java.jinja"),
    ),
    (
        "typescript/package.json.jinja",
        include_str!("templates/typescript/package.json.jinja"),
    ),
    (
        "typescript/tsconfig.jinja",
        include_str!("templates/typescript/tsconfig.jinja"),
    ),
    (
        "typescript/vitest.config.ts.jinja",
        include_str!("templates/typescript/vitest.config.ts.jinja"),
    ),
    (
        "typescript/setup.ts.jinja",
        include_str!("templates/typescript/setup.ts.jinja"),
    ),
    (
        "typescript/globalSetup.ts.jinja",
        include_str!("templates/typescript/globalSetup.ts.jinja"),
    ),
    (
        "typescript/assertion.jinja",
        include_str!("templates/typescript/assertion.jinja"),
    ),
    (
        "typescript/synthetic_assertion.jinja",
        include_str!("templates/typescript/synthetic_assertion.jinja"),
    ),
    (
        "typescript/visitor_method.jinja",
        include_str!("templates/typescript/visitor_method.jinja"),
    ),
    (
        "typescript/test_file.jinja",
        include_str!("templates/typescript/test_file.jinja"),
    ),
    (
        "typescript/test_function.jinja",
        include_str!("templates/typescript/test_function.jinja"),
    ),
    (
        "typescript/snippet_body.jinja",
        include_str!("templates/typescript/snippet_body.jinja"),
    ),
    (
        "typescript/helpers.jinja",
        include_str!("templates/typescript/helpers.jinja"),
    ),
    (
        "typescript/http_test.jinja",
        include_str!("templates/typescript/http_test.jinja"),
    ),
    (
        "typescript/cache_isolation_setup.jinja",
        include_str!("templates/typescript/cache_isolation_setup.jinja"),
    ),
    (
        "typescript/http_test_skip_101.jinja",
        include_str!("templates/typescript/http_test_skip_101.jinja"),
    ),
    (
        "typescript/app_harness.mjs.jinja",
        include_str!("templates/typescript/app_harness.mjs.jinja"),
    ),
    (
        "typescript/globalSetup_server.ts.jinja",
        include_str!("templates/typescript/globalSetup_server.ts.jinja"),
    ),
    ("go/snippet_body.jinja", include_str!("templates/go/snippet_body.jinja")),
    ("go/dto_literal.jinja", include_str!("templates/go/dto_literal.jinja")),
    (
        "go/read_file_helper.jinja",
        include_str!("templates/go/read_file_helper.jinja"),
    ),
    (
        "go/empty_dto_literal.jinja",
        include_str!("templates/go/empty_dto_literal.jinja"),
    ),
    (
        "python/http_test.jinja",
        include_str!("templates/python/http_test.jinja"),
    ),
    (
        "python/http_101_skip.jinja",
        include_str!("templates/python/http_101_skip.jinja"),
    ),
    (
        "python/app_harness.py.jinja",
        include_str!("templates/python/app_harness.py.jinja"),
    ),
    (
        "elixir/app_harness.exs.jinja",
        include_str!("templates/elixir/app_harness.exs.jinja"),
    ),
    (
        "elixir/snippet_body.jinja",
        include_str!("templates/elixir/snippet_body.jinja"),
    ),
    (
        "elixir/http_snippet.jinja",
        include_str!("templates/elixir/http_snippet.jinja"),
    ),
    (
        "elixir/test_helper_server.exs.jinja",
        include_str!("templates/elixir/test_helper_server.exs.jinja"),
    ),
    (
        "elixir/test_helper_mock_server.exs.jinja",
        include_str!("templates/elixir/test_helper_mock_server.exs.jinja"),
    ),
    (
        "dart/app_harness.dart.jinja",
        include_str!("templates/dart/app_harness.dart.jinja"),
    ),
    (
        "dart/snippet_body.jinja",
        include_str!("templates/dart/snippet_body.jinja"),
    ),
    ("dart/typed_dto.jinja", include_str!("templates/dart/typed_dto.jinja")),
    (
        "dart/http_snippet.jinja",
        include_str!("templates/dart/http_snippet.jinja"),
    ),
    (
        "dart/void_not_error_call.jinja",
        include_str!("templates/dart/void_not_error_call.jinja"),
    ),
    (
        "swift/app_harness.swift.jinja",
        include_str!("templates/swift/app_harness.swift.jinja"),
    ),
    (
        "swift/snippet_body.jinja",
        include_str!("templates/swift/snippet_body.jinja"),
    ),
    (
        "wasm/package.json.jinja",
        include_str!("templates/wasm/package.json.jinja"),
    ),
    (
        "wasm/vitest.config.ts.jinja",
        include_str!("templates/wasm/vitest.config.ts.jinja"),
    ),
    (
        "wasm/globalSetup.ts.jinja",
        include_str!("templates/wasm/globalSetup.ts.jinja"),
    ),
    ("wasm/tsconfig.jinja", include_str!("templates/wasm/tsconfig.jinja")),
    (
        "wasm/visitor_method.jinja",
        include_str!("templates/wasm/visitor_method.jinja"),
    ),
    (
        "zig/json_assertion.jinja",
        include_str!("templates/zig/json_assertion.jinja"),
    ),
    (
        "zig/snippet_body.jinja",
        include_str!("templates/zig/snippet_body.jinja"),
    ),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    for (name, src) in TEMPLATES {
        env.add_template(name, src).expect("built-in template is valid");
    }
    env
}

pub(crate) fn render(template_name: &str, ctx: minijinja::Value) -> String {
    let rendered = make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"));
    crate::core::keep_marker::strip_keep_markers(&rendered)
}

/// `not_empty` is the one assertion that silently degrades into a no-op: a check that
/// stringifies before measuring, or that leans on the host language's truthiness, reads
/// as coverage while passing on empty data. Every emitter below must reject an empty
/// collection and an empty string, and must still accept a legitimate `0` / `0.0` / `false`.
#[cfg(test)]
mod not_empty_tests {
    use super::render;

    /// Compare emitted code by token sequence so a template's cosmetic indentation
    /// (which minijinja's whitespace control also influences) cannot mask a behaviour change.
    fn code(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn not_empty_for_python_rejects_empty_sized_values_but_accepts_zero() {
        let rendered = render(
            "python/assertion.jinja",
            minijinja::context! { assertion_type => "not_empty", field_access => "result.content" },
        );
        assert_eq!(
            rendered.trim(),
            "assert result.content is not None and (not hasattr(result.content, \"__len__\") \
             or len(result.content) > 0)  # noqa: S101"
        );
    }

    #[test]
    fn not_empty_for_php_arrays_measures_the_element_count() {
        let rendered = render(
            "php/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "$result->chunks",
                field_is_array => true,
            },
        );
        assert_eq!(
            rendered.trim(),
            "$this->assertGreaterThan(0, count($result->chunks ?? []), 'expected non-empty value');"
        );
    }

    #[test]
    fn not_empty_for_php_scalars_rejects_empty_string_but_accepts_zero() {
        let rendered = render(
            "php/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "$result->content",
                field_is_array => false,
            },
        );
        // assertNotEmpty() routes through empty(), which would reject 0, 0.0, "0" and false.
        assert!(!rendered.contains("assertNotEmpty"), "got: {rendered}");
        assert_eq!(
            rendered.trim(),
            "$this->assertNotSame('', $result->content ?? '', 'expected non-empty value');"
        );
    }

    #[test]
    fn not_empty_for_ruby_asks_the_value_not_its_string_form() {
        let rendered = render(
            "ruby/assertion.jinja",
            minijinja::context! { assertion_type => "not_empty", field_expr => "result.content" },
        );
        // `[].to_s` is "[]" — a non-empty string — so the old form could never fail.
        assert!(!rendered.contains(".to_s"), "got: {rendered}");
        assert_eq!(
            rendered.trim(),
            "expect(result.content.respond_to?(:empty?) ? !result.content.empty? : !result.content.nil?).to be(true)"
        );
    }

    #[test]
    fn not_empty_for_java_measures_collections_instead_of_their_string_form() {
        let rendered = render(
            "java/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "java.util.Optional.ofNullable(result.content())",
            },
        );
        assert!(!rendered.contains("toString"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "assertTrue(java.util.Optional.ofNullable(result.content()).filter(value -> switch ((Object) value) {
                case CharSequence text -> !text.isEmpty();
                case java.util.Collection<?> items -> !items.isEmpty();
                case java.util.Map<?, ?> entries -> !entries.isEmpty();
                default -> true;
            }).isPresent(), \"expected non-empty value\");"
            )
        );
    }

    #[test]
    fn not_empty_for_java_concrete_fields_still_use_is_empty() {
        let rendered = render(
            "java/assertion.jinja",
            minijinja::context! { assertion_type => "not_empty", field_expr => "result.content()" },
        );
        assert_eq!(
            code(&rendered),
            code("assertFalse(result.content().isEmpty(), \"expected non-empty value\");")
        );
    }

    #[test]
    fn not_empty_for_csharp_pattern_matches_instead_of_stringifying() {
        let rendered = render(
            "csharp/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "result.Content",
                field_needs_json_serialize => false,
                skipped_reason => "",
            },
        );
        // `.ToString()` on a struct yields the type name, and the old `?.` form did not
        // compile for a non-nullable value type.
        assert!(!rendered.contains("ToString"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "Assert.True(((object?)result.Content) switch
            {
                null => false,
                string text => text.Length > 0,
                System.Collections.ICollection items => items.Count > 0,
                _ => true,
            }, \"expected non-empty value\");"
            )
        );
    }

    #[test]
    fn not_empty_for_csharp_collections_still_use_assert_not_empty() {
        let rendered = render(
            "csharp/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "result.Chunks",
                field_needs_json_serialize => true,
                skipped_reason => "",
            },
        );
        assert_eq!(code(&rendered), code("Assert.NotEmpty(result.Chunks);"));
    }

    #[test]
    fn not_empty_for_zig_json_rejects_empty_array_and_empty_string() {
        let rendered = render(
            "zig/json_assertion.jinja",
            minijinja::context! { assertion_type => "not_empty", field_expr => "_content" },
        );
        // `!= .null` accepted an empty array and an empty string.
        assert!(!rendered.contains("!= .null"), "got: {rendered}");
        assert_eq!(
            code(&rendered),
            code(
                "{
                const _ne = _content;
                try testing.expect(switch (_ne) {
                    .null => false,
                    .string => |_s| _s.len > 0,
                    .array => |_a| _a.items.len > 0,
                    .object => |_o| _o.count() > 0,
                    else => true,
                });
            }"
            )
        );
    }

    #[test]
    fn not_empty_for_typescript_sizes_strings_and_arrays_and_accepts_zero() {
        let rendered = render(
            "typescript/assertion.jinja",
            minijinja::context! {
                assertion_type => "not_empty",
                field_expr => "result.content",
                field_is_optional => false,
            },
        );
        assert_eq!(
            code(&rendered),
            code(
                "{
                const _v = result.content;
                if (typeof _v === \"string\" || Array.isArray(_v)) {
                    expect(_v.length).toBeGreaterThan(0);
                } else {
                    expect(_v).toBeDefined();
                    expect(_v).not.toBeNull();
                }
            }"
            )
        );
    }
}

#[cfg(test)]
mod template_registration_tests {
    use super::TEMPLATES;
    use std::collections::HashSet;
    use std::path::Path;

    /// `go/harness_main.go.jinja` is read by its own private `minijinja::Environment` in
    /// `render_harness_main` (`src/e2e/codegen/go.rs`), via a local `include_str!` rather than
    /// through this shared `TEMPLATES` registry. That function is currently dead code — its own
    /// doc comment says the server-pattern harness is now emitted by a consumer `Extension` and
    /// alef no longer calls it, kept only pending a dead-code sweep — so the file is genuinely
    /// unreachable through `render()`, but deleting it would break `render_harness_main`'s
    /// `include_str!`, and that's an emitter-side change out of scope here. ~keep
    const ALLOWLISTED_UNREGISTERED: &[&str] = &["go/harness_main.go.jinja"];

    /// `render()` resolves names against `TEMPLATES`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into this array compiles fine
    /// (`include_str!` only runs for entries that are listed) and panics only once an
    /// emitter reaches it at generation time. Compare by content rather than by
    /// registered key: some backends register a file under a shortened or aliased name,
    /// which is fine, but every file's bytes must appear in `TEMPLATES` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/e2e/templates"));
        let registered_contents: HashSet<&str> = TEMPLATES.iter().map(|(_, content)| *content).collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.retain(|path| !ALLOWLISTED_UNREGISTERED.contains(&path.as_str()));
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATES: {unregistered:?}"
        );
    }

    fn collect_unregistered(
        root: &Path,
        dir: &Path,
        registered_contents: &HashSet<&str>,
        unregistered: &mut Vec<String>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read templates directory") {
            let entry = entry.expect("read templates directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect_unregistered(root, &path, registered_contents, unregistered);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jinja") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read template file");
            if !registered_contents.contains(content.as_str()) {
                let relative = path
                    .strip_prefix(root)
                    .expect("template path under templates root")
                    .to_str()
                    .expect("template path is valid UTF-8")
                    .replace('\\', "/");
                unregistered.push(relative);
            }
        }
    }
}

/// A synthetic assertion is appended to the output with a bare `push_str`, so the template
/// — not the caller — owns its line terminator. `trim_blocks` already eats the newline that
/// follows a block tag, so a branch that also closes with `{%- endif %}` renders a bare
/// fragment with no newline at either end, and whatever the next emitter writes lands on the
/// same physical line. That is harmless between two statements and destructive after a
/// comment, which swallows the statement that follows it. Observed in a downstream consumer's
/// generated PHP smoke suite, where two skip comments shared one physical line.
#[cfg(test)]
mod synthetic_assertion_line_discipline {
    use super::render;

    const KIND_KEYED_TEMPLATES: [&str; 3] = [
        "php/synthetic_assertion.jinja",
        "java/synthetic_assertion.jinja",
        "r/synthetic_assertion.jinja",
    ];

    fn skip_comment(template: &str) -> String {
        if template == "typescript/synthetic_assertion.jinja" {
            return render(
                template,
                minijinja::context! {
                    assertion_type => "unsupported_by_this_backend",
                    field_name => "metadata.format.excel.sheet_count",
                },
            );
        }
        render(
            template,
            minijinja::context! {
                assertion_kind => "skipped",
                assertion_type => "unsupported_by_this_backend",
                field_name => "metadata.format.excel.sheet_count",
            },
        )
    }

    fn every_template() -> Vec<&'static str> {
        let mut templates = KIND_KEYED_TEMPLATES.to_vec();
        templates.push("typescript/synthetic_assertion.jinja");
        templates
    }

    #[test]
    fn a_skip_comment_terminates_its_own_line() {
        for template in every_template() {
            let rendered = skip_comment(template);
            assert!(
                rendered.contains("skipped:"),
                "{template} rendered no skip comment: {rendered:?}"
            );
            assert!(
                rendered.ends_with('\n'),
                "{template} left its comment unterminated: {rendered:?}"
            );
            assert_eq!(
                rendered.matches('\n').count(),
                1,
                "{template} did not render exactly one line: {rendered:?}"
            );
        }
    }

    /// The defect is *concatenation*, so the assertion has to be made on two appended
    /// renders. Checking a single render in isolation is the vacuous version of this test:
    /// it passes whether or not the fragment can collide with its successor.
    #[test]
    fn two_appended_skip_comments_do_not_share_a_physical_line() {
        for template in every_template() {
            let mut out = String::new();
            out.push_str(&skip_comment(template));
            out.push_str(&skip_comment(template));
            assert_eq!(
                out.lines().count(),
                2,
                "{template} merged two appended assertions onto one line: {out:?}"
            );
        }
    }

    /// Positive control: the branches that emit real code must still emit it, unchanged
    /// apart from now owning their terminator.
    #[test]
    fn a_rendered_assertion_still_emits_its_statement_on_one_line() {
        let php = render(
            "php/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "$carry",
                field_name => "chunks_have_content",
            },
        );
        assert_eq!(php, "        $this->assertTrue($carry);\n");

        let java = render(
            "java/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "carry",
                field_name => "chunks_have_content",
            },
        );
        assert_eq!(java, "        assertTrue(carry, \"expected true\");\n");
    }

    /// A statement followed by a comment is the destructive ordering: without a terminator
    /// the comment starts on the statement's line, and every later emitter is commented out.
    #[test]
    fn a_comment_appended_after_a_statement_starts_its_own_line() {
        let mut out = String::new();
        out.push_str(&render(
            "php/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "chunks_content",
                assertion_type => "is_true",
                pred => "$carry",
                field_name => "chunks_have_content",
            },
        ));
        out.push_str(&skip_comment("php/synthetic_assertion.jinja"));

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "statement and comment shared a line: {out:?}");
        assert!(!lines[0].contains("//"), "the statement was commented out: {out:?}");
        assert!(
            lines[1].trim_start().starts_with("//"),
            "unexpected second line: {out:?}"
        );
    }
}
