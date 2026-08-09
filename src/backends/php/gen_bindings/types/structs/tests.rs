use super::*;

#[cfg(test)]
mod redundant_field_shorthand_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::PrimitiveType;

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    /// `from_json`-constructor path (`use_from_json == true`): a single-word field name
    /// (e.g. `provider`) is already valid snake_case, so `to_php_name` maps it to itself.
    /// The emitted `Self { ... }` literal must use field-init shorthand, not `provider:
    /// provider`.
    #[test]
    fn from_json_constructor_uses_shorthand_for_single_word_field() {
        let typ = TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "test_lib::LlmConfig".to_string(),
            fields: vec![field("provider", TypeRef::String, false)],
            has_default: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        );

        assert!(
            !out.contains("provider: provider"),
            "single-word field must use shorthand, not `provider: provider`:\n{out}"
        );
        assert!(
            out.contains("{ provider }") || out.contains("{ provider, "),
            "expected field-init shorthand for `provider`:\n{out}"
        );
    }

    /// Same `from_json`-constructor path, but for a multi-word field whose PHP param name
    /// (`chunkSize`) genuinely differs from the Rust field name (`chunk_size`) — the real
    /// `field: expr` form must still be emitted.
    #[test]
    fn from_json_constructor_keeps_named_form_for_multi_word_field() {
        let typ = TypeDef {
            name: "ChunkConfig".to_string(),
            rust_path: "test_lib::ChunkConfig".to_string(),
            fields: vec![field("chunk_size", TypeRef::Primitive(PrimitiveType::U32), false)],
            has_default: true,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            true,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        );

        assert!(
            out.contains("chunk_size: chunkSize"),
            "differing param name must keep explicit field-init form:\n{out}"
        );
    }

    /// Plain `#[php(constructor)]` path (no serde, no defaults): single-word field must
    /// also emit shorthand rather than `label: label`.
    #[test]
    fn plain_constructor_uses_shorthand_for_single_word_field() {
        let typ = TypeDef {
            name: "SimpleLabel".to_string(),
            rust_path: "test_lib::SimpleLabel".to_string(),
            fields: vec![field("label", TypeRef::String, false)],
            has_default: false,
            ..Default::default()
        };

        let out = gen_struct_methods_with_exclude(
            &typ,
            &mapper(),
            false,
            "test_lib",
            &AHashSet::new(),
            &AHashSet::new(),
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        );

        assert!(
            !out.contains("label: label"),
            "single-word field must use shorthand, not `label: label`:\n{out}"
        );
        assert!(
            out.contains("{ label }") || out.contains("{ label, "),
            "expected field-init shorthand for `label`:\n{out}"
        );
    }
}

