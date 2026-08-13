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
    const FIRST_VARIANT_CODE: u32 = 100;
    const LAST_VARIANT_CODE: u32 = i32::MAX as u32;

    pub fn for_variant(error_type: &str, variant: &str) -> Self {
        let identity = format!("{error_type}::{variant}");
        let digest = blake3::hash(identity.as_bytes());
        let mut code_bytes = [0_u8; size_of::<u32>()];
        code_bytes.copy_from_slice(&digest.as_bytes()[..size_of::<u32>()]);
        let code_space = Self::LAST_VARIANT_CODE - Self::FIRST_VARIANT_CODE + 1;
        let code = Self::FIRST_VARIANT_CODE + u32::from_le_bytes(code_bytes) % code_space;

        Self {
            code,
            error_type: error_type.to_string(),
            variant: variant.to_string(),
        }
    }

    pub(crate) fn ensure_unique_codes(taxonomies: &mut [Self]) {
        let mut order: Vec<usize> = (0..taxonomies.len()).collect();
        order.sort_by(|&left, &right| {
            (&taxonomies[left].error_type, &taxonomies[left].variant)
                .cmp(&(&taxonomies[right].error_type, &taxonomies[right].variant))
        });

        let mut used = std::collections::HashSet::new();
        for index in order {
            while !used.insert(taxonomies[index].code) {
                taxonomies[index].code = if taxonomies[index].code == Self::LAST_VARIANT_CODE {
                    Self::FIRST_VARIANT_CODE
                } else {
                    taxonomies[index].code + 1
                };
            }
        }
    }
}

#[cfg(test)]
mod error_taxonomy_tests {
    use super::ErrorTaxonomy;

    #[test]
    fn variant_codes_are_stable_nonzero_and_variant_specific() {
        let cases = [
            ("sample::RequestError", "InvalidInput"),
            ("sample::RequestError", "Unavailable"),
            ("sample::StorageError", "Unavailable"),
        ];
        let taxonomies: Vec<_> = cases
            .iter()
            .map(|(error_type, variant)| ErrorTaxonomy::for_variant(error_type, variant))
            .collect();

        for (taxonomy, (error_type, variant)) in taxonomies.iter().zip(cases) {
            assert_ne!(taxonomy.code, 0);
            assert_eq!(taxonomy.error_type, error_type);
            assert_eq!(taxonomy.variant, variant);
            assert_eq!(taxonomy, &ErrorTaxonomy::for_variant(error_type, variant));
        }
        assert_ne!(taxonomies[0].code, taxonomies[1].code);
        assert_ne!(taxonomies[1].code, taxonomies[2].code);
    }

    #[test]
    fn legacy_serialized_taxonomy_defaults_compatibly() {
        let taxonomy: ErrorTaxonomy = serde_json::from_str("{}").expect("legacy metadata deserializes");

        assert_eq!(taxonomy, ErrorTaxonomy::default());
    }

    #[test]
    fn colliding_codes_are_resolved_deterministically() {
        let cases = [("sample::Beta", "Busy"), ("sample::Alpha", "Invalid")];
        let mut taxonomies: Vec<_> = cases
            .iter()
            .map(|(error_type, variant)| ErrorTaxonomy {
                code: ErrorTaxonomy::FIRST_VARIANT_CODE,
                error_type: error_type.to_string(),
                variant: variant.to_string(),
            })
            .collect();

        ErrorTaxonomy::ensure_unique_codes(&mut taxonomies);

        assert_eq!(taxonomies[0].code, ErrorTaxonomy::FIRST_VARIANT_CODE + 1);
        assert_eq!(taxonomies[1].code, ErrorTaxonomy::FIRST_VARIANT_CODE);
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
