//! C e2e test generator using assert.h and a Makefile.
//!
//! Generates `e2e/c/Makefile`, per-category `test_{category}.c` files,
//! a `main.c` test runner, a `test_runner.h` header, and a
//! `download_ffi.sh` script for downloading prebuilt FFI libraries from
//! GitHub releases.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::escape::{escape_c, sanitize_filename};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// C e2e code generator.
pub struct CCodegen;

/// Returns true when `t` is a primitive C scalar type (uint64_t, int32_t, double,
/// etc.) that should be emitted as a typed local variable rather than a heap
/// `char*` accessor result.
fn is_primitive_c_type(t: &str) -> bool {
    matches!(
        t,
        "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uintptr_t"
            | "intptr_t"
            | "size_t"
            | "ssize_t"
            | "double"
            | "float"
            | "bool"
            | "int"
    )
}

/// Returns `true` when `fields_c_types["{parent}.{field}"]` is the magic
/// sentinel `"skip"` — the C codegen should omit any assertion that touches
/// this field rather than emitting a call to a non-existent FFI function.
fn is_skipped_c_field(fields_c_types: &HashMap<String, String>, parent_snake: &str, field_snake: &str) -> bool {
    let key = format!("{parent_snake}.{field_snake}");
    fields_c_types.get(&key).is_some_and(|t| t == "skip")
}

/// The C ABI represents every opaque/named type (`TypeRef::Named`) as the
/// scalar generational handle `AlefHandle` (`typedef uint64_t {PREFIX}AlefHandle`)
/// — see `src/backends/ffi/type_map.rs::c_param_optional`/`c_return_optional`.
/// An absent optional argument of that kind must therefore use `0` as its
/// "none" sentinel, matching the FFI bridge codegen's own convention
/// (`src/backends/ffi/gen_bindings/helpers.rs::ffi_null_return_value`,
/// `Some("AlefHandle") => "0"`). Every other arg kind (`string`, `mock_url`,
/// `bytes`, ...) is a genuine C pointer (`const char *`, `void *`) and keeps
/// the `NULL` sentinel.
///
/// `"json_object"` args are handle-typed here because the C e2e codegen always
/// materializes them via a `{prefix}_{type}_from_json(...)` call that returns
/// an `AlefHandle`. `"handle"` args (used by other language codegens for an
/// argument that is already a pre-built handle) are handle-typed for the same
/// reason: the parameter they fill is declared `AlefHandle`, never a pointer.
fn c_optional_sentinel(arg_type: &str) -> &'static str {
    if matches!(arg_type, "json_object" | "handle") {
        "0"
    } else {
        "NULL"
    }
}

/// Infer the opaque-handle PascalCase return type for a bare-field accessor.
///
/// Returns `Some(pascal_type)` when the accessor `{prefix}_{parent}_{field}`
/// returns a pointer to an opaque struct (e.g. `SAMPLELLMUsage*`) rather than
/// a `char*` or primitive scalar.
///
/// Detection strategy:
/// 1. Direct lookup `fields_c_types["{parent}.{field}"]` — if present and
///    NOT a primitive AND NOT `char*`, treat as an opaque handle of that
///    PascalCase type.
/// 2. Inferred lookup — when ANY key in `fields_c_types` starts with
///    `"{field}."` (the snake_case of `field` as a parent type), the field
///    must be a struct whose nested fields are mapped. Default the struct
///    type to `field.to_pascal_case()`. This mirrors the fallback used by
///    `emit_nested_accessor` for intermediate segments.
///
/// Returns `None` when the field looks like a `char*` string accessor.
fn infer_opaque_handle_type(
    fields_c_types: &HashMap<String, String>,
    parent_snake_type: &str,
    field_snake: &str,
) -> Option<String> {
    let lookup_key = format!("{parent_snake_type}.{field_snake}");
    if let Some(t) = fields_c_types.get(&lookup_key) {
        if !is_primitive_c_type(t) && t != "char*" {
            return Some(t.clone());
        }
        // Primitive or explicit char* — caller handles those paths.
        return None;
    }
    // Inferred: nested keys exist with `field_snake` as the parent type prefix.
    let nested_prefix = format!("{field_snake}.");
    if fields_c_types.keys().any(|k| k.starts_with(&nested_prefix)) {
        return Some(field_snake.to_pascal_case());
    }
    None
}

/// Try to emit an enum-aware field accessor: when `raw_field`/`resolved_field`
/// is registered in `fields_enum` AND `fields_c_types[parent.field]` resolves
/// to a non-primitive PascalCase type name, treat the accessor return as an
/// opaque enum pointer and convert it to `char*` via the FFI's
/// `{prefix}_{enum_snake}_to_string` accessor.
///
/// Without this, the C codegen would default-declare the accessor result as
/// `char* status = {prefix}_batch_object_status(result);` and string-compare
/// it — but the FFI returns `SAMPLELLMBatchStatus*` (an opaque enum struct
/// pointer), not a C string. The mismatch causes immediate `Abort trap: 6` /
/// `strcmp(NULL,...)` failures in every assertion that targets an enum field.
///
/// Returns `true` when an accessor was emitted (caller must NOT emit the
/// default `char*` declaration). When emitted, the opaque-enum handle is
/// pushed to `intermediate_handles` so the existing cleanup loop frees it via
/// `{prefix}_{enum_snake}_free(...)` after the test body runs.
#[allow(clippy::too_many_arguments)]
fn try_emit_enum_accessor(
    out: &mut String,
    prefix: &str,
    prefix_upper: &str,
    raw_field: &str,
    resolved_field: &str,
    parent_snake_type: &str,
    accessor_fn: &str,
    parent_handle: &str,
    local_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
) -> bool {
    if !(fields_enum.contains(raw_field) || fields_enum.contains(resolved_field)) {
        return false;
    }
    let lookup_key = format!("{parent_snake_type}.{resolved_field}");
    let Some(enum_pascal) = fields_c_types.get(&lookup_key) else {
        return false;
    };
    if is_primitive_c_type(enum_pascal) || enum_pascal == "char*" {
        return false;
    }
    let enum_snake = enum_pascal.to_snake_case();
    let handle_var = format!("{local_var}_handle");
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle {handle_var} = {accessor_fn}({parent_handle});"
    );
    let _ = writeln!(out, "    assert({handle_var} != 0);");
    let _ = writeln!(
        out,
        "    char* {local_var} = {prefix}_{enum_snake}_to_string({handle_var});"
    );
    intermediate_handles.push((handle_var, enum_snake));
    true
}

