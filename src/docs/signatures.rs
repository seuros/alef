use crate::core::config::Language;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};
use crate::docs::naming::{field_name, func_name, to_camel_case, type_name};
use crate::docs::type_mapping::doc_type;
use heck::{ToPascalCase, ToSnakeCase};

pub(crate) fn render_function_signature(func: &FunctionDef, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => render_python_fn_sig(func, ffi_prefix),
        Language::Node | Language::Wasm => render_typescript_fn_sig(func, ffi_prefix),
        Language::Go => render_go_fn_sig(func, ffi_prefix),
        Language::Java => render_java_fn_sig(func, ffi_prefix),
        Language::Ruby => render_ruby_fn_sig(func),
        Language::Ffi | Language::C | Language::Jni => render_c_fn_sig(func, ffi_prefix),
        Language::Php => render_php_fn_sig(func, ffi_prefix),
        Language::Elixir => render_elixir_fn_sig(func),
        Language::R => render_r_fn_sig(func),
        Language::Csharp => render_csharp_fn_sig(func, ffi_prefix),
        Language::Rust => render_rust_fn_sig(func, ffi_prefix),
        Language::Kotlin | Language::KotlinAndroid => render_kotlin_fn_sig(func, ffi_prefix),
        Language::Swift => render_swift_fn_sig(func, ffi_prefix),
        Language::Dart => render_dart_fn_sig(func, ffi_prefix),
        Language::Zig => render_zig_fn_sig(func, ffi_prefix),
        Language::Gleam => {
            format!("// Phase 1: {lang} backend signature generation")
        }
    }
}

pub(crate) fn render_python_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Python, "a function signature");
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Python, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty} = None")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Python, ffi_prefix);
    format!("def {}({}) -> {}", name, params.join(", "), ret)
}

pub(crate) fn render_typescript_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Node, "a function signature");
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Node, ffi_prefix);
            if p.optional {
                format!("{pname}?: {pty}")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Node, ffi_prefix);
    if func.is_async {
        format!("function {}({}): Promise<{}>", name, params.join(", "), ret)
    } else {
        format!("function {}({}): {}", name, params.join(", "), ret)
    }
}

/// ~keep Real Go bindings always pointer-wrap a `TypeRef::Named` return, fallible or not --
/// `gen_method_wrapper` (backends/go/gen_bindings/methods.rs) and `gen_function_wrapper`
/// (.../functions.rs) both route any non-Primitive/Duration/String/Char/Path return through
/// `go_optional_type` (backends/go/type_map.rs), which pointer-wraps `Named`. `doc_type`'s Go
/// arm renders the bare type name -- correct for a *type page* heading (Go structs are still
/// named after the Rust type) -- so the pointer belongs at the call site rendering a
/// *signature*, not in `doc_type` itself. Verified from source, not the constructor-shape
/// discrepancy: this applies to every Go method/function returning a Named type, not just
/// constructors -- opaque-handle params get the same treatment conditionally on opacity,
/// which `doc_type` cannot determine and is therefore NOT modeled here; see the docs-writer
/// report for that gap.
fn go_return_type(return_type: &TypeRef, ret: String) -> String {
    if matches!(return_type, TypeRef::Named(_)) {
        format!("*{ret}")
    } else {
        ret
    }
}

pub(crate) fn render_go_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_pascal_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Go, "a function signature");
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Go, ffi_prefix);
            format!("{pname} {pty}")
        })
        .collect();
    let ret = go_return_type(&func.return_type, doc_type(&func.return_type, Language::Go, ffi_prefix));
    if func.error_type.is_some() {
        if ret.is_empty() {
            format!("func {}({}) error", name, params.join(", "))
        } else {
            format!("func {}({}) ({}, error)", name, params.join(", "), ret)
        }
    } else if ret.is_empty() {
        format!("func {}({})", name, params.join(", "))
    } else {
        format!("func {}({}) {}", name, params.join(", "), ret)
    }
}

pub(crate) fn render_java_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Java, "a function signature");
    let ret = doc_type(&func.return_type, Language::Java, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Java, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    // ~keep Every generated Java free function crosses the FFI boundary through
    // `emit_method_header` (backends/java/gen_bindings/ffi_class/sync_functions.rs), which
    // renders `ffi_method_signature.jinja` -- `throws {{ exception_class }}` -- unconditionally,
    // with no `error_type`-gated branch. The FFI crossing itself (marshaling, allocation) can
    // fail even when the wrapped Rust function is infallible, so `func.error_type` (a fact
    // about the *core* Rust signature) is the wrong oracle for this clause.
    let throws = format!(" throws {ffi_prefix}RsException");
    format!("public static {} {}({}){}", ret, name, params.join(", "), throws)
}

