//! Identifier legality and escaping: whether a name can be spelled at all in a target language,
//! and how to make it spellable.
//!
//! This is orthogonal to which name a surface picks — [`super::host`], [`super::wire`] and
//! [`super::symbols`] all funnel their result through here before it reaches a template. ~keep

use super::surfaces::{IdentifierContext, NameError, NameSurface};
use super::symbols::sanitize_symbol_component;
use crate::core::config::Language;

/// Return a language-safe identifier for a generated name surface.
pub fn escape_identifier(lang: Language, name: &str, surface: NameSurface) -> String {
    let context = match surface {
        NameSurface::PublicHost => IdentifierContext::PublicMember,
        NameSurface::Wire => IdentifierContext::Wire,
        NameSurface::InternalRust => IdentifierContext::InternalRust,
        NameSurface::Abi => IdentifierContext::AbiSymbol,
    };
    escape_identifier_for(lang, name, context)
}

/// Return a language-safe identifier for a specific context.
pub fn escape_identifier_for(lang: Language, name: &str, context: IdentifierContext) -> String {
    match context {
        IdentifierContext::Wire => name.to_string(),
        IdentifierContext::InternalRust => crate::core::keywords::rust_raw_ident(name),
        IdentifierContext::AbiSymbol => sanitize_symbol_component(name),
        IdentifierContext::SwiftSource => crate::core::keywords::swift_case_ident(name),
        IdentifierContext::SwiftRustShim => crate::core::keywords::swift_ident(name),
        IdentifierContext::KotlinSource => backtick_keyword(lang, name),
        IdentifierContext::KotlinRustBridge => crate::core::keywords::kotlin_ident(name),
        IdentifierContext::DartType => dart_type_identifier(name, None),
        IdentifierContext::DartValue => dart_value_identifier(name),
        IdentifierContext::DartTupleField => dart_tuple_field_identifier(name),
        IdentifierContext::PublicType
        | IdentifierContext::PublicMember
        | IdentifierContext::PublicParameter
        | IdentifierContext::PublicEnumVariant => match lang {
            Language::Swift => crate::core::keywords::swift_case_ident(name),
            Language::Zig => crate::core::keywords::zig_ident(name),
            Language::Python => crate::core::keywords::python_ident(name),
            Language::Kotlin | Language::KotlinAndroid => crate::core::keywords::kotlin_ident(name),
            Language::Dart => match context {
                IdentifierContext::PublicType => dart_type_identifier(name, None),
                IdentifierContext::PublicMember
                | IdentifierContext::PublicParameter
                | IdentifierContext::PublicEnumVariant => dart_value_identifier(name),
                _ => unreachable!("matched public identifier contexts only"),
            },
            Language::Gleam => crate::core::keywords::gleam_ident(name),
            _ if is_reserved_keyword(lang, name) => format!("{name}_"),
            _ => name.to_string(),
        },
    }
}