impl E2eCodegen for CCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve default call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let result_var = &call.result_var;
        let prefix = overrides
            .and_then(|o| o.prefix.as_ref())
            .cloned()
            .or_else(|| config.ffi.as_ref().and_then(|ffi| ffi.prefix.as_ref()).cloned())
            .unwrap_or_default();
        let header = overrides
            .and_then(|o| o.header.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_header_name());

        // Resolve package config.
        let c_pkg = e2e_config.resolve_package("c");
        // lib_name is the actual Rust library name (for linking)
        let lib_name = config.ffi_lib_name();

        // ffi_pkg_name is the release artifact package name (for downloads).
        // Derived from lib_name (for example, "sample_ffi" stays "sample_ffi") because
        // the publish workflow stages tarballs as "${lib_name}-v${VERSION}-${TRIPLE}.tar.gz".
        // The explicit e2e package name is a fallback for edge cases where the release
        // artifact name differs from the library name.
        let ffi_pkg_name = c_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| lib_name.clone());

        // Filter active groups (with non-skipped fixtures).
        let active_groups: Vec<(&FixtureGroup, Vec<&Fixture>)> = groups
            .iter()
            .filter_map(|group| {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    .filter(|f| f.visitor.is_none())
                    .collect();
                if active.is_empty() { None } else { Some((group, active)) }
            })
            .collect();

        // Collect active visitor fixtures (flattened across all groups).
        let visitor_fixtures: Vec<&Fixture> = groups
            .iter()
            .flat_map(|group| group.fixtures.iter())
            .filter(|f| super::should_include_fixture(f, lang, e2e_config))
            .filter(|f| f.visitor.is_some())
            .filter(|f| c_visitor_fixture_has_typed_call(f, e2e_config))
            .collect();

        // Resolve FFI crate path for local repo builds.
        // Default to `../../crates/{name}-ffi` derived from the crate name so that
        // projects with named FFI crates resolve to `../../crates/{name}-ffi/include/`
        // rather than the generic (incorrect) `../../crates/ffi`.
        // When `[crates.output] ffi` is set explicitly, derive the crate path from
        // that value so that renamed FFI crates (e.g. `parser-core-core-ffi`) resolve
        // correctly without any hardcoded special cases.
        let ffi_crate_path = c_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_crate_path());

        // Generate Makefile.
        let mut category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
        if !visitor_fixtures.is_empty() {
            category_names.push("visitor".to_string());
        }
        let needs_mock_server = active_groups
            .iter()
            .flat_map(|(_, fixtures)| fixtures.iter())
            .any(|f| f.needs_mock_server());
        files.push(GeneratedFile {
            path: output_base.join("Makefile"),
            content: render_makefile(&category_names, &header, &ffi_crate_path, &lib_name, needs_mock_server),
            generated_header: true,
        });

        // Generate download_ffi.sh for downloading prebuilt FFI from GitHub releases.
        let github_repo = config.github_repo();
        let version = config.resolved_version().unwrap_or_else(|| "0.0.0".to_string());
        files.push(GeneratedFile {
            path: output_base.join("download_ffi.sh"),
            content: render_download_script(&github_repo, &version, &ffi_pkg_name),
            generated_header: true,
        });

        // Generate test_runner.h.
        files.push(GeneratedFile {
            path: output_base.join("test_runner.h"),
            content: render_test_runner_header(&active_groups, &visitor_fixtures),
            generated_header: true,
        });

        // Generate main.c.
        files.push(GeneratedFile {
            path: output_base.join("main.c"),
            content: render_main_c(&active_groups, &visitor_fixtures, &e2e_config.env),
            generated_header: true,
        });

        // Generate .gitignore so locally-built binaries and mock-server pipe
        // artifacts are never accidentally checked in. A committed macOS Mach-O
        // `run_tests` binary will fail Linux CI with `Exec format error`.
        files.push(GeneratedFile {
            path: output_base.join(".gitignore"),
            content: render_gitignore(),
            generated_header: false,
        });

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &std::collections::HashSet::new(),
        );

        // Generate per-category test files.
        // Each fixture may reference a named call config (fixture.call), so we pass
        // e2e_config to render_test_file so it can resolve per-fixture call settings.
        for (group, active) in &active_groups {
            let filename = format!("test_{}.c", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                active,
                &header,
                &prefix,
                result_var,
                e2e_config,
                lang,
                &field_resolver,
                config,
                type_defs,
            )?;
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        // Generate test_visitor.c if there are visitor fixtures.
        if !visitor_fixtures.is_empty() {
            files.push(GeneratedFile {
                path: output_base.join("test_visitor.c"),
                content: render_visitor_test_file(&visitor_fixtures, &header, &prefix, e2e_config, config),
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        render_c_snippet(fixture, e2e_config, config, type_defs, &[])
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
    ) -> Result<String> {
        render_c_snippet(fixture, e2e_config, config, type_defs, functions)
    }

    fn language_name(&self) -> &'static str {
        "c"
    }
}

fn render_c_snippet(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    let mut info = resolve_fixture_call_info(fixture, e2e_config, config, "c", functions);
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let prefix = call
        .overrides
        .get("c")
        .and_then(|value| value.prefix.clone())
        .or_else(|| config.ffi.as_ref().and_then(|value| value.prefix.clone()))
        .unwrap_or_else(|| config.ffi_prefix());
    if info.client_factory.is_none()
        && info.c_engine_factory.is_none()
        && !prefix.is_empty()
        && !info.function_name.starts_with(&format!("{prefix}_"))
    {
        info.function_name = crate::codegen::naming::abi_symbol(&prefix, &info.function_name);
    }
    let header = call
        .overrides
        .get("c")
        .and_then(|value| value.header.clone())
        .unwrap_or_else(|| config.ffi_header_name());
    let (ir_reachable_fields, ir_known_excluded_fields) = FieldResolver::ir_field_sets(type_defs);
    let resolver = FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
    test_function::render_snippet_body(test_function::SnippetContext {
        fixture,
        e2e_config,
        header: &header,
        prefix: &prefix,
        info: &info,
        field_resolver: &resolver,
        config,
        type_defs,
    })
}

/// Resolve per-call-config C-specific settings for a given call config and lang.
struct ResolvedCallInfo {
    function_name: String,
    result_type_name: String,
    options_type_name: String,
    client_factory: Option<String>,
    args: Vec<crate::e2e::config::ArgMapping>,
    raw_c_result_type: Option<String>,
    c_free_fn: Option<String>,
    c_engine_factory: Option<String>,
    result_is_option: bool,
    returns_void: bool,
    /// When `true`, the FFI signature for this method follows the byte-buffer
    /// out-pointer pattern: `int32_t fn(this, req, uint8_t** out_ptr,
    /// uintptr_t* out_len, uintptr_t* out_cap)`. The C codegen emits out-param
    /// declarations, a status-code check, and `<prefix>_free_bytes` rather
    /// than treating the result as an opaque response handle.
    result_is_bytes: bool,
    streaming: Option<bool>,
    /// Per-language `extra_args` from call overrides — verbatim trailing
    /// arguments appended after the configured `args`. The C codegen passes
    /// `NULL` for absent optional pointers via this mechanism.
    extra_args: Vec<String>,
}

fn resolve_call_info(call: &CallConfig, lang: &str, functions: &[crate::core::ir::FunctionDef]) -> ResolvedCallInfo {
    let overrides = call.overrides.get(lang);
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.clone());
    // Fall back to the *base* (non-C-overridden) function name when no explicit
    // result_type is set.  Using the C-overridden name (e.g. "htm_convert") would
    // produce a doubled-prefix type like `HTMHtmConvert*`; the base name
    // ("convert") yields the correct `HTMConvert*` shape.
    let result_type_name = overrides
        .and_then(|o| o.result_type.as_ref())
        .cloned()
        .or_else(|| resolve_ir_result_type(call, lang, functions))
        .unwrap_or_else(|| fallback_result_type_name(call, lang, functions));
    let options_type_name = overrides
        .and_then(|o| o.options_type.as_deref())
        .or(call.options_type.as_deref())
        .unwrap_or_default()
        .to_string();
    let client_factory = overrides.and_then(|o| o.client_factory.as_ref()).cloned();
    let raw_c_result_type = overrides
        .and_then(|o| o.raw_c_result_type.clone())
        .or_else(|| return_shape::resolve_raw_c_result_type(call, lang, functions));
    let c_free_fn = overrides.and_then(|o| o.c_free_fn.clone());
    let c_engine_factory = overrides.and_then(|o| o.c_engine_factory.clone());
    let result_is_option = overrides
        .and_then(|o| if o.result_is_option { Some(true) } else { None })
        .unwrap_or(call.result_is_option);
    let returns_void = call.returns_void;
    // result_is_bytes is read from either the call-level config (preferred —
    // the byte-buffer FFI shape is identical across languages that use the
    // same FFI crate) or the per-language override (back-compat with the
    // pattern used by Java / PHP / etc.).
    let result_is_bytes = call.result_is_bytes || overrides.is_some_and(|o| o.result_is_bytes);
    let extra_args = overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let mut args = call.args.clone();
    // `functions` is the Rust core's IR, so this lookup wants the Rust identity and must NOT
    // resolve `overrides.c.function` — that names a prefixed C export (`samplellm_chat`), not
    // the Rust function (`chat`). `core_lookup_name` keeps the base name as the key and only
    // supplies a fallback when the base names nothing at all, which stops the key degrading
    // to `""` and silently deriving arg/result types from the empty string. ~keep
    let core_lookup_name = call.core_lookup_name(lang);
    if let Some(function) = core_lookup_name
        .as_deref()
        .and_then(|name| functions.iter().find(|function| function.name == name))
    {
        for (index, arg) in args.iter_mut().enumerate() {
            if arg.element_type.is_some() || arg.arg_type != "json_object" {
                continue;
            }
            let parameter = function
                .params
                .iter()
                .find(|parameter| parameter.name == arg.name)
                .or_else(|| function.params.get(index));
            arg.element_type = parameter
                .and_then(|parameter| named_type(&parameter.ty))
                .map(str::to_string);
        }
    }
    ResolvedCallInfo {
        function_name,
        result_type_name,
        options_type_name,
        client_factory,
        args,
        raw_c_result_type,
        c_free_fn,
        c_engine_factory,
        result_is_option,
        returns_void,
        result_is_bytes,
        streaming: call.streaming_enabled(),
        extra_args,
    }
}

