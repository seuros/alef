//! The public host-language identifier surface: the names a consumer of a generated binding
//! actually types.
//!
//! [`public_host_identifier`] is the single entry point; the per-kind helpers below are private
//! to the `naming` module so no backend can reach past the escaping step. ~keep

use super::case::{pascal_to_screaming_snake, pascal_to_snake};
use super::identifiers::escape_identifier_for;
use super::languages::{csharp_type_name, go_param_name, go_type_name, to_csharp_name, to_go_name};
use super::surfaces::{IdentifierContext, PublicIdentifierKind};
use crate::core::config::Language;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};

/// Resolve a public field/property identifier, applying `rename_fields` before language casing.
pub fn public_field_name(lang: Language, rust_field_name: &str, rename_fields_value: Option<&str>) -> String {
    let base = rename_fields_value.unwrap_or(rust_field_name);
    public_host_identifier(lang, PublicIdentifierKind::Field, base)
}

/// Resolve a public host-language identifier for a Rust name.
pub fn public_host_identifier(lang: Language, kind: PublicIdentifierKind, rust_name: &str) -> String {
    let converted = match kind {
        PublicIdentifierKind::Type => public_type_name(lang, rust_name),
        PublicIdentifierKind::EnumVariant => public_enum_variant_name(lang, rust_name),
        PublicIdentifierKind::Function | PublicIdentifierKind::Method | PublicIdentifierKind::Field => {
            public_member_name(lang, rust_name)
        }
        PublicIdentifierKind::Parameter => public_parameter_name(lang, rust_name),
    };
    escape_identifier_for(lang, &converted, public_identifier_context(kind))
}

/// Qualify a type name with a dotted package/namespace, leaving an already-qualified name alone.
///
/// JVM-family and .NET targets accept a type spelled either bare (`SampleClient`) or fully
/// qualified (`dev.sample.bindings.SampleClient`), and one configured value — an
/// `[e2e.call.overrides.<lang>] class`, an `options_type`, a bridge class — reaches several
/// emitters. Prefixing unconditionally turns the qualified spelling into
/// `dev.sample.bindings.dev.sample.bindings.SampleClient`, which does not resolve; never
/// prefixing leaves a bare name unresolvable from a child package. The "does it already carry a
/// package" decision has to be made in exactly one place, or two emitters reading the same config
/// value disagree and the generated file carries both spellings. ~keep
pub fn qualified_type_path(package: &str, type_name: &str) -> String {
    if package.is_empty() || type_name.contains('.') {
        return type_name.to_string();
    }
    format!("{package}.{type_name}")
}

fn public_member_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::Ffi | Language::R | Language::Rust => {
            name.to_snake_case()
        }
        Language::Go => to_go_name(name),
        Language::Csharp => to_csharp_name(name),
        Language::Node
        | Language::Php
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart => name.to_lower_camel_case(),
        Language::Gleam | Language::Zig | Language::C | Language::Jni => name.to_snake_case(),
    }
}

fn public_parameter_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Go => go_param_name(name),
        _ => public_member_name(lang, name),
    }
}

pub(super) fn public_type_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Go => go_type_name(&name.to_pascal_case()),
        Language::Csharp => csharp_type_name(&name.to_pascal_case()),
        Language::Python
        | Language::Node
        | Language::Ruby
        | Language::Php
        | Language::Elixir
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig
        | Language::Ffi
        | Language::R
        | Language::Rust
        | Language::C
        | Language::Jni => name.to_pascal_case(),
    }
}

fn public_enum_variant_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Python | Language::Ffi | Language::C | Language::Rust => pascal_to_screaming_snake(name),
        Language::Ruby | Language::Elixir | Language::R | Language::Gleam | Language::Zig => pascal_to_snake(name),
        Language::Go => go_type_name(&name.to_pascal_case()),
        Language::Csharp => csharp_type_name(&name.to_pascal_case()),
        Language::Node
        | Language::Php
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Jni => name.to_pascal_case(),
    }
}

fn public_identifier_context(kind: PublicIdentifierKind) -> IdentifierContext {
    match kind {
        PublicIdentifierKind::Function | PublicIdentifierKind::Method | PublicIdentifierKind::Field => {
            IdentifierContext::PublicMember
        }
        PublicIdentifierKind::Type => IdentifierContext::PublicType,
        PublicIdentifierKind::EnumVariant => IdentifierContext::PublicEnumVariant,
        PublicIdentifierKind::Parameter => IdentifierContext::PublicParameter,
    }
}

