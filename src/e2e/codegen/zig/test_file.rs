use super::args::build_args_and_setup;
use super::assertions::{assertion_emits_code, render_assertion, render_json_assertion};
use super::http::render_http_test_case;
use super::visitor::{emit_visitor_test_body, resolve_zig_visitor_call_symbols};
use super::*;
use crate::core::hash::{self, CommentStyle};

/// Close the error arm of the `if (call) |_| {..} else |_| {..}` shape and, when the fixture
/// declares an `error` value, actually compare it.
///
/// ~keep Zig sits on the same C ABI as the `c` backend, which reports a failure as
/// `set_last_error(alef_ffi_error_code(&e), &e.to_string())` — a Display message plus a numeric
/// taxonomy code. The generated binding exposes the message as `_last_error()` and dispatches the
/// code to a declared error-set member, so `@errorName` yields the variant name. Together they
/// reproduce the message-or-type-name disjunction `crate::e2e::codegen::declared_error_value`
/// documents. Before this, the declared value was discarded and every valued `error` assertion was
/// weakened to a bare "the call failed" check that could not tell one failure from another.
fn emit_declared_error_value_assertion(out: &mut String, declared: Option<&str>, module_name: &str, for_docs: bool) {
    // ~keep A documentation snippet is a `main`, not a `test`: `testing` is never bound in
    // `zig/snippet_body.jinja`, so `try testing.expect(..)` would not compile there, and the
    // snippet emitter rewrites the literal `else |_| {}` arm into a printing one. Snippet output
    // therefore stays byte-identical to before this change.
    let Some(declared) = declared.filter(|_| !for_docs) else {
        let _ = writeln!(out, "    }} else |_| {{}}");
        return;
    };
    let expected = escape_zig(declared);
    let _ = writeln!(out, "    }} else |_err| {{");
    let _ = writeln!(out, "        const _err_name = @errorName(_err);");
    let _ = writeln!(
        out,
        "        const _err_message: []const u8 = {module_name}._last_error() orelse \"\";"
    );
    let _ = writeln!(
        out,
        "        try testing.expect(std.mem.indexOf(u8, _err_message, \"{expected}\") != null or std.mem.indexOf(u8, _err_name, \"{expected}\") != null);"
    );
    let _ = writeln!(out, "    }}");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "const std = @import(\"std\");");
    let _ = writeln!(out, "const testing = std.testing;");
    let _ = writeln!(out, "const {module_name} = @import(\"{module_name}\");");
    let _ = writeln!(out);

    // Propagate the configured e2e environment to native code that reads it via getenv. Zig has no per-suite setup
    // hook, so each test body calls allow_private_network(). The managed environment does not reach libc, so push each
    // value through setenv. ~keep
    if !e2e_config.env.is_empty() {
        let _ = writeln!(
            out,
            "extern \"c\" fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;"
        );
        let _ = writeln!(out, "fn allow_private_network() void {{");
        let mut keys: Vec<&String> = e2e_config.env.keys().collect();
        keys.sort();
        for k in keys {
            let v = &e2e_config.env[k];
            let _ = writeln!(out, "    _ = setenv(\"{k}\", \"{v}\", 1);");
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.http.is_some() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_fn(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                module_name,
                ffi_prefix,
                config,
                type_defs,
                true,
                false,
            );
        }
        let _ = writeln!(out);
    }

    out
}

#[derive(Debug, Clone)]
struct ZigStreamingAdapterMetadata {
    owner_type: String,
    item_type: String,
    request_type: String,
    adapter_name: String,
}

fn resolve_zig_streaming_adapter(
    config: &ResolvedCrateConfig,
    function_name: &str,
) -> Option<ZigStreamingAdapterMetadata> {
    config
        .adapters
        .iter()
        .find(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.name == function_name)
        .and_then(|adapter| {
            Some(ZigStreamingAdapterMetadata {
                owner_type: adapter.owner_type.clone()?,
                item_type: adapter.item_type.clone()?,
                request_type: adapter
                    .request_type
                    .as_deref()
                    .and_then(|path| path.rsplit("::").next())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)?,
                adapter_name: adapter.name.clone(),
            })
        })
}

