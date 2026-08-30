//! The two name surfaces no host-language consumer ever reads: internal generated Rust
//! identifiers, and C ABI / native symbols.
//!
//! These are kept away from [`super::host`] deliberately. A C ABI member name is a global
//! identifier subject to link-time collision, and an internal Rust name only has to satisfy
//! rustc — neither may be derived with a host-language casing helper. ~keep

use super::case::pascal_to_snake;
use heck::{ToPascalCase, ToSnakeCase};

/// Resolve an internal Rust identifier and raw-escape Rust keywords.
pub fn internal_rust_identifier(name: &str) -> String {
    crate::core::keywords::rust_raw_ident(name)
}

/// Resolve a C-style ABI symbol with an explicit prefix.
pub fn abi_symbol(prefix: &str, name: &str) -> String {
    to_c_name(prefix, name)
}

/// Convert a Rust name to a C-style prefixed snake_case identifier (e.g. `prefix_name`).
pub fn to_c_name(prefix: &str, name: &str) -> String {
    format!("{}_{}", prefix, name.to_snake_case())
}

/// Resolve an ABI symbol from already-separated path components.
pub fn abi_symbol_from_components<I, S>(components: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parts = components
        .into_iter()
        .enumerate()
        .map(|(idx, component)| {
            let sanitized = sanitize_symbol_component(component.as_ref());
            if idx == 0 {
                sanitized
            } else {
                sanitized.trim_start_matches('_').to_string()
            }
        })
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let symbol = parts.join("_");
    if symbol.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        parts.insert(0, "_".to_string());
        parts.join("_")
    } else {
        symbol
    }
}

pub(super) fn sanitize_symbol_component(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len() + 1);
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }
    while sanitized.contains("__") {
        sanitized = sanitized.replace("__", "_");
    }
    let sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("_{sanitized}")
    } else {
        sanitized
    }
}

/// Split an identifier or path into its lowercase words, treating every
/// non-alphanumeric character as a separator and splitting PascalCase runs.
fn identifier_words(name: &str) -> Vec<String> {
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|chunk| !chunk.is_empty())
        .flat_map(|chunk| {
            pascal_to_snake(chunk)
                .split('_')
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Drop each word that repeats the word immediately before it, ignoring case.
fn collapse_repeated_words(words: Vec<String>) -> Vec<String> {
    let mut collapsed: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        if collapsed
            .last()
            .is_some_and(|previous| previous.eq_ignore_ascii_case(&word))
        {
            continue;
        }
        collapsed.push(word);
    }
    collapsed
}

/// The C ABI member-name prefix for alef's own built-in error codes. ~keep
///
/// cbindgen applies `[export] prefix` to the enum *type* but emits member names
/// verbatim, and C enum members are global identifiers. Unprefixed members would
/// therefore collide both with platform headers (X11 `#define None 0L`) and with
/// a second alef-generated library linked into the same translation unit, so the
/// project's ABI prefix is baked into the member name here.
pub fn ffi_builtin_error_code_prefix(abi_prefix: &str) -> String {
    format!("{abi_prefix}_alef").to_pascal_case()
}

/// Produce a project-agnostic C ABI error-enum member from its canonical Rust identity. ~keep
pub fn ffi_error_code_variant_name(error_type: &str, variant: &str) -> String {
    let path_words = collapse_repeated_words(identifier_words(error_type));
    let mut variant_words = identifier_words(variant);
    // Collapsing repeats inside the type path is safe because the path is constant
    // across a type's variants, so distinct variants stay distinct. The boundary
    // elision below is narrower but not injective: a type owning both `ErrorFoo`
    // and `Foo` folds onto one name. That is the pre-existing trade-off, and C
    // rejects the duplicate enumerator at compile time rather than mapping a
    // code silently, so the readability win is kept. ~keep
    if path_words.last().is_some_and(|word| word == "error")
        && variant_words.first().is_some_and(|word| word == "error")
        && variant_words.len() > 1
    {
        variant_words.remove(0);
    }
    path_words
        .into_iter()
        .chain(variant_words)
        .collect::<Vec<_>>()
        .join("_")
        .to_pascal_case()
}
