use crate::backends::zig::gen_bindings::errors::resolve_zig_error_type;
use crate::backends::zig::gen_bindings::helpers::emit_cleaned_zig_doc;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::AsSnakeCase;

use super::render;

/// Emit a Zig struct type for a streaming iterator.
///
/// The struct holds a stream handle and provides `next()` and `deinit()` methods
/// to incrementally consume chunks without eagerly collecting them all into memory.
pub(super) fn emit_streaming_struct(
    method: &MethodDef,
    _ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    item_type: &str,
    declared_errors: &[String],
    out: &mut String,
) {
    let method_snake = AsSnakeCase(&method.name).to_string();
    let item_snake = AsSnakeCase(item_type).to_string();
    let struct_name = format!("{}Stream", item_type);

    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors))
        .unwrap_or_else(|| "anyerror".to_string());

    out.push_str(&render(
        "opaque_stream_struct.jinja",
        minijinja::context! {
            item_type => item_type,
            struct_name => &struct_name,
            zig_error_type => &zig_error_type,
            prefix => prefix,
            type_snake => type_snake,
            method_snake => &method_snake,
            item_snake => &item_snake,
        },
    ));
}

/// Emit a streaming method on an opaque handle wrapper struct.
///
/// Streaming methods use the iterator-handle pattern (`_start` / `_next` / `_free`)
/// and return a struct type that provides `next()` and `deinit()` methods for
/// incremental, backpressure-aware consumption. Callers can cancel by dropping
/// the struct early without draining the entire stream.
pub(super) fn emit_opaque_streaming_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    item_type: &str,
    declared_errors: &[String],
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let method_snake = AsSnakeCase(&method.name).to_string();
    let struct_name = format!("{}Stream", item_type);
    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors))
        .unwrap_or_else(|| "anyerror".to_string());

    let req_param = method.params.first().map(|p| p.name.as_str()).unwrap_or("req");
    let req_param_lower = req_param.to_lowercase();
    let req_type_snake = if let Some(p) = method.params.first() {
        if let TypeRef::Named(n) = &p.ty {
            AsSnakeCase(n).to_string()
        } else {
            "chat_completion_request".to_string()
        }
    } else {
        "chat_completion_request".to_string()
    };

    out.push_str(&render(
        "opaque_stream_method.jinja",
        minijinja::context! {
            method_name => &method.name,
            type_name => &ty.name,
            req_param => req_param,
            zig_error_type => &zig_error_type,
            struct_name => &struct_name,
            req_param_lower => &req_param_lower,
            prefix => prefix,
            req_type_snake => &req_type_snake,
            type_snake => type_snake,
            method_snake => &method_snake,
            c_handle => "handle",
        },
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn stream_template_checks_json_and_releases_every_native_value() {
        let rendered = crate::backends::zig::template_env::render(
            "opaque_stream_struct.jinja",
            minijinja::context! {
                struct_name => "RecordStream",
                item_type => "Record",
                zig_error_type => "RequestError",
                prefix => "sample",
                type_snake => "client",
                method_snake => "stream_records",
                item_snake => "record",
            },
        );

        assert!(rendered.contains("if (_json == null) return _error_with_message(RequestError)"));
        assert!(rendered.contains("defer c.sample_record_free(_chunk)"));
        assert!(rendered.contains("defer c.sample_free_string(_json)"));
        assert!(rendered.contains("pub fn deinit"));
        assert!(rendered.contains("sample_client_stream_records_free"));
        assert!(rendered.contains("_handle: u64"));
        assert!(rendered.contains("if (_chunk == 0)"));
        assert!(rendered.contains("self._handle = 0"));
    }
}