pub(crate) fn render_ruby_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Ruby, "a function signature");
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            if p.optional { format!("{pname}: nil") } else { pname }
        })
        .collect();
    format!("def self.{}({})", name, params.join(", "))
}

pub(crate) fn render_c_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let prefix = ffi_prefix.to_snake_case();
    let name = format!("{}_{}", prefix, func.name.to_snake_case());
    let ret = doc_type(&func.return_type, Language::Ffi, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Ffi, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    // ~keep `doc_type` already renders `TypeRef::Named` as the scalar `AlefHandle` token
    // for Ffi/C (see type_mapping.rs), so no pointer suffix belongs here -- adding one
    // was the bug that put a `TYPE*` signature above an `AlefHandle result = ...` example.
    //
    // A fallible function whose logical return type is `()` has no value slot left to
    // signal failure through, so the FFI backend repurposes the return itself as a status
    // code: `gen_function_wrapper_footer`/`gen_free_function` (backends/ffi/gen_bindings/
    // functions/orchestration.rs) emit `i32` -- not `void` -- whenever
    // `has_error && is_void_return(&func.return_type)`, which cbindgen renders as
    // `int32_t`. Documenting `void` there tells a caller they can skip the check.
    let ret_str = match &func.return_type {
        TypeRef::Unit if func.error_type.is_some() => "int32_t".to_string(),
        TypeRef::Unit => "void".to_string(),
        _ => ret,
    };
    format!("{} {}({});", ret_str, name, params.join(", "))
}

pub(crate) fn render_php_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Php, "a function signature");
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = format!("${}", to_camel_case(&p.name));
            let pty = doc_type(&p.ty, Language::Php, ffi_prefix);
            if p.optional {
                format!("?{pty} {pname} = null")
            } else {
                format!("{pty} {pname}")
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Php, ffi_prefix);
    format!("public static function {}({}): {}", name, params.join(", "), ret)
}

pub(crate) fn render_elixir_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Elixir, "a function signature");
    let params: Vec<String> = func.params.iter().map(|p| p.name.to_snake_case()).collect();
    format!(
        "@spec {}({}) :: {{:ok, term()}} | {{:error, term()}}\ndef {}({})",
        name,
        params.join(", "),
        name,
        params.join(", ")
    )
}

pub(crate) fn render_r_fn_sig(func: &FunctionDef) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            if p.optional { format!("{pname} = NULL") } else { pname }
        })
        .collect();
    format!("{}({})", name, params.join(", "))
}

pub(crate) fn render_csharp_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_pascal_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Csharp, "a function signature");
    let ret = doc_type(&func.return_type, Language::Csharp, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Csharp, ffi_prefix);
            if p.optional {
                format!("{pty}? {pname} = null")
            } else {
                format!("{pty} {pname}")
            }
        })
        .collect();
    if func.is_async {
        let async_name = if name.ends_with("Async") {
            name.clone()
        } else {
            format!("{name}Async")
        };
        let task_ret = if ret == "void" {
            "Task".to_string()
        } else {
            format!("Task<{ret}>")
        };
        format!("public static async {} {}({})", task_ret, async_name, params.join(", "))
    } else {
        format!("public static {} {}({})", ret, name, params.join(", "))
    }
}

pub(crate) fn render_rust_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Rust, ffi_prefix);
            if p.optional {
                format!("{pname}: Option<{pty}>")
            } else {
                match &p.ty {
                    TypeRef::String | TypeRef::Char => format!("{pname}: &str"),
                    TypeRef::Bytes => format!("{pname}: &[u8]"),
                    _ => format!("{pname}: {pty}"),
                }
            }
        })
        .collect();
    let ret = doc_type(&func.return_type, Language::Rust, ffi_prefix);
    let error_part = if let Some(err) = &func.error_type {
        let err_ty = type_name(err, Language::Rust, ffi_prefix);
        if ret == "()" {
            format!(" -> Result<(), {err_ty}>")
        } else {
            format!(" -> Result<{ret}, {err_ty}>")
        }
    } else if ret == "()" {
        String::new()
    } else {
        format!(" -> {ret}")
    };
    if func.is_async {
        format!("pub async fn {}({}){}", name, params.join(", "), error_part)
    } else {
        format!("pub fn {}({}){}", name, params.join(", "), error_part)
    }
}

