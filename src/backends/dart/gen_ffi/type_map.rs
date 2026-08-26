use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};

use crate::backends::dart::type_map::DartMapper;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::Language;

/// The `dart:ffi` native C type for a function parameter (in the native typedef).
pub(super) fn native_param_type(p: &ParamDef) -> String {
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Utf8>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => native_primitive(prim),
        TypeRef::Char => "Uint32".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// The Dart callable type for a function parameter (in the Dart typedef).
pub(super) fn dart_callable_type(p: &ParamDef) -> String {
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Utf8>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => dart_primitive_callable(prim),
        TypeRef::Char => "int".to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Dart public wrapper parameter declaration (e.g. `String name`).
pub(super) fn dart_wrapper_param(p: &ParamDef) -> String {
    let ty = dart_type(&p.ty, p.optional);
    let name = dart_param_name(&p.name);
    format!("{ty} {name}")
}

/// Argument expression to pass into the low-level `_fnName` call.
pub(super) fn call_arg_name(p: &ParamDef) -> String {
    let name = dart_param_name(&p.name);
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{name}Native.cast<Utf8>()")
        }
        _ => name,
    }
}

pub(super) fn dart_param_name(name: &str) -> String {
    public_host_identifier(Language::Dart, PublicIdentifierKind::Parameter, name)
}

/// The `dart:ffi` (native, Dart) type pair for the C type the FFI crate really returns.
///
/// Asked of [`crate::backends::ffi::type_map::c_return_type`] — the layer that emits the symbol —
/// rather than re-derived from the `TypeRef`. An `Optional` return used to fall through to
/// `Pointer<Void>` here while the crate exported `int64_t`, so Dart read an integer as an address;
/// `Option<bool>` was worse still, a 4-byte `i32` read as an 8-byte pointer. Nothing downstream
/// can catch that: `gen_ffi` never parses the cbindgen header, so the straddle is invisible from
/// the Dart side.
///
/// Panics on a C spelling this table does not know, so a new FFI return shape fails at generation
/// time instead of silently reintroducing a `Pointer<Void>`. ~keep
fn ffi_return_shape(ty: &TypeRef) -> (&'static str, &'static str) {
    // A handle return is `AlefHandle` whatever the core path is, so the import is not consulted.
    let c_type = crate::backends::ffi::type_map::c_return_type(ty, "");
    match c_type.as_ref() {
        "()" => ("Void", "void"),
        "i8" => ("Int8", "int"),
        "i16" => ("Int16", "int"),
        "i32" => ("Int32", "int"),
        "i64" => ("Int64", "int"),
        "u8" => ("Uint8", "int"),
        "u16" => ("Uint16", "int"),
        "u32" => ("Uint32", "int"),
        "u64" | "AlefHandle" => ("Uint64", "int"),
        "usize" => ("Size", "int"),
        "isize" => ("IntPtr", "int"),
        "f32" => ("Float", "double"),
        "f64" => ("Double", "double"),
        "*mut std::ffi::c_char" => ("Pointer<Char>", "Pointer<Char>"),
        "*mut u8" => ("Pointer<Uint8>", "Pointer<Uint8>"),
        other => panic!(
            "dart:ffi backend: the FFI crate returns `{other}` for {ty:?}, which has no declared \
             dart:ffi shape; add it to `ffi_return_shape` rather than letting the typedef fall \
             back to a pointer of the wrong width"
        ),
    }
}

/// Native C return type (used in the native typedef).
pub(super) fn native_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "Void".to_string(),
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Char>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => native_primitive(prim),
        TypeRef::Char => "Uint32".to_string(),
        TypeRef::Optional(_) => ffi_return_shape(ty).0.to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Dart callable return type (used in the Dart typedef).