fn named_type(type_ref: &crate::core::ir::TypeRef) -> Option<&str> {
    match type_ref {
        crate::core::ir::TypeRef::Named(name) => Some(name),
        crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => named_type(inner),
        _ => None,
    }
}

/// Name the type a call's result handle points at, read from the core IR.
///
/// `FunctionDef::return_type` is already the `Ok` type: the extractor splits `Result<T, E>`
/// into `return_type = T` plus a separate `error_type`, so a fallible
/// `fn complete(..) -> Result<CompletionResponse, String>` resolves to `CompletionResponse`.
///
/// The named type is reached through [`named_type`], the recursive unwrapper this module
/// already uses for argument element types — a second, one-level-deep match sitting beside it
/// answered `None` for `Result<Vec<Model>, E>` and every other nesting, and every `None` here
/// lands on [`fallback_result_type_name`].
fn resolve_ir_result_type(call: &CallConfig, lang: &str, functions: &[crate::core::ir::FunctionDef]) -> Option<String> {
    let lookup_name = call.core_lookup_name(lang)?;
    let function = functions.iter().find(|function| function.name == *lookup_name)?;
    named_type(&function.return_type).map(str::to_string)
}

/// Last-resort result type when neither config nor the IR names one: PascalCase the call's
/// function name.
///
/// This invents a name. When it is wrong the damage surfaces stages later and quietly:
/// `result_type_name` becomes both the accessor prefix (`{prefix}_{result_snake}_{leaf}`) and
/// the `parent_is_ir_type` flag, and `ensure_leaf_field_exists` default-allows every leaf whose
/// parent is not an IR type — so a fabricated type does not fail generation, it *disables* the
/// nested-field verification for that fixture.
///
/// It nonetheless has to stay, because resolution legitimately cannot answer for two shapes:
///
/// - `E2eCodegen::generate` carries no `functions` parameter, so `render_test_file` passes an
///   empty slice and no call on the generated-test-file path can resolve at all. Only the
///   snippet path (`generate_e2e` → `render_snippet_body_with_functions`) is threaded.
/// - `ApiSurface::functions` holds free `pub fn`s only; a call naming an inherent or trait
///   method (a client's `chat`, say) is a `MethodDef` and is never in the slice.
///
/// Both are guess-shaped, so it warns when the IR was available and the call still did not
/// resolve — the actionable case, fixed by setting `result_type` on the call override — and
/// stays at debug for the structural gap, which is not per-call actionable and would otherwise
/// warn once for every fixture in the suite.
fn fallback_result_type_name(call: &CallConfig, lang: &str, functions: &[crate::core::ir::FunctionDef]) -> String {
    let result_type = call.function.to_pascal_case();
    if functions.is_empty() {
        tracing::debug!(
            call = %call.function,
            language = %lang,
            %result_type,
            "no core IR functions available to this generator; result type derived from the call name"
        );
    } else {
        tracing::warn!(
            call = %call.function,
            language = %lang,
            %result_type,
            "call did not resolve to a core IR function with a named return type; result type \
             derived from the call name, which disables nested-field verification if the name is \
             not a real type — set `result_type` on the call override"
        );
    }
    result_type
}