pub(crate) fn render_kotlin_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Kotlin, "a function signature");
    let ret = doc_type(&func.return_type, Language::Kotlin, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Kotlin, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty}? = null")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let throws = func
        .error_type
        .as_ref()
        .map(|e| format!("@Throws({}::class)\n", type_name(e, Language::Kotlin, ffi_prefix)))
        .unwrap_or_default();
    let ret_part = if ret == "Unit" {
        String::new()
    } else {
        format!(": {ret}")
    };
    format!("{throws}fun {name}({}){ret_part}", params.join(", "))
}

pub(crate) fn render_swift_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Swift, "a function signature");
    let ret = doc_type(&func.return_type, Language::Swift, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Swift, ffi_prefix);
            if p.optional {
                format!("{pname}: {pty}? = nil")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let throws = if func.error_type.is_some() { " throws" } else { "" };
    let ret_part = if ret == "Void" {
        String::new()
    } else {
        format!(" -> {ret}")
    };
    format!("public static func {name}({}){throws}{ret_part}", params.join(", "))
}

pub(crate) fn render_dart_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = to_camel_case(&func.name);
    crate::docs::formatting::assert_valid_identifier(&name, Language::Dart, "a function signature");
    let ret = doc_type(&func.return_type, Language::Dart, ffi_prefix);
    let required: Vec<String> = func
        .params
        .iter()
        .filter(|p| !p.optional)
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Dart, ffi_prefix);
            format!("{pty} {pname}")
        })
        .collect();
    let optional: Vec<String> = func
        .params
        .iter()
        .filter(|p| p.optional)
        .map(|p| {
            let pname = to_camel_case(&p.name);
            let pty = doc_type(&p.ty, Language::Dart, ffi_prefix);
            format!("{pty}? {pname}")
        })
        .collect();
    // ~keep Real Dart params are grouped exactly as `emit_function`'s `params_str` match
    // (backends/dart/gen_bindings/functions.rs) does: required positional params joined with
    // any optional ones wrapped in `{}` (Dart named-optional syntax), never `[]`
    // (positional-optional) -- a caller following a `[]`-bracketed doc could not call the
    // named form the real binding actually exposes.
    let params_str = match (required.is_empty(), optional.is_empty()) {
        (_, true) => required.join(", "),
        (true, false) => format!("{{{}}}", optional.join(", ")),
        (false, false) => format!("{}, {{{}}}", required.join(", "), optional.join(", ")),
    };
    // ~keep Every generated Dart free function is `Future<T>` (`Future<void>` for a `Unit`
    // return), never a bare `T` -- `emit_function`'s return-type branch
    // (backends/dart/gen_bindings/functions.rs) is unconditional, with no `is_async` check:
    // flutter_rust_bridge dispatches every non-`#[frb(sync)]` call across the FFI boundary
    // asynchronously, regardless of whether the wrapped Rust function is itself `async fn`.
    let future_ret = if ret == "void" {
        "Future<void>".to_string()
    } else {
        format!("Future<{ret}>")
    };
    format!("{future_ret} {name}({params_str})")
}

pub(crate) fn render_zig_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func.name.to_snake_case();
    crate::docs::formatting::assert_valid_identifier(&name, Language::Zig, "a function signature");
    let ret = doc_type(&func.return_type, Language::Zig, ffi_prefix);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = doc_type(&p.ty, Language::Zig, ffi_prefix);
            if p.optional {
                format!("{pname}: ?{pty}")
            } else {
                format!("{pname}: {pty}")
            }
        })
        .collect();
    let ret_str = if let Some(err) = &func.error_type {
        let err_ty = type_name(err, Language::Zig, ffi_prefix);
        if ret == "void" {
            format!("{err_ty}!void")
        } else {
            format!("{err_ty}!{ret}")
        }
    } else {
        ret
    };
    format!("pub fn {name}({}) {ret_str}", params.join(", "))
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MethodSignatureOverride {
    pub(crate) name: Option<String>,
    pub(crate) return_type: Option<String>,
    pub(crate) signature: Option<String>,
}