pub(super) fn dart_callable_return(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "Pointer<Char>".to_string()
        }
        TypeRef::Bytes => "Pointer<Uint8>".to_string(),
        TypeRef::Primitive(prim) => dart_primitive_callable(prim),
        TypeRef::Char => "int".to_string(),
        TypeRef::Optional(_) => ffi_return_shape(ty).1.to_string(),
        _ => "Pointer<Void>".to_string(),
    }
}

/// Public Dart return type in the wrapper function signature.
pub(super) fn dart_public_return(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        _ => dart_type(ty, false),
    }
}

/// Convert a raw C return value to the public Dart type.
///
/// The `Optional` arm reads the shape [`ffi_return_shape`] declared, so the conversion can never
/// be derived from a different fact than the typedef it consumes. A scalar leaf stays a bare
/// passthrough: its absence is answered out of band by the presence companion (`result_presence`),
/// not by inspecting the value. `Option<bool>` is the one scalar that still needs converting,
/// because the FFI crate returns it as `i32`. ~keep
pub(super) fn unwrap_return_expr(raw: &str, ty: &TypeRef, _free_symbol: &str, _error_code_symbol: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            owned_string_expr(raw)
        }
        TypeRef::Optional(inner) => optional_unwrap_return_expr(raw, inner, ty),
        _ => raw.to_string(),
    }
}

/// Copy a `Pointer<Char>` the FFI crate owns into a Dart `String` and release it.
fn owned_string_expr(raw: &str) -> String {
    format!("() {{ final s = {raw}.cast<Utf8>().toDartString(); _freeString({raw}.cast<Char>()); return s; }}()")
}

/// The `Optional<T>` half of [`unwrap_return_expr`].
fn optional_unwrap_return_expr(raw: &str, inner: &TypeRef, ty: &TypeRef) -> String {
    if ffi_return_shape(ty).1 == "Pointer<Char>" {
        return format!("{raw} == nullptr ? null : {}", owned_string_expr(raw));
    }
    match inner {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{raw} != 0"),
        _ => raw.to_string(),
    }
}

/// `dart:ffi` native (C) type for a primitive.
pub(super) fn native_primitive(prim: &PrimitiveType) -> String {
    match prim {
        PrimitiveType::Bool => "Bool".to_string(),
        PrimitiveType::U8 => "Uint8".to_string(),
        PrimitiveType::I8 => "Int8".to_string(),
        PrimitiveType::U16 => "Uint16".to_string(),
        PrimitiveType::I16 => "Int16".to_string(),
        PrimitiveType::U32 => "Uint32".to_string(),
        PrimitiveType::I32 => "Int32".to_string(),
        PrimitiveType::U64 => "Uint64".to_string(),
        PrimitiveType::I64 => "Int64".to_string(),
        PrimitiveType::Usize => "Size".to_string(),
        PrimitiveType::Isize => "IntPtr".to_string(),
        PrimitiveType::F32 => "Float".to_string(),
        PrimitiveType::F64 => "Double".to_string(),
    }
}

/// Dart callable (non-native) type for a primitive.
pub(super) fn dart_primitive_callable(prim: &PrimitiveType) -> String {
    match prim {
        PrimitiveType::Bool => "bool".to_string(),
        PrimitiveType::F32 | PrimitiveType::F64 => "double".to_string(),
        _ => "int".to_string(),
    }
}

/// Public Dart type (high-level) for a type ref.
pub(super) fn dart_type(ty: &TypeRef, optional: bool) -> String {
    let inner = match ty {
        TypeRef::Bytes => "Uint8List".to_string(),
        TypeRef::Optional(inner) => return dart_type(inner, true),
        TypeRef::Vec(inner) => format!("List<{}>", dart_type(inner, false)),
        TypeRef::Map(k, v) => format!("Map<{}, {}>", dart_type(k, false), dart_type(v, false)),
        TypeRef::Primitive(prim) => DartMapper.primitive(prim).into_owned(),
        _ => DartMapper.map_type(ty),
    };
    if optional { format!("{inner}?") } else { inner }
}

pub(super) fn dart_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}
