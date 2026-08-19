//! Which types the generated C FFI actually exports constructors for.

/// Whether `type_name` is a std type the FFI crate never emits a `<prefix>_<type>_from_json`
/// constructor for.
///
/// ~keep The FFI exports `_from_json` / `_free` only for types the crate itself defines (and, for
/// enums, only when one is used as a pointer param). For a `Vec<String>` argument `element_type`
/// resolves to the std type `String`, and building a handle from it emitted a call to
/// `<prefix>_string_from_json` -- a function nothing declares -- so every generated snippet using
/// such an argument failed to compile with "call to undeclared function". The C ABI takes that
/// argument as a plain `const char *` JSON string anyway, so these are skipped and
/// `build_args_string_c`'s literal path splices the JSON in directly. This is deliberately a
/// closed list of std types rather than an "is it in the IR" test: skipping is only ever correct
/// when the type definitionally has no constructor, and every other type keeps today's behaviour.
pub(super) fn is_std_type_without_ffi_constructor(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "str"
            | "bool"
            | "char"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}