#[allow(clippy::too_many_arguments)]
fn render_test_fn(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    wrap_as_test: bool,
    for_docs: bool,
) {
    // Resolve per-fixture call config.
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
        e2e_config.effective_fields_method_calls(call_config),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;
    // Client factory: when set, the test instantiates a client object via
    // `module.factory_fn(...)` and calls methods on the instance rather than
    // calling top-level package functions directly.
    // Mirrors the go codegen pattern (go.rs:981-1028 / CallOverride.client_factory).
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // When `result_is_json_struct = true`, the Zig function returns `[]u8` JSON.
    // The test parses it with `std.json.parseFromSlice(std.json.Value, ...)` and
    // traverses the dynamic JSON object for field assertions.
    //
    // Client-factory methods on opaque handles always return JSON `[]u8` because
    // the zig backend serializes struct results via the FFI's `*_to_json` helper
    // (see alef-backend-zig/src/gen_bindings/opaque_handles.rs). Force the flag
    // on whenever a client_factory is in play so the test path parses the JSON
    // result rather than attempting direct field access on `[]u8`.
    //
    // Exception: when the call returns raw bytes (e.g. speech/file_content use the
    // FFI byte-buffer out-pointer shape and return `[]u8` audio/file bytes rather
    // than a serialised struct). Detect this by checking the call-level flag first
    // and then falling back to any per-language override that declares `result_is_bytes`.
    // The zig and C bindings share the same byte-buffer convention, so a C override
    // of `result_is_bytes = true` is a reliable proxy when no zig override exists.
    let call_result_is_bytes = call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_json_struct =
        !call_result_is_bytes && (call_overrides.is_some_and(|o| o.result_is_json_struct) || client_factory.is_some());

    // Whether the bare wrapper return type is `?T` (Optional). The zig backend
    // emits `?[]u8` for nullable JSON results and `?<Primitive>` for nullable
    // primitives, so assertions on the bare result must use null-checks rather
    // than `.len`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and fall back to the per-call language override for
    // backwards compatibility.
    let result_is_simple = call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple);

    // Whether the Zig wrapper returns an error union (`try` is required).
    //
    // The Zig backend nearly always returns an error union: any function with
    // string/path/json_object/bytes parameters must allocate a null-terminated
    // copy (→ `error{OutOfMemory}!T`), any fallible function (`returns_result`)
    // wraps a `DomainError||error{OutOfMemory}!T`, and any function whose return
    // type is a string/JSON/collection blob also needs heap allocation.
    //
    // The ONLY case where `try` is incorrect is a function that is:
    //   - genuinely infallible (no Rust Result<T,E>)
    //   - takes no allocating parameters (no string/path/bytes/json_object args)
    //   - returns a primitive directly (u64, bool, etc.)
    //
    // Rather than attempting to infer this from incomplete config information,
    // we default to emitting `try` and require an explicit opt-out:
    //
    //   [crates.e2e.calls.language_count.overrides.zig]
    //   returns_result = false
    //
    // Special case: functions named `unregister_*` always return error unions
    // (plugin trait unregister calls) and must always use `try`, regardless
    // of the `returns_result` override.
    //
    // This is safer than guessing wrong and producing un-compilable Zig.
    let call_returns_error_union =
        function_name.starts_with("unregister_") || call_overrides.and_then(|o| o.returns_result) != Some(false);

    let test_name = fixture.id.to_snake_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str, setup_needs_gpa) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        module_name,
        config,
        type_defs,
        fixture,
    );
    // Append per-call zig extra_args (e.g. `["null"]` for the trailing
    // optional `query` parameter on `list_files` / `list_batches`). Mirrors
    // the same mechanism used by go/python/swift codegen — zig's method
    // signatures require every optional positional argument to be supplied
    // explicitly, so the e2e config carries a per-language extras list.
    let extra_args = recipe.extra_args;
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // Pre-compute whether any assertion will emit code that references `result` /
    // `allocator`. Used to decide whether to emit the GPA allocator binding.
    let any_happy_emits_code = fixture
        .assertions
        .iter()
        .any(|a| assertion_emits_code(a, field_resolver));

    // Pre-compute streaming-virtual path conditions.
    let has_streaming_virtual_assertions = fixture.assertions.iter().any(|a| {
        a.field
            .as_ref()
            .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
    });
    let is_stream_fn = function_name.contains("stream");
    let streaming_adapter = if has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some() {
        resolve_zig_streaming_adapter(config, &function_name)
    } else {
        None
    };
    let uses_streaming_virtual_path =
        result_is_json_struct && has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some();
    // Whether the streaming-virtual path also parses JSON (for non-streaming assertions).
    let streaming_path_has_non_streaming = uses_streaming_virtual_path
        && fixture.assertions.iter().any(|a| {
            !a.field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
                && !matches!(a.assertion_type.as_str(), "not_error" | "error")
                && a.field
                    .as_ref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_valid_for_result(f))
        });

    if wrap_as_test {
        let _ = writeln!(out, "test \"{test_name}\" {{");
        let _ = writeln!(out, "    // {description}");
        if !e2e_config.env.is_empty() {
            let _ = writeln!(out, "    allow_private_network();");
        }
    }

    // Visitor fixtures bypass the high-level `convert(html, options)` wrapper
    // and inline the FFI sequence so we can attach the generated visitor callbacks
    // vtable to the options handle. The vtable is populated by per-fixture
    // C-callable thunks emitted by `zig_visitors::build_zig_visitor`.
    if let Some(visitor_spec) = &fixture.visitor {
        let html = fixture.input.get("html").and_then(|v| v.as_str()).unwrap_or_default();
        let options_value = fixture.input.get("options").cloned();
        let visitor_symbols = resolve_zig_visitor_call_symbols(call_config, &recipe, ffi_prefix);
        emit_visitor_test_body(
            out,
            &fixture.id,
            html,
            options_value.as_ref(),
            visitor_spec,
            module_name,
            &visitor_symbols,
            &fixture.assertions,
            expects_error,
            field_resolver,
            wrap_as_test,
        );
        if wrap_as_test {
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
        }
        return;
    }

    // Emit GPA allocator only when it will actually be used: setup lines that
    // need GPA allocation (mock_url), or a JSON-struct result path where the test
    // will call `std.json.parseFromSlice`. The binding is not needed for
    // error-only paths or tests with no field assertions.
    // Note: `bytes` arg setup uses c_allocator directly and does NOT require GPA.
    // For the streaming-virtual path, `allocator` is only needed if there are also
    // non-streaming assertions that require JSON parsing via parseFromSlice.
    let needs_gpa = setup_needs_gpa
        || streaming_path_has_non_streaming
        || (!uses_streaming_virtual_path && result_is_json_struct && !expects_error && any_happy_emits_code);
    if needs_gpa {
        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");
        let _ = writeln!(out);
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    // Client factory: when configured, instantiate a client object via the named
    // constructor function and call the method on the instance.
    // In test mode the client is pointed at MOCK_SERVER_URL/fixtures/<id> (mirrors
    // go.rs:981-1028). A documentation snippet is published verbatim to readers, so it
    // must never carry that harness wiring or a literal credential: it reads the real
    // credential from the environment and leaves `base_url` at the binding default,
    // matching what `java/snippet_body.jinja` and `csharp/snippet_body.jinja` emit. ~keep
    // When not configured, fall back to calling the top-level package function directly.
    let call_prefix = if let Some(factory) = client_factory {
        if for_docs {
            let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
            let _ = writeln!(
                out,
                "    const _api_key = std.c.getenv(\"{api_key_var}\") orelse return error.MissingApiKey;"
            );
            // The factory's second positional slot is base_url (matches the mock-server
            // arm below, which passes `_mock_url` there). A docs fixture that names
            // `docs.client.base_url` fills the slot; otherwise it stays `null` so the
            // binding falls back to its own default endpoint. ~keep
            let base_url_arg = match crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client()) {
                Some(url) => format!("\"{}\"", escape_zig(url)),
                None => "null".to_string(),
            };
            let _ = writeln!(
                out,
                "    var _client = try {module_name}.{factory}(std.mem.span(_api_key), {base_url_arg}, null, null, null);"
            );
        } else {
            let fixture_id = &fixture.id;
            let _ = writeln!(
                out,
                "    const _mock_url = try std.fmt.allocPrintSentinel(std.heap.c_allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}}, 0);"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_mock_url);");
            let _ = writeln!(
                out,
                "    var _client = try {module_name}.{factory}(\"test-key\", _mock_url, null, null, null);"
            );
        }
        let _ = writeln!(out, "    defer _client.free();");
        "_client".to_string()
    } else {
        module_name.to_string()
    };

    if expects_error {
        // Error-path test: the call must fail. `if (expr) |_| {...} else |_| {...}`
        // captures both arms of the error union explicitly, so an unexpected success
        // fails the test via `return error.TestUnexpectedResult`. The previous shape —
        // `... catch { try testing.expect(true); return; }` — could never fail: on
        // success the catch block simply never ran and execution fell through, and
        // `expect(true)` inside the catch was a tautology on error. ~keep
        //
        // The success arm discards its capture (`|_|`) rather than binding `result`,
        // since a fixture asserting `error` has nothing meaningful to check once the
        // call has already failed the test by succeeding.
        let _ = writeln!(out, "    if ({call_prefix}.{function_name}({args_str})) |_| {{");
        let _ = writeln!(out, "        return error.TestUnexpectedResult;");
        emit_declared_error_value_assertion(
            out,
            crate::e2e::codegen::declared_error_value(fixture),
            module_name,
            for_docs,
        );
        if !for_docs {
            crate::e2e::codegen::error_path_assertions::emit(out, fixture, "    // ", "zig");
        }
    } else if fixture.assertions.is_empty() {
        // No assertions: emit a call to verify compilation.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    } else {
        // Happy path: call and assert. Detect whether any assertion actually
        // emits code that references `result` (some — like `not_error` — emit
        // nothing) so we don't leave an unused local, which Zig 0.16 rejects.
        let any_emits_code = fixture
            .assertions
            .iter()
            .any(|a| assertion_emits_code(a, field_resolver));
        if call_result_is_bytes && client_factory.is_some() {
            // Bytes path: the function returns raw `[]u8` (audio/file bytes), not
            // a JSON struct. Call, defer-free, then check len for not_empty/is_empty.
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let has_bytes_assertions = fixture
                .assertions
                .iter()
                .any(|a| matches!(a.assertion_type.as_str(), "not_empty" | "is_empty"));
            if has_bytes_assertions {
                for assertion in &fixture.assertions {
                    match assertion.assertion_type.as_str() {
                        "not_empty" => {
                            let _ = writeln!(out, "    try testing.expect(_result_json.len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), _result_json.len);");
                        }
                        "not_error" | "error" => {}
                        _ => {
                            let atype = &assertion.assertion_type;
                            let _ = writeln!(
                                out,
                                "    // bytes result: assertion '{atype}' not implemented for zig bytes"
                            );
                        }
                    }
                }
            }
        } else if result_is_json_struct {
            // When streaming-virtual field assertions are present (pre-computed above),
            // emit raw FFI code to collect all chunks instead of calling
            // the high-level streaming wrapper (which only returns the last chunk's JSON).
            if uses_streaming_virtual_path {
                let Some(streaming_adapter) = streaming_adapter.as_ref() else {
                    let _ = writeln!(
                        out,
                        "    // skipped: streaming fixture requires matching [[crates.adapters]] metadata for zig e2e codegen"
                    );
                    let _ = writeln!(out, "    return error.SkipZigTest;");
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    return;
                };
                let owner_snake = streaming_adapter.owner_type.to_snake_case();
                let request_snake = streaming_adapter.request_type.to_snake_case();
                let request_from_json = format!("{ffi_prefix}_{request_snake}_from_json");
                let request_free = format!("{ffi_prefix}_{request_snake}_free");
                let stream_start = format!("{ffi_prefix}_{owner_snake}_{}_start", streaming_adapter.adapter_name);
                let stream_free = format!("{ffi_prefix}_{owner_snake}_{}_free", streaming_adapter.adapter_name);
                // Streaming-virtual path: inline FFI collect.
                // Build a sentinel-terminated request string.
                let _ = writeln!(
                    out,
                    "    const _req_z = try std.heap.c_allocator.dupeZ(u8, {args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_req_z);");
                let _ = writeln!(
                    out,
                    "    const _req_handle = {module_name}.c.{request_from_json}(_req_z.ptr);"
                );
                let _ = writeln!(out, "    defer {module_name}.c.{request_free}(_req_handle);");
                let _ = writeln!(
                    out,
                    "    const _stream_handle = {module_name}.c.{stream_start}(_client._handle, _req_handle);"
                );
                let _ = writeln!(out, "    if (_stream_handle == 0) return error.StreamStartFailed;");
                let _ = writeln!(out, "    defer {module_name}.c.{stream_free}(_stream_handle);");
                // Emit the collect snippet (already has 4-space indentation baked in).
                let snip = StreamingFieldResolver::collect_snippet_zig(
                    "_stream_handle",
                    "chunks",
                    module_name,
                    ffi_prefix,
                    &streaming_adapter.owner_type,
                    &streaming_adapter.adapter_name,
                    &streaming_adapter.item_type,
                );
                out.push_str("    ");
                out.push_str(&snip);
                out.push('\n');
                // For non-streaming assertions (e.g. usage), we also need _result_json.
                // Re-serialize the last chunk in `chunks` to get the JSON.
                if streaming_path_has_non_streaming {
                    let _ = writeln!(
                        out,
                        "    const _result_json = if (chunks.items.len > 0) chunks.items[chunks.items.len - 1] else &[_]u8{{}};"
                    );
                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                }
                let mut assertions_body = String::new();
                for assertion in &fixture.assertions {
                    render_json_assertion(&mut assertions_body, assertion, result_var, field_resolver, true);
                }
                crate::e2e::codegen::fail_on_unavailable_field_markers(
                    &assertions_body,
                    "zig",
                    &fixture.id,
                    &fixture.assertions,
                );
                crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "zig", &fixture.id);
                out.push_str(&assertions_body);
            } else {
                // JSON struct path: parse result JSON and access fields dynamically.
                let _ = writeln!(
                    out,
                    "    const _result_json = try {call_prefix}.{function_name}({args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
                if any_emits_code {
                    // For certain functions like `interact()`, the result is a struct that
                    // the fixture expects to access via a wrapper field (e.g. "interaction.action_results").
                    // Since the Zig binding returns the serialized struct directly (without wrapping),
                    // we wrap it in a JSON object with the appropriate key before parsing.
                    let wrap_field = match function_name.as_str() {
                        "interact" => Some("interaction"),
                        _ => None,
                    };

                    let parse_json_var = if let Some(field) = wrap_field {
                        // Build the Zig format string for wrapping: {"field":{s}}
                        // In Zig: `std.fmt.allocPrint(..., "{\"field\":{s}}", .{value})`
                        // In Rust string literal: "{{{{\\\"field\\\":{{s}}}}}}" (each { → {{, each \ → \\)
                        let _ = writeln!(
                            out,
                            "    const _wrapped_json = try std.fmt.allocPrint(allocator, \"{{{{\\\"{}\\\":{{s}}}}}}\", .{{_result_json}});",
                            field
                        );
                        let _ = writeln!(out, "    defer allocator.free(_wrapped_json);");
                        "_wrapped_json".to_string()
                    } else {
                        // _result_json is already a []u8 slice from the Zig wrapper function,
                        // so pass it directly to parseFromSlice.
                        "_result_json".to_string()
                    };

                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, {parse_json_var}, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                    let mut assertions_body = String::new();
                    for assertion in &fixture.assertions {
                        render_json_assertion(&mut assertions_body, assertion, result_var, field_resolver, false);
                    }
                    crate::e2e::codegen::fail_on_unavailable_field_markers(
                        &assertions_body,
                        "zig",
                        &fixture.id,
                        &fixture.assertions,
                    );
                    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(
                        &assertions_body,
                        "zig",
                        &fixture.id,
                    );
                    out.push_str(&assertions_body);
                } else {
                    let _ = writeln!(out, "    std.debug.print(\"{{s}}\\n\", .{{_result_json}});");
                }
            }
        } else if any_emits_code {
            let try_kw = if call_returns_error_union { "try " } else { "" };
            let _ = writeln!(
                out,
                "    const {result_var} = {try_kw}{call_prefix}.{function_name}({args_str});"
            );
            let mut assertions_body = String::new();
            for assertion in &fixture.assertions {
                render_assertion(
                    &mut assertions_body,
                    assertion,
                    result_var,
                    field_resolver,
                    enum_fields,
                    result_is_option,
                    result_is_simple,
                );
            }
            crate::e2e::codegen::fail_on_unavailable_field_markers(
                &assertions_body,
                "zig",
                &fixture.id,
                &fixture.assertions,
            );
            crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "zig", &fixture.id);
            out.push_str(&assertions_body);
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    }

    if wrap_as_test {
        let _ = writeln!(out, "}}");
    }
}

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    module_name: &str,
    ffi_prefix: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> anyhow::Result<String> {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let result_var = call.effective_result_var();
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if call.args.iter().any(|argument| argument.arg_type == "test_backend") {
        anyhow::bail!("zig snippet `{}` requires test-backend lifecycle teardown", fixture.id);
    }
    let mut call_fixture = fixture.clone();
    if !expects_error {
        call_fixture.assertions.clear();
    }
    let mut test = String::new();
    render_test_fn(
        &mut test,
        &call_fixture,
        e2e_config,
        "",
        "",
        &[],
        module_name,
        ffi_prefix,
        config,
        type_defs,
        false,
        true,
    );
    // The test-mode error path captures the failure with a discarded `else |_|` arm
    // (nothing to report inside `test { ... }`). The snippet is a runnable `main`,
    // so swap in a named capture that prints the caught error instead. ~keep
    let mut body = test
        .lines()
        .map(|line| {
            let line = line.replace(
                "else |_| {}",
                "else |err| { std.debug.print(\"call failed as expected: {s}\\n\", .{@errorName(err)}); }",
            );
            if !expects_error && !call.returns_void {
                line.replacen("_ = try ", &format!("const {result_var} = try "), 1)
                    .replacen("_ = ", &format!("const {result_var} = "), 1)
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A `result_is_json_struct` call binds `_result_json` — a `[]u8` payload — instead of a
    // typed `result`, so no `docs.shows` field path has a struct to read from. Those
    // fixtures keep the whole-payload print below. ~keep
    let binds_typed_result = !expects_error && !call.returns_void && !body.contains("const _result_json =");
    let presentation = if binds_typed_result {
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "zig")
    } else {
        Vec::new()
    };
    if !expects_error && !call.returns_void && presentation.is_empty() {
        let displayed_result = if body.contains("const _result_json =") {
            "_result_json"
        } else {
            result_var
        };
        let format = if displayed_result == "_result_json" {
            "{s}"
        } else {
            "{any}"
        };
        // `join("\n")` above drops the trailing newline `render_test_fn` wrote, so appending
        // without restoring it splices the print onto the end of the last statement line —
        // legal Zig, but a published snippet a reader has to un-run-on. ~keep
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "    std.debug.print(\"{format}\\n\", .{{{displayed_result}}});\n"
        ));
    }
    Ok(crate::e2e::template_env::render(
        "zig/snippet_body.jinja",
        minijinja::context! { module => module_name, body => body, body_is_indented => true,
        presentation => presentation },
    ))
}