/// Resolve call info for a fixture, with fallback to default call's client_factory.
///
/// Named call configs (e.g. `[e2e.calls.embed]`) may not repeat the `client_factory`
/// setting. We fall back to the default `[e2e.call]` override's client_factory so that
/// all methods on the same client use the same pattern.
fn resolve_fixture_call_info(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    lang: &str,
    functions: &[crate::core::ir::FunctionDef],
) -> ResolvedCallInfo {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let mut info = resolve_call_info(call, lang, functions);

    // `trait_bridge_derived_c_identity` derives the C ABI symbol the FFI backend
    // actually generates for a trait-bridge registry operation, rather than trusting
    // the raw `fixture.call` config text (`register_fn`/`unregister_fn`/`clear_fn`),
    // which can diverge from it for `unregister`/`clear` (see that function's doc
    // comment for the exact derivation rule). A fixture author who set
    // `skip.languages` for `lang` has already declared that this generator cannot
    // speak for it, so this fallback must not run for a skipped fixture.
    // `src/e2e/snippets/mod.rs` applies an equivalent guard before it ever calls into
    // this generator, but this check must not depend on that upstream filtering having
    // happened -- a caller that reaches this function directly (as this module's own
    // unit tests, and the compiled e2e test-file path via `render_test_file`, both do)
    // must get the same protection on its own terms.
    let skipped_for_lang = fixture.skip.as_ref().is_some_and(|skip| skip.should_skip(lang));
    if info.function_name.is_empty()
        && !skipped_for_lang
        && let Some((operation, derived_name)) =
            crate::e2e::codegen::recipe::trait_bridge_derived_c_identity(config, fixture)
    {
        info.function_name = derived_name;
        // `unregister`/`clear` C exports always take a trailing `out_error` out-param
        // that the shared, language-agnostic `[crates.e2e.calls.*]` args config has no
        // way to express (other bindings surface it via an exception/error-return
        // mechanism instead). `register` needs no such treatment here: register-shaped
        // fixtures require vtable/user_data wiring this generic void-call fallback does
        // not build, so they never reach this branch as a `returns_void` call in
        // practice. See `unregister_fn.jinja` / `clear_fn.jinja` for the ABI shapes.
        if matches!(
            operation,
            crate::e2e::codegen::recipe::TraitBridgeRegistryOperation::Unregister
                | crate::e2e::codegen::recipe::TraitBridgeRegistryOperation::Clear
        ) {
            info.extra_args.push("NULL".to_string());
        }
    }

    let default_overrides = e2e_config.call.overrides.get(lang);

    // Fallback: if the named call has no client_factory override, inherit from the
    // default call config so all calls use the same client pattern.
    if info.client_factory.is_none()
        && let Some(factory) = default_overrides.and_then(|o| o.client_factory.as_ref())
    {
        info.client_factory = Some(factory.clone());
    }

    // Fallback: if the named call has no c_engine_factory override, inherit from the
    // default call config so all calls use the same engine pattern.
    if info.c_engine_factory.is_none()
        && let Some(factory) = default_overrides.and_then(|o| o.c_engine_factory.as_ref())
    {
        info.c_engine_factory = Some(factory.clone());
    }

    info
}