#[allow(dead_code)]
pub(crate) fn render_method_signature(
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    render_method_signature_with_override(method, type_name_str, lang, ffi_prefix, None)
}

pub(crate) fn render_method_signature_with_override(
    method: &MethodDef,
    type_name_str: &str,
    lang: Language,
    ffi_prefix: &str,
    signature_override: Option<&MethodSignatureOverride>,
) -> String {
    if let Some(signature) = signature_override.and_then(|override_| override_.signature.as_deref()) {
        return signature.to_string();
    }

    let name = signature_override
        .and_then(|override_| override_.name.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| func_name(&method.name, lang, ffi_prefix));
    // ~keep Every documented method name must be a legal identifier in `lang` -- reject a
    // reserved-word collision (Java/Dart's `new`, etc.) rather than silently document code
    // that would not compile. See formatting.rs's `assert_valid_identifier`.
    crate::docs::formatting::assert_valid_identifier(&name, lang, "a method signature");
    let overridden_ret = signature_override.and_then(|override_| override_.return_type.as_deref());
    // ~keep An explicitly overridden return type is already the backend-accurate spelling (the
    // streaming adapters supply Dart `Stream<T>`, Swift `AsyncThrowingStream<T, Error>`, ...), so
    // the per-language async wrappers below must not re-wrap it into `Future<Stream<T>>`. Only an
    // IR-derived return type needs that wrapping.
    let ret_is_overridden = overridden_ret.is_some();
    let ret = overridden_ret
        .map(str::to_string)
        .unwrap_or_else(|| doc_type(&method.return_type, lang, ffi_prefix));

    match lang {
        Language::Python => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = field_name(&p.name, lang);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname}: {pty}")
                })
                .collect();
            if method.is_static {
                format!("@staticmethod\ndef {}({}) -> {}", name, params.join(", "), ret)
            } else {
                let mut all_params = vec!["self".to_string()];
                all_params.extend(params);
                format!("def {}({}) -> {}", name, all_params.join(", "), ret)
            }
        }
        Language::Node | Language::Wasm => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = field_name(&p.name, lang);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname}: {pty}")
                })
                .collect();
            let ret = if method.is_async {
                format!("Promise<{ret}>")
            } else {
                ret
            };
            if method.is_static {
                format!("static {}({}): {}", name, params.join(", "), ret)
            } else {
                format!("{}({}): {}", name, params.join(", "), ret)
            }
        }
        Language::Ruby => {
            let params: Vec<String> = method.params.iter().map(|p| p.name.to_snake_case()).collect();
            if method.is_static {
                format!("def self.{}({})", name, params.join(", "))
            } else {
                format!("def {}({})", name, params.join(", "))
            }
        }
        Language::Go => {
            let go_receiver_type = type_name(type_name_str, Language::Go, ffi_prefix);
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pname} {pty}")
                })
                .collect();
            // ~keep A Named return is always pointer-wrapped in real Go, fallible or not --
            // see `go_return_type`'s doc comment for the source citation. Skipped when a
            // curated override already supplied the return text (streaming.rs) -- that
            // string is trusted as-is, not reinterpreted from `method.return_type`'s shape.
            let has_return_override = signature_override.and_then(|o| o.return_type.as_deref()).is_some();
            let ret = if has_return_override {
                ret
            } else {
                go_return_type(&method.return_type, ret)
            };
            // ~keep A static Go method is a free function `func {Type}{Method}(...)`, never
            // a method with a receiver -- see method_signature_static.jinja vs
            // method_signature_instance.jinja (backends/go/templates/). Rendering every Go
            // method with `func (o *Type) Method(...)` regardless of `is_static` documented
            // a receiver parameter that does not exist on the real generated function --
            // the same "IR through a per-language template that never checked a flag" defect
            // that broke the C signatures, here fabricating a receiver instead of a pointer.
            let head = if method.is_static {
                format!("func {go_receiver_type}{name}")
            } else {
                format!("func (o *{go_receiver_type}) {name}")
            };
            if method.error_type.is_some() {
                if ret.is_empty() {
                    format!("{head}({}) error", params.join(", "))
                } else {
                    format!("{head}({}) ({}, error)", params.join(", "), ret)
                }
            } else if ret.is_empty() {
                format!("{head}({})", params.join(", "))
            } else {
                format!("{head}({}) {}", params.join(", "), ret)
            }
        }
        Language::Java => {
            // ~keep Java's keyword renames are applied once, by `func_name`, which mirrors the
            // backend's `safe_java_method_name` (`default` -> `defaultInstance`, `new` ->
            // `create`). A second rename here used to map `default` -> `defaultOptions` and
            // silently won, emitting a name the Java backend never generates.
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            // ~keep Every generated Java method -- instance or static factory, fallible or
            // not -- crosses the FFI boundary through `emit_instance_method_header` /
            // `emit_static_factory_header` (gen_bindings/types/opaque/{instance,extended}.rs),
            // which append `throws {main_class}Exception` unconditionally except for `clone`.
            // The FFI crossing itself (marshaling, allocation) can fail even when the wrapped
            // Rust method returns a bare `T`, so `method.error_type` (a fact about the *core*
            // Rust signature) is the wrong oracle here -- it must always be present and must
            // always name the FFI exception, never the domain error type.
            let throws = if method.name == "clone" {
                String::new()
            } else {
                format!(" throws {ffi_prefix}RsException")
            };
            if method.is_static {
                format!("public static {} {}({}){}", ret, name, params.join(", "), throws)
            } else {
                format!("public {} {}({}){}", ret, name, params.join(", "), throws)
            }
        }
        Language::Csharp => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            if method.is_async {
                let async_name = if name.ends_with("Async") {
                    name.clone()
                } else {
                    format!("{name}Async")
                };
                let task_ret = if ret == "void" {
                    "Task".to_string()
                } else {
                    format!("Task<{ret}>")
                };
                format!("public async {} {}({})", task_ret, async_name, params.join(", "))
            } else {
                format!("public {} {}({})", ret, name, params.join(", "))
            }
        }
        Language::Php => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = format!("${}", to_camel_case(&p.name));
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            if method.is_static {
                format!("public static function {}({}): {}", name, params.join(", "), ret)
            } else {
                format!("public function {}({}): {}", name, params.join(", "), ret)
            }
        }
        Language::Elixir => {
            // ~keep An instance Elixir method takes the struct as an explicit leading `obj`
            // parameter -- rustler's codegen (`gen_bindings/helpers/conversions.rs`) pushes
            // `"obj"` onto `def_args` whenever `method.receiver.is_some()`, and the jinja
            // template emits `def {{ method_name }}({{ def_args }})` from that list verbatim --
            // there is no implicit `self` the way Ruby/Python have one. `method.is_static` is a
            // safe proxy for `receiver.is_some()`: both derive from the same `detect_receiver()`
            // call (extract/extractor/functions/methods.rs), so a static method (no receiver)
            // must not get the `obj` param, exactly mirroring the real generator.
            let mut params: Vec<String> = if method.is_static {
                Vec::new()
            } else {
                vec!["obj".to_string()]
            };
            params.extend(method.params.iter().map(|p| p.name.to_snake_case()));
            format!("def {}({})", name, params.join(", "))
        }
        Language::R => {
            let params: Vec<String> = method.params.iter().map(|p| p.name.to_snake_case()).collect();
            format!("{}({})", name, params.join(", "))
        }
        Language::Ffi | Language::C | Language::Jni => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            // ~keep Same status-code convention as render_c_fn_sig: a fallible method
            // whose logical return is `()` reports failure through the return itself, so
            // the ABI is `int32_t`, not `void`. Skipped when a curated override already
            // supplied a return type (that string is trusted as-is).
            let has_return_override = signature_override.and_then(|o| o.return_type.as_deref()).is_some();
            let ret = if matches!(lang, Language::Ffi | Language::C)
                && !has_return_override
                && matches!(method.return_type, TypeRef::Unit)
                && method.error_type.is_some()
            {
                "int32_t".to_string()
            } else {
                ret
            };
            format!("{} {}({});", ret, name, params.join(", "))
        }
        Language::Rust => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: Option<{pty}>")
                    } else {
                        match &p.ty {
                            TypeRef::String | TypeRef::Char => format!("{pname}: &str"),
                            TypeRef::Bytes => format!("{pname}: &[u8]"),
                            _ => format!("{pname}: {pty}"),
                        }
                    }
                })
                .collect();
            let ret = if let Some(err) = &method.error_type {
                let err_ty = type_name(err, Language::Rust, ffi_prefix);
                if ret == "()" {
                    format!("Result<(), {err_ty}>")
                } else {
                    format!("Result<{ret}, {err_ty}>")
                }
            } else {
                ret
            };
            let fn_keyword = if method.is_async { "pub async fn" } else { "pub fn" };
            if method.is_static {
                if ret == "()" {
                    format!("{fn_keyword} {}({})", name, params.join(", "))
                } else {
                    format!("{fn_keyword} {}({}) -> {}", name, params.join(", "), ret)
                }
            } else {
                let mut all_params = vec!["&self".to_string()];
                all_params.extend(params);
                if ret == "()" {
                    format!("{fn_keyword} {}({})", name, all_params.join(", "))
                } else {
                    format!("{fn_keyword} {}({}) -> {}", name, all_params.join(", "), ret)
                }
            }
        }
        Language::Kotlin | Language::KotlinAndroid => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: {pty}? = null")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let throws = method
                .error_type
                .as_ref()
                .map(|e| format!("@Throws({}::class)\n", type_name(e, lang, ffi_prefix)))
                .unwrap_or_default();
            let ret_part = if ret == "Unit" {
                String::new()
            } else {
                format!(": {ret}")
            };
            if method.is_static {
                format!("{throws}@JvmStatic\nfun {name}({}){ret_part}", params.join(", "))
            } else {
                format!("{throws}fun {name}({}){ret_part}", params.join(", "))
            }
        }
        Language::Swift => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: {pty}? = nil")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let throws = if method.error_type.is_some() { " throws" } else { "" };
            let ret_part = if ret == "Void" {
                String::new()
            } else {
                format!(" -> {ret}")
            };
            if method.is_static {
                format!("public static func {name}({}){throws}{ret_part}", params.join(", "))
            } else {
                format!("public func {name}({}){throws}{ret_part}", params.join(", "))
            }
        }
        Language::Dart => {
            let required: Vec<String> = method
                .params
                .iter()
                .filter(|p| !p.optional)
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty} {pname}")
                })
                .collect();
            let optional: Vec<String> = method
                .params
                .iter()
                .filter(|p| p.optional)
                .map(|p| {
                    let pname = to_camel_case(&p.name);
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    format!("{pty}? {pname}")
                })
                .collect();
            // ~keep Same real-backend grouping as `render_dart_fn_sig` -- required positional
            // params, then any optional ones grouped in `{}` (Dart named-optional syntax), not
            // `[]`. See that function's doc comment for the source citation.
            let params_str = match (required.is_empty(), optional.is_empty()) {
                (_, true) => required.join(", "),
                (true, false) => format!("{{{}}}", optional.join(", ")),
                (false, false) => format!("{}, {{{}}}", required.join(", "), optional.join(", ")),
            };
            let static_kw = if method.is_static { "static " } else { "" };
            // ~keep Same unconditional `Future<T>` wrap as `render_dart_fn_sig` -- flutter_rust_bridge
            // dispatches instance and static methods across the FFI boundary the same way it
            // dispatches free functions, so this is not gated on `method.is_async` or
            // `method.is_static` either. See that function's doc comment for the source citation.
            let future_ret = if ret_is_overridden {
                ret.clone()
            } else if ret == "void" {
                "Future<void>".to_string()
            } else {
                format!("Future<{ret}>")
            };
            format!("{static_kw}{future_ret} {name}({params_str})")
        }
        Language::Zig => {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_snake_case();
                    let pty = doc_type(&p.ty, lang, ffi_prefix);
                    if p.optional {
                        format!("{pname}: ?{pty}")
                    } else {
                        format!("{pname}: {pty}")
                    }
                })
                .collect();
            let ret_str = if let Some(err) = &method.error_type {
                let err_ty = type_name(err, lang, ffi_prefix);
                if ret == "void" {
                    format!("{err_ty}!void")
                } else {
                    format!("{err_ty}!{ret}")
                }
            } else {
                ret
            };
            let receiver_ty = type_name(type_name_str, lang, ffi_prefix);
            let mut all_params = if method.is_static {
                Vec::new()
            } else {
                vec![format!("self: *const {receiver_ty}")]
            };
            all_params.extend(params);
            format!("pub fn {name}({}) {ret_str}", all_params.join(", "))
        }
        Language::Gleam => {
            format!("// Phase 1: {lang} backend method signature generation")
        }
    }
}

#[cfg(test)]
#[path = "signatures/tests.rs"]
mod tests;