#[cfg(test)]
mod snippet_tests {
    use super::*;

    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );
        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(
            !rendered.contains("MOCK_SERVER"),
            "mock-server env var leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("\"test-key\""),
            "literal credential leaked:\n{rendered}"
        );
        assert!(
            rendered.contains("std.c.getenv(\"API_KEY\")"),
            "credential is not read from the environment:\n{rendered}"
        );
        assert!(
            rendered.contains("sample.create_client(std.mem.span(_api_key), null, null, null, null)"),
            "client is not constructed the way a reader would:\n{rendered}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Zig
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
    /// (`python/mod.rs::client_factory_snippet_renders_the_base_url_the_fixture_documents`).
    /// Paired with `client_factory_snippet_without_docs_client_keeps_the_base_url_slot_null`
    /// below as the negative control: an indiscriminate "always add base_url" change would
    /// fail that test. ~keep
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url",
            "description": "Custom base URL",
            "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(
            rendered.contains(
                "sample.create_client(std.mem.span(_api_key), \"https://llm.internal.example.com/v1\", null, null, null)"
            ),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{rendered}"
        );
    }

    /// Negative control for the test above: a fixture that declares no `docs.client` must
    /// keep the base-url slot as a bare `null`, exactly what the generator rendered before
    /// this fixture's docs got wired up. ~keep
    #[test]
    fn client_factory_snippet_without_docs_client_keeps_the_base_url_slot_null() {
        let fixture = Fixture {
            id: "no_docs_client".into(),
            description: "No docs client".into(),
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(
            rendered.contains("sample.create_client(std.mem.span(_api_key), null, null, null, null)"),
            "a fixture with no docs.client must keep the bare base-url slot:\n{rendered}"
        );
    }

    #[test]
    fn e2e_test_file_still_points_the_client_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );
        let rendered = render_test_file(
            "errors",
            &[&fixture],
            &e2e,
            "chat",
            "result",
            &[],
            "sample",
            "sample",
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(
            rendered.contains("MOCK_SERVER_URL"),
            "e2e test lost its mock-server wiring:\n{rendered}"
        );
        assert!(
            rendered.contains("/fixtures/rate_limit_429"),
            "e2e test lost its per-fixture route:\n{rendered}"
        );
    }

    #[test]
    fn snippet_keeps_import_and_call_without_test_harness() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "count".into();
        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");
        assert!(rendered.contains("const sample = @import(\"sample\")"));
        assert!(rendered.contains("const result = try sample.count()"));
        assert!(rendered.contains("std.debug.print(\"{any}\\n\", .{result})"));
        assert!(!rendered.contains("test \""));
        assert!(!rendered.contains("defer "));
        assert!(rendered.contains("pub fn main() !void"));
        assert!(!rendered.contains("testing."));
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(rendered.contains("const result = try sample.process()"), "{rendered}");
        assert!(
            rendered.contains("std.debug.print(\"{s}\\n\", .{ result.summary });"),
            "{rendered}"
        );
        assert!(rendered.contains("for (result.items) |item| {"), "{rendered}");
        assert!(
            rendered.contains("std.debug.print(\"{any}\\n\", .{ item.label });"),
            "{rendered}"
        );
        assert!(
            !rendered.contains(".{result})"),
            "the whole-result fallback must give way to the documented presentation:\n{rendered}"
        );
    }

    /// A `result_is_json_struct` call binds `_result_json` (a `[]u8` payload) and never a
    /// typed `result`, so `docs.shows` field paths have no struct to read from — the
    /// snippet must keep printing the whole payload rather than emit `result.summary`.
    #[test]
    fn json_struct_result_keeps_the_payload_print_instead_of_field_accessors() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true}
            ]}}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.result_fields = ["summary".to_string()].into_iter().collect();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                result_is_json_struct: true,
                ..Default::default()
            },
        );

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(!rendered.contains("result.summary"), "{rendered}");
        assert!(
            rendered.contains("std.debug.print(\"{s}\\n\", .{_result_json})"),
            "{rendered}"
        );
    }

    #[test]
    fn json_result_snippet_consumes_and_frees_the_result() {
        let fixture = Fixture {
            id: "process".into(),
            description: "Process".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                result_is_json_struct: true,
                ..Default::default()
            },
        );

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(
            rendered.contains("const _result_json = try sample.process()"),
            "{rendered}"
        );
        assert!(rendered.contains("free(_result_json)"), "{rendered}");
        assert!(rendered.contains("std.debug.print(\"{s}\\n\", .{_result_json})"));
    }

    #[test]
    fn generated_tests_preserve_abort_failures() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "count".into();
        let rendered = render_test_file(
            "smoke",
            &[&fixture],
            &e2e,
            "count",
            "result",
            &[],
            "sample",
            "sample",
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(!rendered.contains("suppress_abort"), "{rendered}");
        assert!(!rendered.contains("signal(6, 1)"), "{rendered}");
    }

    #[test]
    fn expected_error_snippet_prints_caught_error_name() {
        let mut fixture = Fixture {
            id: "invalid".into(),
            description: "Invalid".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");
        assert!(rendered.contains("else |err|"));
        assert!(rendered.contains("call failed as expected"));
        assert!(rendered.contains("return error.TestUnexpectedResult"));
        assert!(!rendered.contains("testing.expect"));
    }

    #[test]
    fn visitor_snippet_reuses_native_callback_setup() {
        let mut fixture = Fixture {
            id: "custom_text".into(),
            description: "Custom text".into(),
            input: serde_json::json!({ "html": "<p>Hello</p>" }),
            ..Fixture::default()
        };
        fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
            callbacks: [("visit_text".into(), crate::e2e::fixture::CallbackAction::Continue)].into(),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "render_document".into();
        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("visitor snippet renders");
        assert!(rendered.contains("pub fn main() !void"));
        assert!(rendered.contains("_visitor"));
        assert!(!rendered.contains("test \""));
        assert!(!rendered.contains("testing."));
        assert!(!rendered.contains("\n    }\n}"), "{rendered}");
    }

    #[test]
    fn streaming_snippet_reuses_error_union_call_preparation() {
        let fixture = Fixture {
            id: "stream_items".into(),
            description: "Stream items".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "stream_items".into();
        e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("streaming snippet renders");

        assert!(rendered.contains("stream_items"), "{rendered}");
        assert!(rendered.contains("pub fn main() !void"));
    }

    #[test]
    fn streaming_e2e_uses_scalar_handle_tokens() {
        let mut fixture = Fixture {
            id: "stream_records".into(),
            description: "Stream records".into(),
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "not_empty".into(),
            field: Some("chunks".into()),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "stream_records".into();
        e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));
        e2e.call.overrides.insert(
            "zig".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                result_is_json_struct: true,
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            adapters: vec![crate::core::config::AdapterConfig {
                name: "stream_records".into(),
                pattern: AdapterPattern::Streaming,
                core_path: "sample::Client::stream_records".into(),
                params: Vec::new(),
                returns: None,
                error_type: None,
                owner_type: Some("Client".into()),
                item_type: Some("Record".into()),
                gil_release: false,
                trait_name: None,
                trait_method: None,
                detect_async: false,
                request_type: Some("sample::RecordRequest".into()),
                skip_languages: Vec::new(),
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_test_file(
            "streaming",
            &[&fixture],
            &e2e,
            "stream_records",
            "result",
            &[],
            "sample",
            "sample",
            &config,
            &[],
        );

        assert!(rendered.contains("sample.c.sample_client_stream_records_start(_client._handle, _req_handle)"));
        assert!(rendered.contains("if (_stream_handle == 0)"));
        assert!(!rendered.contains("@ptrCast(_client._handle)"));
    }

    /// Fixture shared by the snippet/test-target pair below: a `json_object` arg whose docs
    /// presentation replaces a nested field with a file read (mirrors the batch `bytes_happy`
    /// fixture that drives `test_documents/html/html.html` into `/inputs/1/bytes`). The file
    /// read is emitted by `render_docs_json`, the SAME emitter `generate()` (test target) and
    /// `render_snippet_body()` (snippet target) both call through `build_args_and_setup`.
    fn docs_bytes_fixture_and_config() -> (Fixture, E2eConfig) {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "docs_nested_bytes_snippet",
            "description": "Process a document loaded from disk",
            "input": {"content": "ignored"},
            "assertions": [],
            "docs": {
                "topic": "guides",
                "presentation": {
                    "files": [{"field": "/content", "path": "document.pdf"}]
                }
            }
        }))
        .expect("fixture");

        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "input".into(),
            field: "input".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        (fixture, e2e)
    }

    /// Doc snippets compile as standalone `pub fn main()` executables, not `zig test`
    /// binaries — `std.testing.io` is only valid inside `builtin.is_test` code (Zig rejects
    /// it with "not testing" otherwise). The docs-file read must therefore never reference
    /// `std.testing`, regardless of target.
    #[test]
    fn snippet_target_never_references_std_testing_for_a_docs_bytes_input() {
        let (fixture, e2e) = docs_bytes_fixture_and_config();

        let rendered = render_snippet_body(&fixture, &e2e, "sample", "sample", &ResolvedCrateConfig::default(), &[])
            .expect("snippet renders");

        assert!(rendered.contains("readFileAlloc"), "{rendered}");
        assert!(rendered.contains("std.Io.Threaded"), "{rendered}");
        assert!(!rendered.contains("std.testing"), "{rendered}");
    }

    /// Sibling control for the snippet-target assertion above: the SAME fixture, rendered
    /// through the e2e test-file generator, still imports `std.testing` (legitimate inside a
    /// `test { ... }` block) — proving the fix removed the *dependency* on `std.testing.io`
    /// from the shared file-read emitter without stripping `std.testing` from the target
    /// that is actually allowed to reference it.
    #[test]
    fn test_target_still_references_std_testing_for_the_same_docs_bytes_input() {
        let (fixture, e2e) = docs_bytes_fixture_and_config();

        let rendered = render_test_file(
            "guides",
            &[&fixture],
            &e2e,
            "process",
            "result",
            &e2e.call.args,
            "sample",
            "sample",
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(rendered.contains("const testing = std.testing;"), "{rendered}");
        assert!(rendered.contains("readFileAlloc"), "{rendered}");
        assert!(rendered.contains("std.Io.Threaded"), "{rendered}");
        assert!(!rendered.contains("std.testing.io"), "{rendered}");
    }
}

#[cfg(test)]
mod expects_error_fails_on_unexpected_success_tests {
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
}

#[cfg(test)]
mod error_value_and_error_field_tests {
    use super::*;

    fn assertion(assertion_type: &str, field: Option<&str>, value: Option<&str>) -> crate::e2e::fixture::Assertion {
        crate::e2e::fixture::Assertion {
            assertion_type: assertion_type.into(),
            field: field.map(str::to_string),
            value: value.map(|v| serde_json::Value::String(v.to_string())),
            ..Default::default()
        }
    }

    fn render(assertions: Vec<crate::e2e::fixture::Assertion>) -> String {
        let fixture = Fixture {
            id: "invalid_input".into(),
            description: "Rejects invalid input".into(),
            assertions,
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_file(
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
        )
    }

    /// The defect: a declared `error` value was discarded, leaving `} else |_| {}` — a check that
    /// passes for ANY failure. The message and the error-set member name must both be compared.
    #[test]
    fn a_declared_error_value_is_compared_against_message_and_error_name() {
        let rendered = render(vec![assertion("error", None, Some("BadRequest"))]);

        assert!(
            rendered.contains("if (sample.parse()) |_| {"),
            "the error block itself must still render: {rendered}"
        );
        assert!(
            rendered.contains("const _err_message: []const u8 = sample._last_error() orelse \"\";"),
            "the FFI message must be bound: {rendered}"
        );
        assert!(
            rendered.contains("std.mem.indexOf(u8, _err_message, \"BadRequest\") != null"),
            "the declared value must be compared to the message: {rendered}"
        );
        assert!(
            rendered.contains("std.mem.indexOf(u8, _err_name, \"BadRequest\") != null"),
            "the declared value must also be compared to @errorName: {rendered}"
        );
        assert!(
            !rendered.contains("} else |_| {}"),
            "the value-discarding arm must be gone: {rendered}"
        );
    }

    /// Negative control for the arm above: with no declared value the output is byte-identical to
    /// the pre-existing shape, so the change cannot have rewritten every error fixture.
    #[test]
    fn an_error_assertion_without_a_value_keeps_the_bare_arm() {
        let rendered = render(vec![assertion("error", None, None)]);

        assert!(rendered.contains("if (sample.parse()) |_| {"), "{rendered}");
        assert!(rendered.contains("} else |_| {}"), "{rendered}");
        assert!(!rendered.contains("_last_error()"), "{rendered}");
    }

    #[test]
    fn an_equals_on_an_error_field_is_named_instead_of_dropped() {
        let rendered = render(vec![
            assertion("error", None, Some("BadRequest")),
            assertion("equals", Some("error.status_code"), None),
        ]);

        // Positive first: the fixture really did produce an error block.
        assert!(
            rendered.contains("return error.TestUnexpectedResult;"),
            "the error block must render before we assert anything about the second assertion: {rendered}"
        );
        assert!(
            rendered.contains(
                "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "{rendered}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "zig");
        assert_eq!(records[0].field, "equals");
    }
}