fn c_visitor_fixture_has_typed_call(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let info = resolve_call_info(call, "c", &[]);
    let has_function = call
        .overrides
        .get("c")
        .and_then(|override_config| override_config.function.as_deref())
        .is_some_and(|function| !function.is_empty());
    has_function && !info.options_type_name.is_empty()
}

mod assertions;
mod call_patterns;
mod docs_input;
mod project;
mod return_shape;
mod runner;
#[cfg(test)]
mod snippet_regressions;
mod streaming;
mod test_function;
mod trait_bridge_snippet;
mod visitor;

use assertions::{
    LeafFieldCheck, build_args_string_c, emit_nested_accessor, ensure_leaf_field_exists, render_assertion,
};
use call_patterns::{render_bytes_test_function, render_engine_factory_test_function};
use project::{render_download_script, render_gitignore, render_makefile};
use runner::{render_main_c, render_test_runner_header};
use streaming::{
    render_c_diagnostic_skip, render_streaming_test_function, resolve_c_client_owner_type, resolve_c_streaming_adapter,
    validate_c_snippet_metadata,
};
use test_function::render_test_function;
use visitor::render_visitor_test_file;

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    header: &str,
    prefix: &str,
    result_var: &str,
    e2e_config: &E2eConfig,
    lang: &str,
    field_resolver: &FieldResolver,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "/* E2e tests for category: {category} */");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <assert.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out, "#include \"{header}\"");
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        // Visitor fixtures are filtered out before render_test_file is called.
        // This guard is a safety net in case a fixture reaches here unexpectedly.
        if fixture.visitor.is_some() {
            panic!(
                "C e2e generator: visitor pattern not supported for fixture: {}",
                fixture.id
            );
        }

        let call_info = resolve_fixture_call_info(fixture, e2e_config, config, lang, &[]);

        // Effective enum fields for this fixture: merge global e2e_config.fields_enum
        // (HashSet) with the per-call C override's enum_fields (HashMap keys). This
        // mirrors Ruby/Java's pattern: global = always-enum-typed paths; per-call =
        // context-dependent paths (BatchObject.status is BatchStatus, but
        // ResponseObject.status is plain String).
        let mut effective_fields_enum = e2e_config.fields_enum.clone();
        let fixture_call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        if let Some(co) = fixture_call.overrides.get(lang) {
            for k in co.enum_fields.keys() {
                effective_fields_enum.insert(k.clone());
            }
        }

        // Per-call field resolver: overrides the top-level resolver when this call
        // declares its own result_fields / fields / fields_optional / fields_array.
        // Without this, `pages.length` on a `crawl` call would skip because the
        // default `result_fields` (configured for the top-level `scrape` call)
        // does not contain `pages`.
        let (ir_reachable_fields, ir_known_excluded_fields) = FieldResolver::ir_field_sets(type_defs);
        let per_call_field_resolver = FieldResolver::new(
            e2e_config.effective_fields(fixture_call),
            e2e_config.effective_fields_optional(fixture_call),
            e2e_config.effective_result_fields(fixture_call),
            e2e_config.effective_fields_array(fixture_call),
            &std::collections::HashSet::new(),
        )
        .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
        let _ = field_resolver; // top-level resolver retained for compat; per-call wins
        let field_resolver = &per_call_field_resolver;

        // `out` accumulates every fixture's rendered function in this file, so the
        // strict-availability scan below must only look at the text THIS fixture's
        // own render appended — scanning the whole buffer would misattribute an
        // earlier fixture's skip comment to this fixture's id.
        let fixture_start = out.len();
        render_test_function(
            &mut out,
            fixture,
            prefix,
            &call_info.function_name,
            result_var,
            &call_info.args,
            field_resolver,
            &e2e_config.fields_c_types,
            &effective_fields_enum,
            &call_info.result_type_name,
            &call_info.options_type_name,
            call_info.client_factory.as_deref(),
            call_info.raw_c_result_type.as_deref(),
            call_info.c_free_fn.as_deref(),
            call_info.c_engine_factory.as_deref(),
            call_info.result_is_option,
            call_info.result_is_bytes,
            call_info.streaming,
            &call_info.extra_args,
            config,
            type_defs,
            false,
        )?;
        crate::e2e::codegen::fail_on_unavailable_field_markers(&out[fixture_start..], "c", &fixture.id);
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    Ok(out)
}

