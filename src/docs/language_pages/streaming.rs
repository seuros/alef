use crate::core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, TypeRef};
use crate::docs::doc_cleaning::{demote_headings_to_start_at, extract_param_docs};
use crate::docs::examples::MethodExampleOverride;
use crate::docs::examples::render_method_example_with_override;
use crate::docs::naming::{func_name, lang_code_fence, method_name, type_name};
use crate::docs::signatures::{MethodSignatureOverride, render_method_signature_with_override};
use crate::docs::{clean_doc, doc_type, template_env};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

use super::function_render::{push_errors, push_parameters_table, push_returns_with_override, push_version_annotation};

#[derive(Debug, Clone)]
pub(super) struct MethodDocsOverride {
    pub(super) heading_name: String,
    pub(super) signature: MethodSignatureOverride,
    pub(super) example: MethodExampleOverride,
    pub(super) return_type: String,
}

pub(super) fn streaming_method_docs_override(
    config: &ResolvedCrateConfig,
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
    ffi_prefix: &str,
    crate_name: &str,
) -> Option<MethodDocsOverride> {
    let adapter = config.adapters.iter().find(|adapter| {
        matches!(adapter.pattern, AdapterPattern::Streaming)
            && adapter.owner_type.as_deref() == Some(type_name_str)
            && !adapter.skip_languages.iter().any(|skip| skip == &lang.to_string())
            && streaming_adapter_matches_method(adapter, method)
    })?;
    let item_type = adapter.item_type.as_deref()?;
    let heading_name = streaming_method_name(adapter, method, lang, ffi_prefix);
    let signature =
        streaming_method_signature_override(adapter, method, type_name_str, item_type, lang, ffi_prefix, crate_name);
    let return_type = streaming_return_type(adapter, type_name_str, item_type, lang, ffi_prefix, true);
    let example = MethodExampleOverride {
        body: streaming_example(config, adapter, method, type_name_str, item_type, lang, ffi_prefix),
    };

    Some(MethodDocsOverride {
        heading_name,
        signature,
        example,
        return_type,
    })
}

pub(super) fn streaming_adapter_matches_method(adapter: &AdapterConfig, method: &MethodDef) -> bool {
    let method_name = method.name.to_snake_case();
    adapter.name.to_snake_case() == method_name
        || adapter
            .core_path
            .rsplit("::")
            .next()
            .is_some_and(|core_name| core_name.to_snake_case() == method_name)
}

pub(super) fn streaming_adapter_skips_method(
    config: &ResolvedCrateConfig,
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
) -> bool {
    config.adapters.iter().any(|adapter| {
        matches!(adapter.pattern, AdapterPattern::Streaming)
            && adapter.owner_type.as_deref() == Some(type_name_str)
            && adapter.skip_languages.iter().any(|skip| skip == &lang.to_string())
            && streaming_adapter_matches_method(adapter, method)
    })
}

pub(super) fn method_visible_in_lang(
    config: &ResolvedCrateConfig,
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
) -> bool {
    (lang == Language::Rust || !method.binding_excluded)
        && !streaming_adapter_skips_method(config, method, type_name_str, lang)
}

fn streaming_method_name(adapter: &AdapterConfig, method: &MethodDef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Csharp => {
            let base = func_name(&adapter.name, lang, ffi_prefix);
            if base.ends_with("Async") {
                base
            } else {
                format!("{base}Async")
            }
        }
        Language::Ffi | Language::C | Language::Jni => streaming_c_start_name(adapter, method, ffi_prefix),
        Language::Zig => adapter.name.to_snake_case(),
        _ => func_name(&adapter.name, lang, ffi_prefix),
    }
}

