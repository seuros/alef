use serde::{Deserialize, Serialize};

/// Indicates the core Rust type wraps the resolved type in a smart pointer or cow.
/// Used by codegen to generate correct From/Into conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CoreWrapper {
    #[default]
    None,
    /// `Cow<'static, str>` — binding uses String, core needs `.into()`
    Cow,
    /// `Arc<T>` — binding unwraps, core wraps with `Arc::new()`
    Arc,
    /// `bytes::Bytes` — binding uses `Vec<u8>`, core needs `Bytes::from()`
    Bytes,
    /// `Arc<Mutex<T>>` — binding wraps with `Arc::new(Mutex::new())`, methods call `.lock()`
    ArcMutex,
    /// `Box<str>` — binding uses String, core needs `.into()` (same shape as Cow
    /// but distinct so backends can keep wrapper-specific behavior addressable).
    Box,
}

/// Typed default value for a field, enabling backends to emit language-native defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DefaultValue {
    BoolLiteral(bool),
    StringLiteral(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    EnumVariant(String),
    /// A zero-argument Rust function that supplies the value at runtime. ~keep
    FunctionCall(String),
    /// A public zero-argument Rust function callable from generated binding crates. ~keep
    PublicFunctionCall(String),
    /// A non-empty collection literal, holding its elements in source order.
    ///
    /// A genuinely empty `vec![]`/`Vec::new()` stays [`DefaultValue::Empty`]: the two are
    /// distinct because every backend already renders "the empty collection" natively, whereas
    /// this variant carries elements that have to be rendered individually. Only produced when
    /// every element is itself representable — anything else falls back to `Empty`, so a
    /// backend never emits a default that silently differs from the Rust one. ~keep
    ListLiteral(Vec<DefaultValue>),
    /// Empty collection or Default::default()
    Empty,
    /// None / null
    None,
}

/// Stable identity metadata for one error variant. ~keep
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorTaxonomy {
    #[serde(default)]
    pub code: u32,
    #[serde(default)]
    pub error_type: String,
    #[serde(default)]
    pub variant: String,
}

impl ErrorTaxonomy {
    pub fn for_variant(code: u32, error_type: &str, variant: &str) -> Self {
        Self {
            code,
            error_type: error_type.to_string(),
            variant: variant.to_string(),
        }
    }
}

#[cfg(test)]
mod error_taxonomy_tests {
    use super::ErrorTaxonomy;

    #[test]
    fn explicit_variant_code_is_preserved() {
        let taxonomy = ErrorTaxonomy::for_variant(101, "sample::RequestError", "InvalidInput");
        assert_eq!(taxonomy.code, 101);
        assert_eq!(taxonomy.error_type, "sample::RequestError");
        assert_eq!(taxonomy.variant, "InvalidInput");
    }

    #[test]
    fn legacy_serialized_taxonomy_defaults_compatibly() {
        let taxonomy: ErrorTaxonomy = serde_json::from_str("{}").expect("legacy metadata deserializes");

        assert_eq!(taxonomy, ErrorTaxonomy::default());
    }
}

/// Deprecation metadata extracted from `#[deprecated(...)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeprecationInfo {
    /// Version when the item was deprecated (from `#[deprecated(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation note (from `#[deprecated(note = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Version annotation on an IR item.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionAnnotation {
    /// Version when this item was introduced (from `#[alef(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation info (from `#[deprecated(...)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationInfo>,
}
