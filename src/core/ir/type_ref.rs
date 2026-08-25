use serde::{Deserialize, Serialize};

/// Reference to a type, with enough info for codegen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TypeRef {
    Primitive(PrimitiveType),
    String,
    /// Rust `char` — single Unicode character. Binding layer represents as single-char string. ~keep
    Char,
    Bytes,
    Optional(Box<TypeRef>),
    Vec(Box<TypeRef>),
    Map(Box<TypeRef>, Box<TypeRef>),
    Named(String),
    Path,
    #[default]
    Unit,
    Json,
    Duration,
}

impl TypeRef {
    /// Render this type as the Rust source text it stands for.
    ///
    /// Unlike the per-language mappers this performs no normalization and no naming policy: a
    /// `Named` leaf is emitted verbatim, including names that are not part of the binding
    /// surface. That is the point — it is used to capture a type *before* the sanitizer
    /// rewrites unbindable leaves to `String`, so the Rust-facing surfaces can still show what
    /// the source declared. ~keep
    pub fn rust_source_display(&self) -> String {
        match self {
            Self::Primitive(p) => p.rust_source_display().to_string(),
            Self::String => "String".to_string(),
            Self::Char => "char".to_string(),
            Self::Bytes => "Vec<u8>".to_string(),
            Self::Optional(inner) => format!("Option<{}>", inner.rust_source_display()),
            Self::Vec(inner) => format!("Vec<{}>", inner.rust_source_display()),
            Self::Map(key, value) => {
                format!(
                    "HashMap<{}, {}>",
                    key.rust_source_display(),
                    value.rust_source_display()
                )
            }
            Self::Named(name) => name.clone(),
            Self::Path => "PathBuf".to_string(),
            Self::Unit => "()".to_string(),
            Self::Json => "serde_json::Value".to_string(),
            Self::Duration => "Duration".to_string(),
        }
    }

    /// Returns true if this type reference contains `Named(name)` at any depth.
    pub fn references_named(&self, name: &str) -> bool {
        match self {
            Self::Named(n) => n == name,
            Self::Optional(inner) | Self::Vec(inner) => inner.references_named(name),
            Self::Map(k, v) => k.references_named(name) || v.references_named(name),
            _ => false,
        }
    }
}

/// Rust primitive types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PrimitiveType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Usize,
    Isize,
}

impl PrimitiveType {
    pub fn rust_source_display(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Usize => "usize",
            Self::Isize => "isize",
        }
    }
}