#[cfg(test)]
mod optional_named_setter_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;

    fn mapper_with_enums(enum_names: &AHashSet<String>) -> PhpMapper {
        PhpMapper {
            enum_names: enum_names.clone(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..Default::default()
        }
    }

    fn names(values: &[&str]) -> AHashSet<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    /// Generate the `#[php_impl]` block for a one-field struct, with the caller controlling
    /// which type names are enums and which are opaque.
    fn gen_one_field_impl(field: FieldDef, enum_names: &AHashSet<String>, opaque_types: &AHashSet<String>) -> String {
        let typ = TypeDef {
            name: "Consumer".to_string(),
            rust_path: "test_lib::Consumer".to_string(),
            fields: vec![field],
            has_default: true,
            has_serde: true,
            ..Default::default()
        };

        gen_struct_methods_with_exclude(
            &typ,
            &mapper_with_enums(enum_names),
            true,
            "test_lib",
            opaque_types,
            enum_names,
            &[],
            &[],
            &AHashSet::new(),
            &[],
            &AHashSet::new(),
            &[],
        )
    }

    /// `class_derives!` — what `#[php_class]` expands to — implements `FromZval<'a>` only for
    /// `&'a T`, so an owned `T` parameter fails ext-php-rs's argument bound with E0277. The
    /// setter must therefore take `Option<&T>`, matching the shape `gen_php_function_params`
    /// already emits for optional non-opaque Named function parameters.
    #[test]
    fn optional_named_struct_setter_takes_a_shared_reference() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn set_ocr(&mut self, value: Option<&OcrConfig>)"),
            "the setter must borrow the wrapped struct, not take it by value:\n{out}"
        );
        assert!(
            !out.contains("value: Option<OcrConfig>"),
            "an owned `Option<T>` parameter does not implement `FromZvalMut` and fails to \
             compile with E0277:\n{out}"
        );
    }

    /// The backing field is owned, so the setter has to clone out of the borrow before
    /// converting — the same `v.clone().into()` step `gen_php_call_args_vec` performs for a
    /// `&T` call argument.
    #[test]
    fn optional_named_struct_setter_clones_out_of_the_borrow() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("self.ocr = value.map(|value| value.clone().into());"),
            "the setter must write a cloned, converted value through to the backing field:\n{out}"
        );
    }

    /// The getter is unaffected: `class_derives!` implements `IntoZval` for the owned `T`, so
    /// returning `Option<T>` by value is already correct and must stay as it is.
    #[test]
    fn optional_named_struct_getter_still_returns_an_owned_value() {
        let out = gen_one_field_impl(
            field(
                "ocr",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrConfig".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn get_ocr(&self) -> Option<OcrConfig>"),
            "the getter must keep returning an owned wrapper:\n{out}"
        );
    }

    /// A field typed as a bare `Named` but marked optional takes the same borrowed shape —
    /// the emitter must not fall back to the owned `Option<T>` form for it either.
    #[test]
    fn bare_named_optional_field_setter_takes_a_shared_reference() {
        let out = gen_one_field_impl(
            field("chunking", TypeRef::Named("ChunkingConfig".to_string()), true),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            out.contains("pub fn set_chunking(&mut self, value: Option<&ChunkingConfig>)"),
            "an optional bare-Named field must also borrow its parameter:\n{out}"
        );
        assert!(
            out.contains("self.chunking = value.map(|value| value.clone().into());"),
            "an optional bare-Named field must clone out of the borrow too:\n{out}"
        );
    }

    /// Unchanged behaviour: an `Option<enum>` field maps to PHP `String` and is already a real
    /// `#[php(prop)]` promoted `readonly` constructor param, so it gets no mutator at all.
    #[test]
    fn optional_named_enum_field_gets_no_setter() {
        let enum_names = names(&["OcrStrategy"]);
        let out = gen_one_field_impl(
            field(
                "ocr_strategy",
                TypeRef::Optional(Box::new(TypeRef::Named("OcrStrategy".to_string()))),
                true,
            ),
            &enum_names,
            &AHashSet::new(),
        );

        assert!(
            !out.contains("pub fn set_ocr_strategy"),
            "an enum field is a readonly promoted property and must not get a back-door \
             mutator:\n{out}"
        );
    }

    /// Unchanged behaviour: opaque handles are reached through their own wither methods, so
    /// they must not additionally get a conflicting `set_<field>`.
    #[test]
    fn optional_opaque_field_gets_no_setter() {
        let out = gen_one_field_impl(
            field(
                "engine",
                TypeRef::Optional(Box::new(TypeRef::Named("EngineHandle".to_string()))),
                true,
            ),
            &AHashSet::new(),
            &names(&["EngineHandle"]),
        );

        assert!(
            !out.contains("pub fn set_engine"),
            "opaque fields must not get a generated setter:\n{out}"
        );
    }

    /// Unchanged behaviour: a scalar field is a plain `#[php(prop)]` and needs no method.
    #[test]
    fn optional_scalar_field_gets_no_setter() {
        let out = gen_one_field_impl(
            field("cache_namespace", TypeRef::Optional(Box::new(TypeRef::String)), true),
            &AHashSet::new(),
            &AHashSet::new(),
        );

        assert!(
            !out.contains("pub fn set_cache_namespace"),
            "a scalar prop field must not get a generated setter:\n{out}"
        );
    }
}