/// Validate that a generated identifier is syntactically usable for a language.
pub fn is_valid_identifier(lang: Language, name: &str, surface: NameSurface) -> bool {
    if matches!(surface, NameSurface::Wire) {
        return !name.is_empty();
    }
    match lang {
        Language::Rust => crate::core::keywords::is_valid_rust_ident_chars(name.trim_start_matches("r#")),
        Language::Swift => {
            let unescaped = name.strip_prefix('`').and_then(|s| s.strip_suffix('`')).unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        Language::Zig => is_ascii_identifier(name) && !name.starts_with(|ch: char| ch.is_ascii_digit()),
        Language::Csharp => {
            let unescaped = name.strip_prefix('@').unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        _ => is_ascii_identifier(name),
    }
}

/// Validate a generated identifier for a specific context.
pub fn validate_identifier(lang: Language, name: &str, context: IdentifierContext) -> Result<(), NameError> {
    if is_valid_identifier_for(lang, name, context) {
        Ok(())
    } else {
        Err(NameError::InvalidIdentifier {
            lang,
            context,
            name: name.to_string(),
        })
    }
}

/// Returns whether a generated identifier is syntactically usable for a specific context.
pub fn is_valid_identifier_for(lang: Language, name: &str, context: IdentifierContext) -> bool {
    match context {
        IdentifierContext::Wire => !name.is_empty(),
        IdentifierContext::InternalRust => {
            crate::core::keywords::is_valid_rust_ident_chars(name.trim_start_matches("r#"))
        }
        IdentifierContext::AbiSymbol => is_ascii_identifier(name),
        IdentifierContext::SwiftSource => {
            let unescaped = name.strip_prefix('`').and_then(|s| s.strip_suffix('`')).unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        IdentifierContext::DartTupleField => name.starts_with("field") && is_ascii_identifier(name),
        _ => is_valid_identifier(lang, name, NameSurface::PublicHost),
    }
}

/// Resolve a Dart type identifier, preserving core type names by adding context.
pub fn dart_type_identifier(name: &str, parent: Option<&str>) -> String {
    if is_dart_core_type(name) || is_reserved_keyword(Language::Dart, name) {
        match parent {
            Some(parent) if !parent.is_empty() => format!("{parent}{name}"),
            _ => format!("{name}Node"),
        }
    } else {
        name.to_string()
    }
}

/// Resolve a Dart value/member identifier.
pub fn dart_value_identifier(name: &str) -> String {
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return format!("field{name}");
    }
    crate::core::keywords::dart_ident(name)
}

/// Resolve a Dart tuple field identifier.
pub fn dart_tuple_field_identifier(name: &str) -> String {
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("field{name}")
    } else {
        dart_value_identifier(name)
    }
}


fn is_ascii_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty string has a first char");
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}


fn backtick_keyword(lang: Language, name: &str) -> String {
    if is_reserved_keyword(lang, name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

fn is_dart_core_type(name: &str) -> bool {
    const DART_CORE_TYPES: &[&str] = &[
        "bool",
        "double",
        "Duration",
        "Error",
        "Exception",
        "Future",
        "int",
        "Invocation",
        "Iterable",
        "Iterator",
        "List",
        "Map",
        "MapEntry",
        "Null",
        "num",
        "Object",
        "Pattern",
        "RegExp",
        "RuneIterator",
        "Runes",
        "Set",
        "Sink",
        "StackTrace",
        "Stream",
        "String",
        "StringBuffer",
        "Symbol",
        "Type",
        "Uri",
    ];
    DART_CORE_TYPES.contains(&name)
}

fn is_reserved_keyword(lang: Language, name: &str) -> bool {
    match lang {
        Language::Python => crate::core::keywords::PYTHON_KEYWORDS.contains(&name),
        Language::Node | Language::Wasm => crate::core::keywords::JS_KEYWORDS.contains(&name),
        Language::Ruby => crate::core::keywords::RUBY_KEYWORDS.contains(&name),
        Language::Php => crate::core::keywords::PHP_KEYWORDS.contains(&name),
        Language::Elixir => crate::core::keywords::ELIXIR_KEYWORDS.contains(&name),
        Language::Go => crate::core::keywords::GO_KEYWORDS.contains(&name),
        Language::Java | Language::Jni => crate::core::keywords::JAVA_KEYWORDS.contains(&name),
        Language::Csharp => crate::core::keywords::CSHARP_KEYWORDS.contains(&name),
        Language::R => crate::core::keywords::R_KEYWORDS.contains(&name),
        Language::Kotlin | Language::KotlinAndroid => crate::core::keywords::KOTLIN_KEYWORDS.contains(&name),
        Language::Swift => crate::core::keywords::SWIFT_KEYWORDS.contains(&name),
        Language::Dart => crate::core::keywords::DART_KEYWORDS.contains(&name),
        Language::Gleam => crate::core::keywords::GLEAM_KEYWORDS.contains(&name),
        Language::Zig => crate::core::keywords::ZIG_KEYWORDS.contains(&name),
        Language::Rust => crate::core::keywords::RUST_KEYWORDS.contains(&name),
        Language::Ffi | Language::C => false,
    }
}
