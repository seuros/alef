use crate::core::config::Language;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use crate::docs::formatting::{IdentifierPosition, report_identifier_violation};
use crate::docs::naming::{func_name, to_camel_case, type_name};
use crate::docs::rust_types::rust_param_type;
use crate::docs::type_mapping::doc_type;
use heck::ToSnakeCase;

pub(crate) fn render_python_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    // ~keep Routed through `func_name`, not a bare `.to_snake_case()`, so a free function
    // whose name collides with a Python keyword (`global`, `class`, ...) is escaped before the
    // gate ever judges it -- the same discipline `method_name` already applies to opaque-type
    // methods. A real consumer's `pub fn global() -> &'static Registry` is what this closes: a
    // free function, not a constructor, so no per-shape renderer branch could have caught it.
    let name = func_name(&func.name, Language::Python, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Python,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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
    // ~keep Routed through `func_name` (Declaration position, checked below) rather than a
    // bare `to_camel_case` -- `func_name` never renames anything for Node/Wasm (the member-
    // position relaxation those two languages get depends on the raw word surviving), so this
    // is a no-op today, but a free function *is* judged in Declaration position, where the
    // reserved word is still illegal in every language including these two. Keeping this call
    // site on the same helper as every other renderer, rather than a hand-rolled exception, is
    // what stops that from silently drifting later.
    let name = func_name(&func.name, Language::Node, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Node,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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
pub(crate) fn go_return_type(return_type: &TypeRef, ret: String) -> String {
    if matches!(return_type, TypeRef::Named(_)) {
        format!("*{ret}")
    } else {
        ret
    }
}

pub(crate) fn render_go_fn_sig(func: &FunctionDef, ffi_prefix: &str) -> String {
    let name = func_name(&func.name, Language::Go, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Go,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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

pub(crate) fn render_java_fn_sig(func: &FunctionDef, ffi_prefix: &str, crate_name: &str) -> String {
    // ~keep Routed through `func_name` rather than a bare `to_camel_case`: a free function
    // named `new` or `default` used to reach the gate unrenamed (only `method_name`'s opaque
    // methods went through `func_name`'s Java table), so this closes the same gap on the
    // free-function path.
    let name = func_name(&func.name, Language::Java, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Java,
        IdentifierPosition::Member,
        "a function signature",
    );
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
    //
    // ~keep The class is named by `backends::java::naming::exception_class_name`, the same
    // derivation `JavaBackend::resolve_main_class` feeds into `<MainClass>Exception.java`.
    // Building it from `ffi_prefix` here documented a class no generated package declares
    // whenever `[ffi] prefix` differed from the crate name.
    let throws = format!(
        " throws {}",
        crate::backends::java::naming::exception_class_name(crate_name)
    );
    format!("public static {} {}({}){}", ret, name, params.join(", "), throws)
}

pub(crate) fn render_ruby_fn_sig(func: &FunctionDef) -> String {
    let name = func_name(&func.name, Language::Ruby, "");
    report_identifier_violation(
        &name,
        Language::Ruby,
        IdentifierPosition::Member,
        "a function signature",
    );
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
    let name = crate::codegen::c_consumer::free_function_symbol(&ffi_prefix.to_snake_case(), &func.name);
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
    let name = func_name(&func.name, Language::Php, ffi_prefix);
    report_identifier_violation(&name, Language::Php, IdentifierPosition::Member, "a function signature");
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
    let name = func_name(&func.name, Language::Elixir, "");
    report_identifier_violation(
        &name,
        Language::Elixir,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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
    let name = func_name(&func.name, Language::R, "");
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
    let name = func_name(&func.name, Language::Csharp, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Csharp,
        IdentifierPosition::Member,
        "a function signature",
    );
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
        let async_name = crate::docs::naming::csharp_async_member_name(&name, true);
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
        .map(|p| format!("{}: {}", p.name.to_snake_case(), rust_param_type(p, ffi_prefix)))
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
    let name = func_name(&func.name, Language::Kotlin, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Kotlin,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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
    // ~keep A free function has no owning type, so there is no `is_swift_static_constructor`
    // shape to divert here -- only the generic keyword escape in `func_name` applies. A free
    // Swift function literally named `init` (declaration-keyword collision) or any other
    // reserved word now renders escaped instead of reaching the gate raw.
    let name = func_name(&func.name, Language::Swift, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Swift,
        IdentifierPosition::Member,
        "a function signature",
    );
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
    let name = func_name(&func.name, Language::Dart, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Dart,
        IdentifierPosition::Declaration,
        "a function signature",
    );
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

pub(crate) fn render_zig_fn_sig(func: &FunctionDef, ffi_prefix: &str, api: &ApiSurface) -> String {
    let name = func_name(&func.name, Language::Zig, ffi_prefix);
    report_identifier_violation(
        &name,
        Language::Zig,
        IdentifierPosition::Declaration,
        "a function signature",
    );
    // ~keep A `Named` DTO does not cross the Zig wrapper boundary as its struct type -- a
    // non-opaque type serialises to JSON-encoded `[]const u8`/`[]u8`, only an opaque handle
    // type keeps its real name. `zig_boundary_param_type`/`zig_boundary_return_type` ask the
    // Zig backend's own emitter (`zig_param_type`/`zig_return_type`) that question instead of
    // re-deriving it here -- see those functions' doc comments.
    let ret = crate::backends::zig::zig_boundary_return_type(&func.return_type, api);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let pname = p.name.to_snake_case();
            let pty = crate::backends::zig::zig_boundary_param_type(&p.ty, p.optional, api);
            format!("{pname}: {pty}")
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