fn streaming_method_signature_override(
    adapter: &AdapterConfig,
    method: &MethodDef,
    type_name_str: &str,
    item_type: &str,
    lang: Language,
    ffi_prefix: &str,
    crate_name: &str,
) -> MethodSignatureOverride {
    let name = streaming_method_name(adapter, method, lang, ffi_prefix);
    let return_type = streaming_return_type(adapter, type_name_str, item_type, lang, ffi_prefix, false);
    let signature = match lang {
        Language::Python => Some(format!(
            "async def {}(self, req: {}) -> {}",
            adapter.name.to_snake_case(),
            first_param_type(method, lang, ffi_prefix),
            return_type
        )),
        Language::Rust => Some(format!(
            "fn {}(&self, req: {}) -> {}",
            adapter.name.to_snake_case(),
            first_param_type(method, Language::Rust, ffi_prefix),
            return_type
        )),
        // ~keep Named by `backends::java::naming::exception_class_name`, the same derivation
        // `JavaBackend::resolve_main_class` feeds into `<MainClass>Exception.java`. This arm
        // used to pascal-case the crate name itself, which is a third spelling of the class
        // alongside the non-streaming signature renderer's and the backend's.
        Language::Java => Some(format!(
            "public java.util.stream.Stream<{}> {}({} req) throws {}",
            type_name(item_type, lang, ffi_prefix),
            name,
            first_param_type(method, lang, ffi_prefix),
            crate::backends::java::naming::exception_class_name(crate_name)
        )),
        Language::Csharp => Some(format!(
            "public async IAsyncEnumerable<{}> {}({} req, CancellationToken cancellationToken = default)",
            type_name(item_type, lang, ffi_prefix),
            name,
            first_param_type(method, lang, ffi_prefix)
        )),
        Language::Swift => Some(format!(
            "public func {}(_ req: {}) async throws -> {}",
            name,
            first_param_type(method, lang, ffi_prefix),
            return_type
        )),
        // ~keep The rustler backend always names the receiver param `obj`, never `client`
        // (`gen_bindings/helpers/conversions.rs`'s `def_args.push("obj".to_string())`).
        Language::Elixir => Some(format!("def {}(obj, req)", adapter.name.to_snake_case())),
        Language::Ffi | Language::C | Language::Jni => Some(streaming_c_start_signature(
            adapter,
            method,
            type_name_str,
            item_type,
            ffi_prefix,
        )),
        Language::Zig => Some(format!(
            "pub fn {}(self: *{}, req: []const u8) {}",
            adapter.name.to_snake_case(),
            type_name(type_name_str, lang, ffi_prefix),
            streaming_zig_return_type(method, item_type, ffi_prefix)
        )),
        _ => None,
    };

    MethodSignatureOverride {
        name: Some(name),
        return_type: Some(return_type),
        signature,
    }
}

fn first_param_type(method: &MethodDef, lang: Language, ffi_prefix: &str) -> String {
    method
        .params
        .first()
        .map(|param| doc_type(&param.ty, lang, ffi_prefix))
        .unwrap_or_else(|| "void".to_string())
}

fn streaming_return_type(
    adapter: &AdapterConfig,
    type_name_str: &str,
    item_type: &str,
    lang: Language,
    ffi_prefix: &str,
    include_outer_result: bool,
) -> String {
    let item = type_name(item_type, lang, ffi_prefix);
    match lang {
        Language::Python => format!("AsyncIterator[{item}]"),
        Language::Node | Language::Wasm => {
            let iter = format!("{}Iterator", adapter.name.to_pascal_case());
            if include_outer_result {
                format!("Promise<{iter}>")
            } else {
                iter
            }
        }
        Language::Ruby => format!("{}Iterator", adapter.name.to_pascal_case()),
        Language::Php => "array<string>".to_string(),
        Language::Elixir => "{:ok, Stream.t()}".to_string(),
        Language::Go => {
            if include_outer_result {
                format!("(<-chan {item}, error)")
            } else {
                format!("<-chan {item}")
            }
        }
        Language::Java => format!("java.util.stream.Stream<{item}>"),
        Language::Csharp => format!("IAsyncEnumerable<{item}>"),
        Language::Rust => format!("BoxFuture<'_, Result<BoxStream<'static, Result<{item}>>>>"),
        Language::Kotlin => format!("Flow<{item}>"),
        Language::KotlinAndroid => format!("kotlinx.coroutines.flow.Flow<{item}>"),
        Language::Swift => format!("AsyncThrowingStream<{item}, Error>"),
        Language::Dart => format!("Stream<{item}>"),
        Language::Ffi | Language::C | Language::Jni => streaming_c_handle_type(adapter, type_name_str, ffi_prefix),
        Language::Zig => streaming_zig_return_type_placeholder(item_type, ffi_prefix),
        Language::R | Language::Gleam => item,
    }
}

