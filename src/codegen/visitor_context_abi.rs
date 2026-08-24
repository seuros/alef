//! The C ABI shape of a trait-bridge visitor context struct.
//!
//! The FFI backend emits the visitor context as a `#[repr(C)]` struct, and every C-ABI host
//! binding has to read that exact struct back — same fields, in the same order, at the same
//! offsets. Deriving the shape twice is how the Java bridge came to hardcode one consumer's
//! field names and offsets, so the producer (`backends::ffi`) and the readers (`backends::java`)
//! share this one derivation instead. ~keep

use crate::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};

/// Pointer and `usize`/`isize` width assumed for the generated context struct.
///
/// The generated struct is emitted once and consumed by host bindings that cannot re-run this
/// derivation per target, so the width is fixed rather than probed. Every platform alef ships
/// C-ABI bindings for is 64-bit. ~keep
const POINTER_BYTES: u64 = 8;

/// Scalar slot a context field occupies in the generated `#[repr(C)]` context struct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ContextScalar {
    Pointer,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Isize,
    Usize,
}

impl ContextScalar {
    /// The Rust spelling the FFI backend writes into the `#[repr(C)]` struct definition.
    pub(crate) fn rust_c_type(self) -> &'static str {
        match self {
            Self::Pointer => "*const std::ffi::c_char",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Isize => "isize",
            Self::Usize => "usize",
        }
    }

    pub(crate) fn byte_size(self) -> u64 {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 => 4,
            Self::I64 | Self::U64 => 8,
            Self::Pointer | Self::Isize | Self::Usize => POINTER_BYTES,
        }
    }

    /// `#[repr(C)]` aligns every scalar in this set to its own width.
    pub(crate) fn byte_alignment(self) -> u64 {
        self.byte_size()
    }
}

/// How a host binding must interpret the scalar slot a context field occupies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ContextFieldShape {
    /// `char *` that is always non-null for the duration of the callback.
    RequiredString,
    /// `char *` that is null when the Rust value is `None`.
    OptionalString,
    /// `i32` carrying 0 or 1.
    Bool,
    /// `i32` carrying a fieldless enum's discriminant index.
    Enum,
    /// A plain integer, read and handed over unchanged.
    Integer,
}

/// One field of the generated context struct, placed at its `#[repr(C)]` offset.
pub(crate) struct ContextAbiField {
    pub name: String,
    pub doc: String,
    pub shape: ContextFieldShape,
    pub scalar: ContextScalar,
    pub byte_offset: u64,
    /// Padding `#[repr(C)]` inserts before this field to satisfy its alignment.
    pub leading_padding: u64,
}

/// The complete `#[repr(C)]` layout of a visitor context struct.
pub(crate) struct ContextAbi {
    pub fields: Vec<ContextAbiField>,
    pub byte_size: u64,
    pub byte_alignment: u64,
    /// Tail padding `#[repr(C)]` adds to round the struct up to its own alignment.
    pub trailing_padding: u64,
}

