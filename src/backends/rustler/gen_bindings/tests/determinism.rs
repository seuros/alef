//! Regression coverage for task #132: alef's rustler/elixir codegen was observed to differ
//! between two `alef all` runs on an unchanged tree. The suspected shape is a discriminator or
//! naming lookup fed by an unordered collection (`AHashMap`/`AHashSet`), where iteration order
//! (or insertion order coming from an unsorted upstream `Vec`) leaks into emitted text.
//!
//! Rust's default/`ahash` hasher state is randomized per *process*, not necessarily per
//! `HashMap`/`HashSet` instance within one process, so calling the generator twice in one test
//! process can pass even when real, separate `alef` invocations would differ. To catch that class
//! of bug without needing multiple processes, `rustler_generation_is_invariant_to_ir_collection_order`
//! feeds the SAME logical IR through the generator twice with its `types`/`enums` vectors in two
//! different orders and asserts byte-identical output: any codegen path that depends on iteration
//! order over an unordered collection keyed by these entries would surface here as a diff. ~keep

use super::{RustlerBackend, test_api, test_config};
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn payload_type(name: &str, field: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_crate::{name}"),
        fields: vec![FieldDef {
            name: field.to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }
}

/// A flat data enum: every variant carries exactly one tuple field of a named type. `tag` mirrors
/// an explicit `#[serde(tag = "...")]`; `None` exercises the `flat_data_enum_discriminator`
/// fallback (`"type"`).
fn flat_data_enum(name: &str, tag: Option<&str>, variants: &[(&str, &str)]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("my_crate::{name}"),
        variants: variants
            .iter()
            .map(|(variant_name, field_type)| EnumVariant {
                name: variant_name.to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named(field_type.to_string()),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            })
            .collect(),
        serde_tag: tag.map(str::to_string),
        has_serde: true,
        ..Default::default()
    }
}

/// Three flat data enums -- one explicitly tagged (mirroring the `format_type` case from the
/// task's consumer-repo report) and two relying on the generic `"type"` fallback -- plus their
/// payload struct types. Multiple untagged flat enums stress any shared-name/shared-fallback path
/// that might resolve which enum "owns" a discriminator via an unordered lookup.
fn determinism_api() -> ApiSurface {
    let mut api = test_api();
    api.types = vec![
        payload_type("PdfMetadata", "pages"),
        payload_type("DocxMetadata", "words"),
        payload_type("NodeMetadata", "depth"),
        payload_type("LeafMetadata", "value"),
        payload_type("AnnotationDataA", "note"),
        payload_type("AnnotationDataB", "tag"),
    ];
    api.enums = vec![
        flat_data_enum(
            "FormatMetadata",
            Some("format_type"),
            &[("Pdf", "PdfMetadata"), ("Docx", "DocxMetadata")],
        ),
        flat_data_enum(
            "NodeKind",
            None,
            &[("Branch", "NodeMetadata"), ("Leaf", "LeafMetadata")],
        ),
        flat_data_enum(
            "AnnotationKind",
            None,
            &[("Comment", "AnnotationDataA"), ("Highlight", "AnnotationDataB")],
        ),
    ];
    api
}

fn generated_files_sorted(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<(String, String)> {
    let backend = RustlerBackend;
    let bindings: Vec<GeneratedFile> = backend.generate_bindings(api, config).expect("generate_bindings");
    let public_api: Vec<GeneratedFile> = backend.generate_public_api(api, config).expect("generate_public_api");

    let mut files: Vec<(String, String)> = bindings
        .into_iter()
        .chain(public_api)
        .map(|file| (file.path.to_string_lossy().into_owned(), file.content))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn rustler_generation_is_byte_identical_across_repeated_runs() {
    let config = test_config();
    let api = determinism_api();

    let run1 = generated_files_sorted(&api, &config);
    let run2 = generated_files_sorted(&api, &config);

    assert_eq!(
        run1, run2,
        "generating twice from the same IR in one process must be byte-identical; a diff here \
         means some rustler codegen path is not even a pure function of its input"
    );
}

#[test]
fn rustler_generation_is_invariant_to_ir_collection_order() {
    let config = test_config();
    let forward = determinism_api();

    let mut reversed = forward.clone();
    reversed.types.reverse();
    reversed.enums.reverse();

    let forward_files = generated_files_sorted(&forward, &config);
    let reversed_files = generated_files_sorted(&reversed, &config);

    assert_eq!(
        forward_files, reversed_files,
        "reversing api.types/api.enums must not change any generated file's content; a diff here \
         means codegen leaks Vec ordering (or an unordered collection built from it, e.g. a \
         discriminator/name lookup) into emitted text -- exactly the class of bug that made two \
         `alef all` runs disagree on an unchanged tree"
    );
}