fn streaming_zig_return_type_placeholder(item_type: &str, ffi_prefix: &str) -> String {
    format!("{}Stream", type_name(item_type, Language::Zig, ffi_prefix))
}

fn streaming_zig_return_type(method: &MethodDef, item_type: &str, ffi_prefix: &str) -> String {
    let stream_type = streaming_zig_return_type_placeholder(item_type, ffi_prefix);
    let error_type = method
        .error_type
        .as_deref()
        .map(|error| type_name(error, Language::Zig, ffi_prefix))
        .unwrap_or_else(|| "anyerror".to_string());
    format!("({error_type}||error{{OutOfMemory}})!{stream_type}")
}

fn streaming_c_start_name(adapter: &AdapterConfig, method: &MethodDef, ffi_prefix: &str) -> String {
    let _ = method;
    crate::codegen::c_consumer::stream_adapter_symbol(
        &ffi_prefix.to_snake_case(),
        adapter.owner_type.as_deref().unwrap_or_default(),
        &adapter.name,
        "start",
    )
}

/// The stream-handle struct is declared by alef itself, so its header name carries the prefix
/// twice: cbindgen's `[export] prefix` (shouty) in front of the PascalCase-prefixed Rust struct
/// name the FFI backend emits. For a crate whose `[ffi] prefix` is the single lowercase word
/// `samplecrate`, the consumer's header spells the handle
/// `SAMPLECRATESamplecrateDefaultClientChatStreamStreamHandle` — prefix shouted, then repeated
/// in PascalCase, before the type/method pair. ~keep
fn streaming_c_handle_type(adapter: &AdapterConfig, type_name_str: &str, ffi_prefix: &str) -> String {
    format!(
        "struct {}{}{}{}StreamHandle *",
        ffi_prefix.to_shouty_snake_case(),
        ffi_prefix.to_pascal_case(),
        type_name_str.to_pascal_case(),
        adapter.name.to_pascal_case()
    )
}

fn streaming_c_start_signature(
    adapter: &AdapterConfig,
    method: &MethodDef,
    type_name_str: &str,
    item_type: &str,
    ffi_prefix: &str,
) -> String {
    let handle_type = streaming_c_handle_type(adapter, type_name_str, ffi_prefix);
    let start_name = streaming_c_start_name(adapter, method, ffi_prefix);
    // ~keep The client and request params are scalar `AlefHandle` tokens under the
    // handle-ABI migration (see FFI_HANDLE_TYPE_NAME in type_mapping.rs), not pointers
    // to opaque structs -- `doc_type` already renders `TypeRef::Named` that way for Ffi/C.
    let owner_type = doc_type(&TypeRef::Named(type_name_str.to_string()), Language::Ffi, ffi_prefix);
    let request_param = method
        .params
        .first()
        .map(|param| match &param.ty {
            TypeRef::Named(_) => format!("{} req", doc_type(&param.ty, Language::Ffi, ffi_prefix)),
            _ => "const void *req".to_string(),
        })
        .unwrap_or_else(|| "const void *req".to_string());
    let _ = item_type;
    format!("{handle_type} {start_name}({owner_type} client, {request_param});")
}