#[allow(clippy::too_many_arguments)]
/// Convert a `serde_json::Value` to a C literal string.
fn json_to_c(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
        serde_json::Value::Bool(true) => "1".to_string(),
        serde_json::Value::Bool(false) => "0".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "NULL".to_string(),
        other => format!("\"{}\"", escape_c(&other.to_string())),
    }
}

/// Emit a test backend stub.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    trait_bridge_snippet::emit_test_backend(trait_bridge, methods, fixture)
}

#[cfg(test)]
mod snippet_tests {
    use super::*;

    #[test]
    fn snippet_keeps_header_and_call_without_test_harness() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_count".into();
        e2e.call.result_var = "result".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("snippet renders");
        assert!(rendered.contains("#include \""));
        assert!(rendered.contains("sample_count("));
        assert!(rendered.contains("int main(void)"));
        assert!(!rendered.contains("void test_"));
        assert!(!rendered.contains("assert("));
        assert!(rendered.contains("_free(result)"), "{rendered}");
    }

    /// `clear_fn = "clear_sample_backends"` (plural, human-written config text) on a
    /// trait named `SampleBackend` (singular). `registration.rs` derives the exported
    /// symbol from the trait name's snake_case form, discarding the config text's
    /// spelling, so the real ABI symbol is `sample_clear_sample_backend` (singular) --
    /// and it takes a trailing `out_error` out-param (`clear_fn.jinja`), so the call
    /// site must pass `NULL`. This fails against the pre-fix code, which trusted
    /// `fixture.call`'s raw text verbatim and emitted the argument-less, plural,
    /// nonexistent `sample_clear_sample_backends()`.
    #[test]
    fn trait_bridge_operation_uses_declared_abi_identity() {
        let fixture = Fixture {
            id: "clear_sample_backends".into(),
            description: "Clear registered sample backends".into(),
            call: Some("clear_sample_backends".into()),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_sample_backends".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                clear_fn: Some("clear_sample_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("C snippet renders");

        assert!(rendered.contains("sample_clear_sample_backend(NULL)"), "{rendered}");
        assert!(!rendered.contains("sample_clear_sample_backends("), "{rendered}");
        assert!(!rendered.contains("has no function identity"), "{rendered}");
    }

    /// `unregister_fn`'s C export always takes a trailing `out_error` out-param
    /// (`unregister_fn.jinja`) in addition to the configured `name` argument, but the
    /// shared, language-agnostic call args config (`args = [{ name, field, type }]`)
    /// has no way to express a C-only out-param. This fails against the pre-fix code:
    /// the void-call branch built its argument list purely from `info.args` and never
    /// consulted `info.extra_args`, so it emitted `sample_unregister_sample_backend(name)`
    /// -- one argument short of the real two-argument ABI signature.
    #[test]
    fn trait_bridge_unregister_appends_out_error_out_param() {
        let fixture = Fixture {
            id: "unregister_sample_backend".into(),
            description: "Unregister a sample backend".into(),
            call: Some("unregister_sample_backend".into()),
            input: serde_json::json!({ "name": "nonexistent-backend" }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "unregister_sample_backend".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                args: vec![crate::core::config::e2e::ArgMapping {
                    name: "name".into(),
                    field: "input.name".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                }],
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                unregister_fn: Some("unregister_sample_backend".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("C snippet renders");

        assert!(
            rendered.contains("sample_unregister_sample_backend(\"nonexistent-backend\", NULL)"),
            "{rendered}"
        );
    }

    /// `resolve_fixture_call_info` must not trust `trait_bridge_function_identity`'s
    /// raw-config-text-derived symbol name for a fixture that declares
    /// `skip.languages = ["c"]` -- exactly the shape of the 13 fixtures fixed in
    /// `8ddaa0559` (via `src/e2e/snippets/mod.rs`'s equivalent guard). This exercises
    /// the resolver directly, independent of the prefixing/template logic that
    /// `render_c_snippet` layers on top, so a regression here is unambiguous: it can
    /// only mean the skip check stopped gating the fallback.
    #[test]
    fn resolve_fixture_call_info_ignores_naive_identity_when_skipped_for_lang() {
        let fixture = Fixture {
            id: "clear_sample_backends".into(),
            call: Some("clear_sample_backends".into()),
            skip: Some(crate::e2e::fixture::SkipDirective {
                languages: vec!["c".into()],
                reason: None,
            }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_sample_backends".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                clear_fn: Some("clear_sample_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let info = resolve_fixture_call_info(&fixture, &e2e, &config, "c", &[]);

        assert_eq!(
            info.function_name, "",
            "skip.languages = [\"c\"] must block the naive identity fallback, leaving no function \
             configured rather than a symbol name that may not exist"
        );
    }

    /// End-to-end counterpart of the resolver-level test above: a fixture skipped for
    /// `c` must not produce a snippet that calls the config-text-derived symbol name.
    /// `render_c_snippet` is exercised directly (not through
    /// `src/e2e/snippets/mod.rs`'s gate) so this proves the C generator's own
    /// invariant, not just the upstream caller's filtering.
    #[test]
    fn trait_bridge_operation_skipped_for_c_does_not_trust_naive_identity() {
        let fixture = Fixture {
            id: "clear_sample_backends".into(),
            description: "Clear registered sample backends".into(),
            call: Some("clear_sample_backends".into()),
            skip: Some(crate::e2e::fixture::SkipDirective {
                languages: vec!["c".into()],
                reason: Some("c FFI export does not match the configured clear_fn text".into()),
            }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_sample_backends".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                clear_fn: Some("clear_sample_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("C snippet renders");

        assert!(
            !rendered.contains("sample_clear_sample_backends("),
            "skipped fixture must not call the naive-identity symbol: {rendered}"
        );
        assert!(rendered.contains("sample_();"), "{rendered}");
    }

    #[test]
    fn expected_error_snippet_checks_the_native_null_result() {
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
        e2e.call.function = "sample_parse".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("snippet renders");
        assert!(rendered.contains("!= 0) { return EXIT_FAILURE; }"), "{rendered}");
        assert!(!rendered.contains("assert("));
    }

    #[test]
    fn engine_factory_snippet_reuses_native_call_preparation() {
        let fixture = Fixture {
            id: "engine_call".into(),
            description: "Engine call".into(),
            input: serde_json::json!({ "url": "https://example.test" }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_scrape".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                c_engine_factory: Some("EngineConfig".into()),
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("engine-factory snippet renders");

        assert!(rendered.contains("create_engine"), "{rendered}");
        assert!(rendered.contains("sample_scrape(engine"), "{rendered}");
        assert!(rendered.contains("crawl_engine_handle_free(engine)"), "{rendered}");
    }

    #[test]
    fn simple_result_snippet_uses_prefixed_string_api() {
        let fixture = Fixture {
            id: "list_formats".into(),
            description: "List formats".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "list_formats".into();
        e2e.call.result_var = "result".into();
        e2e.call.result_is_simple = true;
        e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                raw_c_result_type: Some("char*".into()),
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("simple-result snippet renders");

        assert!(rendered.contains("char* result = sample_list_formats();"), "{rendered}");
        assert!(rendered.contains("sample_free_string(result);"), "{rendered}");
        assert!(!rendered.contains("SAMPLEListFormats"), "{rendered}");
    }

    #[test]
    fn scalar_result_snippets_preserve_numeric_types_without_string_cleanup() {
        for raw_type in ["int32_t", "bool"] {
            let fixture = Fixture {
                id: "count_formats".into(),
                description: "Count formats".into(),
                ..Fixture::default()
            };
            let mut e2e = E2eConfig::default();
            e2e.call.function = "count_formats".into();
            e2e.call.result_var = "result".into();
            e2e.call.result_is_simple = true;
            e2e.call.overrides.insert(
                "c".into(),
                crate::core::config::e2e::CallOverride {
                    raw_c_result_type: Some(raw_type.into()),
                    ..Default::default()
                },
            );
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };

            let rendered = CCodegen
                .render_snippet_body(&fixture, &e2e, &config, &[], &[])
                .expect("numeric-result snippet renders");

            assert!(
                rendered.contains(&format!("{raw_type} result = sample_count_formats();")),
                "{rendered}"
            );
            assert!(!rendered.contains("free_string"), "{rendered}");
        }
    }

    #[test]
    fn raw_result_error_snippet_fails_on_unexpected_success() {
        for (raw_type, expected_failure_check) in [
            ("char*", "if (result != 0) { return EXIT_FAILURE; }"),
            ("int32_t", "if (result != 0) { return EXIT_FAILURE; }"),
            ("uintptr_t", "assert(sample_last_error_code() != 0"),
        ] {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let mut e2e = E2eConfig::default();
            e2e.call.function = "parse_input".into();
            e2e.call.result_var = "result".into();
            e2e.call.result_is_simple = true;
            e2e.call.overrides.insert(
                "c".into(),
                crate::core::config::e2e::CallOverride {
                    raw_c_result_type: Some(raw_type.into()),
                    ..Default::default()
                },
            );
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };

            let rendered = CCodegen
                .render_snippet_body(&fixture, &e2e, &config, &[], &[])
                .expect("raw-result error snippet renders");

            assert!(
                rendered.contains(expected_failure_check),
                "raw_type={raw_type}: {rendered}"
            );
        }
    }

    #[test]
    fn raw_result_test_function_asserts_failure_per_result_type() {
        // Direct test of the real e2e-test-file emitter (render_test_function),
        // which is where the defect lived: for raw_c_result_type functions
        // (char*/int32_t/uintptr_t), an "error"-only fixture previously emitted
        // no assertion at all, so a call that unexpectedly SUCCEEDED still made
        // the generated test pass. Assert the exact failing construct per type.
        let cases: &[(&str, &str)] = &[
            ("char*", "assert(result == NULL && \"expected call to fail\");"),
            ("int32_t", "assert(result < 0 && \"expected call to fail\");"),
            (
                "uintptr_t",
                "assert(sample_last_error_code() != 0 && \"expected call to fail\");",
            ),
        ];
        for (raw_type, expected_assert) in cases {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };
            let field_resolver = FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            );

            let mut out = String::new();
            render_test_function(
                &mut out,
                &fixture,
                "sample",
                "sample_parse_input",
                "result",
                &[],
                &field_resolver,
                &HashMap::new(),
                &HashSet::new(),
                "Result",
                "",
                None,
                Some(raw_type),
                None,
                None,
                false,
                false,
                None,
                &[],
                &config,
                &[],
                false,
            )
            .expect("test fixture renders");

            assert!(
                out.contains(expected_assert),
                "raw_type={raw_type}: expected `{expected_assert}` in:\n{out}"
            );
            assert!(
                !out.contains("expected call to succeed"),
                "raw_type={raw_type}: unexpected success-path assertion in:\n{out}"
            );
        }
    }

    #[test]
    fn raw_result_test_function_falls_back_to_last_error_code_for_unmodeled_raw_types() {
        // raw_c_result_type is a free-form config string (bool, uint64_t, size_t, ...),
        // not a closed char*/int32_t/uintptr_t set. A fixture using any type outside
        // that trio must still emit a failing check via the always-present
        // last_error_code FFI symbol — not silently emit nothing.
        for raw_type in ["bool", "uint64_t", "size_t"] {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };
            let field_resolver = FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            );

            let mut out = String::new();
            render_test_function(
                &mut out,
                &fixture,
                "sample",
                "sample_parse_input",
                "result",
                &[],
                &field_resolver,
                &HashMap::new(),
                &HashSet::new(),
                "Result",
                "",
                None,
                Some(raw_type),
                None,
                None,
                false,
                false,
                None,
                &[],
                &config,
                &[],
                false,
            )
            .expect("test fixture renders");

            assert!(
                out.contains("assert(sample_last_error_code() != 0 && \"expected call to fail\");"),
                "raw_type={raw_type}: expected last_error_code fallback assert in:\n{out}"
            );
        }
    }

    #[test]
    fn void_result_snippet_calls_api_without_placeholder_result() {
        let fixture = Fixture {
            id: "clear_formats".into(),
            description: "Clear formats".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "clear_formats".into();
        e2e.call.returns_void = true;
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("void-result snippet renders");

        assert!(rendered.contains("sample_clear_formats();"), "{rendered}");
        assert!(!rendered.contains("result ="), "{rendered}");
        assert!(!rendered.contains("_free("), "{rendered}");
    }
}

#[cfg(test)]
mod result_type_resolution_tests {
    use super::*;
    use crate::core::ir::{FunctionDef, TypeRef};

    fn call_named(function: &str) -> CallConfig {
        CallConfig {
            function: function.to_string(),
            ..CallConfig::default()
        }
    }

    fn function_returning(name: &str, return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            return_type,
            error_type: error_type.map(str::to_string),
            ..FunctionDef::default()
        }
    }

    /// The case that was silently wrong. The extractor splits `Result<T, E>` into
    /// `return_type = T` plus a separate `error_type`, so a fallible
    /// `pub fn complete(..) -> Result<CompletionResponse, String>` must resolve to
    /// `CompletionResponse` — never to `Complete`, the PascalCased call name, which is not a
    /// type at all and disables the nested-field walk that reads it.
    #[test]
    fn should_resolve_a_fallible_functions_result_type_to_its_ok_type() {
        let functions = vec![function_returning(
            "complete",
            TypeRef::Named("CompletionResponse".to_string()),
            Some("String"),
        )];

        assert_eq!(
            resolve_ir_result_type(&call_named("complete"), "c", &functions),
            Some("CompletionResponse".to_string())
        );
    }

    /// Control: the `Optional(Named)` shape the previous one-level match already handled must
    /// keep resolving to the same name.
    #[test]
    fn should_resolve_an_optional_named_return_type_unchanged() {
        let functions = vec![function_returning(
            "find_model",
            TypeRef::Optional(Box::new(TypeRef::Named("Model".to_string()))),
            Some("String"),
        )];

        assert_eq!(
            resolve_ir_result_type(&call_named("find_model"), "c", &functions),
            Some("Model".to_string())
        );
    }

    /// `Result<Vec<Model>, E>` answered `None` under the one-level match and fell through to
    /// the call-name fallback, even though the sibling `named_type` in this very module already
    /// unwrapped `Vec`.
    #[test]
    fn should_resolve_through_a_collection_return_type() {
        let functions = vec![function_returning(
            "list_models",
            TypeRef::Vec(Box::new(TypeRef::Named("Model".to_string()))),
            Some("String"),
        )];

        assert_eq!(
            resolve_ir_result_type(&call_named("list_models"), "c", &functions),
            Some("Model".to_string())
        );
    }

    /// A return type with no named type in it has no result type to name; the caller's
    /// fallback, not a wrong name, is the right answer here.
    #[test]
    fn should_not_invent_a_result_type_for_an_unnamed_return() {
        let functions = vec![function_returning("ping", TypeRef::Unit, None)];

        assert_eq!(resolve_ir_result_type(&call_named("ping"), "c", &functions), None);
    }

    /// The generated-test-file path passes an empty `functions` slice, and free-function IR
    /// never contains methods, so resolution cannot answer for either shape. The fallback is
    /// load-bearing for exactly those cases — this pins that it still produces the documented
    /// PascalCase name rather than failing generation.
    #[test]
    fn should_fall_back_to_the_pascal_cased_call_name_without_ir_functions() {
        assert_eq!(resolve_ir_result_type(&call_named("complete"), "c", &[]), None);
        assert_eq!(
            fallback_result_type_name(&call_named("complete"), "c", &[]),
            "Complete".to_string(),
            "the fallback must stay, and its output must stay the documented shape"
        );
    }

    /// End to end through `resolve_call_info`: with the IR threaded, the resolved type wins
    /// over the fallback; with no IR, the fallback still applies.
    #[test]
    fn should_prefer_the_resolved_result_type_over_the_call_name_fallback() {
        let functions = vec![function_returning(
            "complete",
            TypeRef::Named("CompletionResponse".to_string()),
            Some("String"),
        )];

        assert_eq!(
            resolve_call_info(&call_named("complete"), "c", &functions).result_type_name,
            "CompletionResponse".to_string()
        );
        assert_eq!(
            resolve_call_info(&call_named("complete"), "c", &[]).result_type_name,
            "Complete".to_string()
        );
    }
}