impl ContextAbi {
    pub(crate) fn field(&self, name: &str) -> Option<&ContextAbiField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// Derives the `#[repr(C)]` layout of `context_def` as the FFI backend emits it.
///
/// Fields whose type has no C representation are dropped from the struct — see
/// [`context_scalar`] — so the returned layout is the authority on which fields actually cross
/// the boundary, not `context_def.fields`.
pub(crate) fn context_abi(context_def: &TypeDef, api: &ApiSurface) -> ContextAbi {
    let mut fields = Vec::new();
    let mut offset = 0_u64;
    let mut alignment = 1_u64;

    for field in &context_def.fields {
        let Some((shape, scalar)) = context_scalar(field, api) else {
            tracing::warn!(
                "visitor context: skipping field `{}.{}` with unsupported type {:?}",
                context_def.name,
                field.name,
                field.ty
            );
            continue;
        };
        let field_alignment = scalar.byte_alignment();
        let leading_padding = (field_alignment - offset % field_alignment) % field_alignment;
        offset += leading_padding;
        fields.push(ContextAbiField {
            name: field.name.clone(),
            doc: context_field_doc(field),
            shape,
            scalar,
            byte_offset: offset,
            leading_padding,
        });
        offset += scalar.byte_size();
        alignment = alignment.max(field_alignment);
    }

    let byte_size = offset.div_ceil(alignment) * alignment;
    ContextAbi {
        fields,
        byte_alignment: alignment,
        trailing_padding: byte_size - offset,
        byte_size,
    }
}

/// Classifies a context field, or returns `None` when it has no C representation.
///
/// `None` means the field is absent from the emitted struct entirely: nested structs,
/// collections, floats, and every optional that is not `Option<String>` have no agreed C shape
/// here, and inventing one on either side of the boundary would put the two sides out of sync.
fn context_scalar(field: &FieldDef, api: &ApiSurface) -> Option<(ContextFieldShape, ContextScalar)> {
    use ContextFieldShape::{Bool, Enum, Integer, OptionalString, RequiredString};

    match (&field.ty, field.optional) {
        (TypeRef::String, false) => Some((RequiredString, ContextScalar::Pointer)),
        (TypeRef::String, true) => Some((OptionalString, ContextScalar::Pointer)),
        (TypeRef::Primitive(PrimitiveType::Bool), false) => Some((Bool, ContextScalar::I32)),
        (TypeRef::Primitive(PrimitiveType::U8), false) => Some((Integer, ContextScalar::U8)),
        (TypeRef::Primitive(PrimitiveType::U16), false) => Some((Integer, ContextScalar::U16)),
        (TypeRef::Primitive(PrimitiveType::U32), false) => Some((Integer, ContextScalar::U32)),
        (TypeRef::Primitive(PrimitiveType::U64), false) => Some((Integer, ContextScalar::U64)),
        (TypeRef::Primitive(PrimitiveType::I8), false) => Some((Integer, ContextScalar::I8)),
        (TypeRef::Primitive(PrimitiveType::I16), false) => Some((Integer, ContextScalar::I16)),
        (TypeRef::Primitive(PrimitiveType::I32), false) => Some((Integer, ContextScalar::I32)),
        (TypeRef::Primitive(PrimitiveType::I64), false) => Some((Integer, ContextScalar::I64)),
        (TypeRef::Primitive(PrimitiveType::Usize), false) => Some((Integer, ContextScalar::Usize)),
        (TypeRef::Primitive(PrimitiveType::Isize), false) => Some((Integer, ContextScalar::Isize)),
        (TypeRef::Named(name), false) if api.enums.iter().any(|enum_def| enum_def.name == *name) => {
            Some((Enum, ContextScalar::I32))
        }
        _ => None,
    }
}

fn context_field_doc(field: &FieldDef) -> String {
    field
        .doc
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("Context field.")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, FieldDef};

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..FieldDef::default()
        }
    }

    fn api_with_enum(name: &str) -> ApiSurface {
        ApiSurface {
            enums: vec![EnumDef {
                name: name.to_string(),
                ..EnumDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    fn context(fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: "TraversalState".to_string(),
            fields,
            ..TypeDef::default()
        }
    }

    #[test]
    fn should_pad_each_field_to_its_own_alignment() {
        let abi = context_abi(
            &context(vec![
                field("label", TypeRef::String, false),
                field("severity", TypeRef::Primitive(PrimitiveType::U8), false),
                field("active", TypeRef::Primitive(PrimitiveType::Bool), false),
                field("offset", TypeRef::Primitive(PrimitiveType::I16), false),
                field("note", TypeRef::String, true),
            ]),
            &ApiSurface::default(),
        );

        let offsets: Vec<u64> = abi.fields.iter().map(|field| field.byte_offset).collect();
        assert_eq!(offsets, vec![0, 8, 12, 16, 24]);
        let padding: Vec<u64> = abi.fields.iter().map(|field| field.leading_padding).collect();
        assert_eq!(padding, vec![0, 0, 3, 0, 6]);
        assert_eq!(abi.byte_size, 32);
        assert_eq!(abi.byte_alignment, 8);
        assert_eq!(abi.trailing_padding, 0);
    }

    #[test]
    fn should_round_struct_size_up_to_its_alignment() {
        let abi = context_abi(
            &context(vec![
                field("depth", TypeRef::Primitive(PrimitiveType::U64), false),
                field("flag", TypeRef::Primitive(PrimitiveType::Bool), false),
            ]),
            &ApiSurface::default(),
        );

        assert_eq!(abi.byte_size, 16);
        assert_eq!(abi.trailing_padding, 4);
    }

    #[test]
    fn should_drop_fields_with_no_c_representation() {
        let abi = context_abi(
            &context(vec![
                field("kept", TypeRef::Primitive(PrimitiveType::I32), false),
                field("weight", TypeRef::Primitive(PrimitiveType::F64), false),
                field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
                field("count", TypeRef::Primitive(PrimitiveType::U32), true),
                field("nested", TypeRef::Named("Other".to_string()), false),
            ]),
            &ApiSurface::default(),
        );

        let names: Vec<&str> = abi.fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names, vec!["kept"]);
        assert!(abi.field("weight").is_none());
    }

    #[test]
    fn should_carry_a_fieldless_enum_as_an_i32_discriminant() {
        let abi = context_abi(
            &context(vec![field("kind", TypeRef::Named("TraversalKind".to_string()), false)]),
            &api_with_enum("TraversalKind"),
        );

        let kind = abi.field("kind").expect("enum field is carried");
        assert_eq!(kind.shape, ContextFieldShape::Enum);
        assert_eq!(kind.scalar, ContextScalar::I32);
        assert_eq!(kind.scalar.rust_c_type(), "i32");
    }

    #[test]
    fn should_distinguish_required_from_optional_strings() {
        let abi = context_abi(
            &context(vec![
                field("name", TypeRef::String, false),
                field("parent", TypeRef::String, true),
            ]),
            &ApiSurface::default(),
        );

        assert_eq!(
            abi.field("name").expect("name").shape,
            ContextFieldShape::RequiredString
        );
        assert_eq!(
            abi.field("parent").expect("parent").shape,
            ContextFieldShape::OptionalString
        );
        assert_eq!(
            abi.field("parent").expect("parent").scalar.rust_c_type(),
            "*const std::ffi::c_char"
        );
    }
}