fn streaming_example(
    config: &ResolvedCrateConfig,
    adapter: &AdapterConfig,
    method: &MethodDef,
    type_name_str: &str,
    item_type: &str,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let method_name = streaming_method_name(adapter, method, lang, ffi_prefix);
    let req_value = streaming_request_sample(method, lang, ffi_prefix);
    let item = type_name(item_type, lang, ffi_prefix);
    match lang {
        Language::Python => {
            format!("stream = instance.{method_name}({req_value})\nasync for chunk in stream:\n    print(chunk)")
        }
        Language::Node => format!(
            "const stream = await instance.{method_name}({req_value});\nfor await (const chunk of stream) {{\n  console.log(chunk);\n}}"
        ),
        Language::Wasm => format!(
            "const stream = await instance.{method_name}({req_value});\nwhile (true) {{\n  const chunk = await stream.next();\n  if (chunk === null) break;\n  console.log(chunk);\n}}"
        ),
        Language::Ruby => {
            format!("stream = instance.{method_name}({req_value})\nstream.each do |chunk|\n  puts chunk\nend")
        }
        Language::Php => {
            format!("foreach ($instance->{method_name}({req_value}) as $chunk) {{\n    var_dump($chunk);\n}}")
        }
        Language::Elixir => format!(
            "{{:ok, stream}} = {}.{}(instance, {req_value})\nEnum.each(stream, &IO.inspect/1)",
            config.name.to_pascal_case(),
            adapter.name.to_snake_case()
        ),
        Language::Go => format!(
            "stream, err := instance.{method_name}({req_value})\nif err != nil {{\n    return err\n}}\nfor chunk := range stream {{\n    fmt.Println(chunk)\n}}"
        ),
        Language::Java => format!(
            "try (var stream = instance.{method_name}({req_value})) {{\n    stream.forEach(System.out::println);\n}}"
        ),
        Language::Csharp => format!(
            "await foreach (var chunk in instance.{method_name}({req_value})) {{\n    Console.WriteLine(chunk);\n}}"
        ),
        Language::Rust => format!(
            "let mut stream = instance.{}({req_value}).await?;\nwhile let Some(chunk) = stream.next().await {{\n    let chunk = chunk?;\n    println!(\"{{chunk:?}}\");\n}}",
            adapter.name.to_snake_case()
        ),
        Language::Kotlin | Language::KotlinAndroid => {
            format!("instance.{method_name}({req_value}).collect {{ chunk ->\n    println(chunk)\n}}")
        }
        Language::Swift => format!(
            "let stream = try await instance.{method_name}({req_value})\nfor try await chunk in stream {{\n    print(chunk)\n}}"
        ),
        Language::Dart => {
            format!("await for (final chunk in instance.{method_name}({req_value})) {{\n  print(chunk);\n}}")
        }
        Language::Ffi | Language::C | Language::Jni => {
            streaming_c_example(adapter, method, type_name_str, item_type, ffi_prefix)
        }
        Language::Zig => format!(
            "var stream = try instance.{method_name}(\"{{}}\");\ndefer stream.deinit();\nwhile (try stream.next()) |chunk| {{\n    _ = chunk;\n}}"
        ),
        Language::R | Language::Gleam => {
            format!("stream <- instance.{method_name}({req_value})\n# Iterate over {item} chunks.")
        }
    }
}

fn streaming_request_sample(method: &MethodDef, lang: Language, ffi_prefix: &str) -> String {
    let Some(param) = method.params.first() else {
        return String::new();
    };
    match &param.ty {
        TypeRef::Named(name) => {
            let ty = type_name(name, lang, ffi_prefix);
            match lang {
                Language::Python | Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart => {
                    format!("{ty}()")
                }
                Language::Node | Language::Wasm | Language::Java | Language::Csharp | Language::Php => {
                    format!("new {ty}()")
                }
                Language::Ruby => format!("{ty}.new"),
                Language::Go => format!("{ty}{{}}"),
                Language::Rust => format!("{ty}::default()"),
                Language::Zig => "\"{}\"".to_string(),
                Language::Elixir => "%{}".to_string(),
                Language::Ffi | Language::C | Language::Jni => "req".to_string(),
                Language::R | Language::Gleam => "{}".to_string(),
            }
        }
        _ => "req".to_string(),
    }
}

