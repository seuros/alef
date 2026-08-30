//! The name-surface vocabulary shared by every naming helper, plus collision detection over a
//! generated-name scope.
//!
//! Every other submodule of `naming` serves exactly one of the surfaces named by [`NameSurface`],
//! so the enums live apart from the helpers that consume them and no surface's module owns the
//! vocabulary the others must speak. ~keep

use crate::core::config::Language;
use std::collections::{HashMap, HashSet};

/// Distinct name surfaces used by generated bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSurface {
    /// Public identifier exposed in the target host language.
    PublicHost,
    /// Wire/JSON field names, tags, and variant values.
    Wire,
    /// Internal Rust identifier emitted by a backend crate.
    InternalRust,
    /// ABI/native symbol such as C FFI or JNI.
    Abi,
}

/// Identifier context within a name surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierContext {
    PublicType,
    PublicMember,
    PublicParameter,
    PublicEnumVariant,
    Wire,
    InternalRust,
    AbiSymbol,
    SwiftSource,
    SwiftRustShim,
    KotlinSource,
    KotlinRustBridge,
    DartType,
    DartValue,
    DartTupleField,
}

/// Public host-language identifier kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicIdentifierKind {
    Function,
    Method,
    Field,
    Type,
    EnumVariant,
    Parameter,
}

/// A generated-name collision within one target scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCollision {
    pub generated: String,
    pub originals: Vec<String>,
}

/// Error raised by centralized naming validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    InvalidIdentifier {
        lang: Language,
        context: IdentifierContext,
        name: String,
    },
    Collision(NameCollision),
}

/// Return all generated-name collisions in a target scope.
pub fn detect_name_collisions<I, O, G>(items: I, generate: G) -> Vec<NameCollision>
where
    I: IntoIterator<Item = O>,
    O: AsRef<str>,
    G: Fn(&str) -> String,
{
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for item in items {
        let original = item.as_ref();
        grouped
            .entry(generate(original))
            .or_default()
            .push(original.to_string());
    }

    grouped
        .into_iter()
        .filter_map(|(generated, originals)| {
            let unique: HashSet<_> = originals.iter().collect();
            (unique.len() > 1).then_some(NameCollision { generated, originals })
        })
        .collect()
}