fn streaming_c_example(
    adapter: &AdapterConfig,
    method: &MethodDef,
    type_name_str: &str,
    item_type: &str,
    ffi_prefix: &str,
) -> String {
    let start_name = streaming_c_start_name(adapter, method, ffi_prefix);
    let handle_type = streaming_c_handle_type(adapter, type_name_str, ffi_prefix);
    let prefix = ffi_prefix.to_snake_case();
    // Spelled exactly as `streaming_c_start_name` spells it, so `_start`, `_next` and `_free`
    // on one page cannot name three different owners. The fallback is unreachable rather than
    // merely unlikely: `streaming_method_docs_override` only selects an adapter whose
    // `owner_type` is `Some(type_name_str)`, and the FFI backend skips an adapter without one
    // entirely (`lib_rs.rs` `continue`s past it), so an ownerless adapter has no C streaming
    // symbols for either side to name. ~keep
    let owner_type = adapter.owner_type.as_deref().unwrap_or_default();
    let next_name = crate::codegen::c_consumer::stream_adapter_symbol(&prefix, owner_type, &adapter.name, "next");
    let free_name = crate::codegen::c_consumer::stream_adapter_symbol(&prefix, owner_type, &adapter.name, "free");
    let item_c = type_name(item_type, Language::Ffi, ffi_prefix);
    // `item_free` is a different symbol family (the item type's own destructor), not a
    // streaming-adapter operation -- it stays a hand-built `{prefix}_{item_type}_free`
    // name. ~keep
    let item_free = format!("{}_{}_free", prefix, item_type.to_snake_case());
    format!(
        "{handle_type} stream = {start_name}(instance, req);\nwhile (stream != NULL) {{\n    {item_c} *chunk = {next_name}(stream);\n    if (chunk == NULL) {{\n        break;\n    }}\n    {item_free}(chunk);\n}}\n{free_name}(stream);"
    )
}

pub(super) fn render_method(
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
    config: &ResolvedCrateConfig,
    ffi_prefix: &str,
    crate_name: &str,
    api: &ApiSurface,
) -> String {
    let mut out = String::new();
    let docs_override = streaming_method_docs_override(config, method, type_name_str, lang, ffi_prefix, crate_name);
    let mname = docs_override
        .as_ref()
        .map(|override_| override_.heading_name.clone())
        .unwrap_or_else(|| method_name(type_name_str, &method.name, lang, ffi_prefix));

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "######", title => format!("{mname}()") },
    ));

    push_version_annotation(&mut out, &method.version);

    let param_docs = extract_param_docs(&method.doc);

    let doc = clean_doc(&method.doc, lang);
    // Nest under the `######` heading emitted just above, rather than shifting by a fixed
    // number of levels. A fixed `+2` assumes the doc comment starts at `#`, and a section that
    // starts anywhere else lands ABOVE its own parent: a rustdoc `# Observability` surfaced as
    // `###` under a `####` item, so it read as a sibling of the page's `### Functions` section and
    // took a bogus entry in the table of contents with it. ~keep
    let doc = demote_headings_to_start_at(&doc, 6);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let lang_code = lang_code_fence(lang);
    let sig = render_method_signature_with_override(
        method,
        type_name_str,
        lang,
        ffi_prefix,
        crate_name,
        docs_override.as_ref().map(|override_| &override_.signature),
        api,
    );
    out.push_str("**Signature:**\n\n");
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));
    out.push('\n');

    out.push_str(&render_method_example_with_override(
        method,
        type_name_str,
        lang,
        ffi_prefix,
        docs_override.as_ref().map(|override_| &override_.example),
    ));
    push_parameters_table(&mut out, &method.params, &param_docs, lang, ffi_prefix, api);
    push_returns_with_override(
        &mut out,
        &method.return_type,
        docs_override.as_ref().map(|override_| override_.return_type.as_str()),
        method.error_type.as_deref(),
        lang,
        ffi_prefix,
        api,
    );
    push_errors(
        &mut out,
        method.error_type.as_deref(),
        &method.return_type,
        lang,
        crate_name,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::AdapterParam;
    use crate::docs::test_helpers::{TEST_CRATE_NAME, TEST_PREFIX, make_method, make_param};

    fn make_adapter(name: &str, owner: &str, item_type: &str) -> AdapterConfig {
        AdapterConfig {
            name: name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("sample_crate::{name}"),
            params: vec![AdapterParam {
                name: "req".to_string(),
                ty: "sample_crate::StreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: Some("String".to_string()),
            owner_type: Some(owner.to_string()),
            item_type: Some(item_type.to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample_crate::StreamRequest".to_string()),
            skip_languages: vec![],
        }
    }

    /// ~keep Regression pin for the `stream_adapter_symbol` rewire in task #67: the old
    /// hand-rolled `format!("{prefix}_{owner}_{name}_start", ...)` and the shared
    /// `codegen::c_consumer::stream_adapter_symbol` produce the identical string for every
    /// owner/adapter-name shape configured today (see naming.rs's
    /// `type_component_agrees_with_the_streaming_backends_heck_conversion`), so this pins that
    /// the rewire really is a no-op rename, not a silent respelling.
    #[test]
    fn test_streaming_c_start_name_unchanged_by_the_stream_adapter_symbol_rewire() {
        let adapter = make_adapter("chat_stream", "DefaultClient", "ChatChunk");
        let method = make_method("chat_stream", vec![], TypeRef::Unit, false, false, None);
        let name = streaming_c_start_name(&adapter, &method, TEST_PREFIX);
        assert_eq!(name, "htm_default_client_chat_stream_start");
    }

    /// Same pin, for the `next`/`free` symbols `streaming_c_example` now derives through
    /// `stream_adapter_symbol` instead of an inline `format!`. `item_free` is deliberately
    /// left out of the rewire (a different symbol family, the item type's own destructor) and
    /// is asserted here unchanged too, so a future edit cannot "fix" it by accident.
    #[test]
    fn test_streaming_c_example_unchanged_by_the_stream_adapter_symbol_rewire() {
        let adapter = make_adapter("chat_stream", "DefaultClient", "ChatChunk");
        let method = make_method("chat_stream", vec![], TypeRef::Unit, false, false, None);
        let example = streaming_c_example(&adapter, &method, "DefaultClient", "ChatChunk", TEST_PREFIX);
        let expected = "struct HTMHtmDefaultClientChatStreamStreamHandle * stream = \
                         htm_default_client_chat_stream_start(instance, req);\n\
                         while (stream != NULL) {\n    \
                         HTMChatChunk *chunk = htm_default_client_chat_stream_next(stream);\n    \
                         if (chunk == NULL) {\n        break;\n    }\n    \
                         htm_chat_chunk_free(chunk);\n\
                         }\n\
                         htm_default_client_chat_stream_free(stream);";
        assert_eq!(example, expected);
    }

    /// The third surface in the "one symbol, three renderers" agreement -- the heading
    /// `render_method` prints for a C method must be the same `method_name` the signature
    /// (`signatures.rs`) and example (`examples.rs`) use. Pinned here rather than alongside
    /// the other two in `signatures/method_signatures.rs` because `render_method` is
    /// `pub(super)` to `language_pages` and is not reachable from that test module.
    #[test]
    fn test_render_method_heading_uses_the_same_c_symbol_as_signature_and_example() {
        let method = make_method(
            "convert",
            vec![make_param("options", TypeRef::Named("ParseOptions".to_string()), false)],
            TypeRef::Named("ConversionResult".to_string()),
            false,
            false,
            None,
        );
        let config = ResolvedCrateConfig::default();
        let doc = render_method(
            &method,
            "Converter",
            Language::C,
            &config,
            TEST_PREFIX,
            TEST_CRATE_NAME,
            &ApiSurface::default(),
        );
        let expected_symbol = method_name("Converter", &method.name, Language::C, TEST_PREFIX);
        assert_eq!(expected_symbol, "htm_converter_convert");
        assert!(
            doc.contains(&format!("###### {expected_symbol}()")),
            "heading must title the method with the same C symbol the signature and example \
             use: {doc}"
        );
    }
}
