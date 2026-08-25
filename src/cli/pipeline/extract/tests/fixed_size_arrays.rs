//! Fixed-size arrays of a named type that the binding surface already carries.
//!
//! `resolve_type` has no IR variant for `syn::Type::Array`, so the extractor stringifies the
//! whole array into a `TypeRef::Named`. Every test here starts from the exact string the real
//! resolver produces (pinned by [`the_resolver_stringifies_a_fixed_size_array_into_a_named_type`])
//! so a hand-written fixture cannot drift out of the sanitization path it is supposed to exercise.

use crate::backends::go::type_map::go_type;
use crate::backends::java::gen_bindings::test_only_gen_record_type;
use crate::cli::pipeline::extract::sanitizer::{TypeSanitization, sanitize_type_ref, sanitize_unknown_types};
use crate::core::config::JavaBuilderMode;
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};
use crate::core::validation::{ValidationCode, validate_api_surface_with_bridged_traits};
use crate::extract::type_resolver::resolve_type;
use ahash::AHashSet;
use std::collections::HashSet;

/// The literal `TypeRef::Named` payload the extractor produces for `[Point; 4]`.
///
/// `quote` renders the array's tokens with a space before the semicolon and `normalize_type_string`
/// only strips spaces adjacent to bracket/paren/comma punctuation, so the `;` keeps its leading
/// space. Anything parsing this string has to tolerate that. ~keep
const RESOLVED_POINT_ARRAY: &str = "[Point ; 4]";

fn known(names: &[&str]) -> AHashSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

fn surface_with_point_and_field(field_ty: TypeRef) -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![
            TypeDef {
                name: "Point".to_string(),
                rust_path: "sample::Point".to_string(),
                ..TypeDef::default()
            },
            TypeDef {
                name: "Quad".to_string(),
                rust_path: "sample::Quad".to_string(),
                fields: vec![FieldDef {
                    name: "corners".to_string(),
                    ty: field_ty,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

fn corners(api: &ApiSurface) -> &FieldDef {
    &api.types[1].fields[0]
}

#[test]
fn the_resolver_stringifies_a_fixed_size_array_into_a_named_type() {
    let parsed = syn::parse_str::<syn::Type>("[Point; 4]").expect("fixture must parse as a Rust type");

    assert_eq!(
        resolve_type(&parsed),
        TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()),
        "the sanitizer fixtures below are only meaningful if this is the string the extractor emits"
    );
}

#[test]
fn a_fixed_size_array_of_a_known_type_lowers_to_a_typed_list() {
    let mut api = surface_with_point_and_field(TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()));
    assert_eq!(
        corners(&api).ty,
        TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()),
        "pre-condition: the field must still be the unlowered array string"
    );

    sanitize_unknown_types(&mut api);

    let corners = corners(&api);
    assert_eq!(corners.ty, TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))));
    assert!(!corners.sanitized, "a lowered array is not a lossy rewrite");
    assert_eq!(
        corners.original_type.as_deref(),
        Some(RESOLVED_POINT_ARRAY),
        "the declared length is the one fact `Vec<Point>` cannot carry, so it must be recorded"
    );
}

#[test]
fn a_fixed_size_array_of_a_known_type_does_not_trip_the_lossy_surface_gate() {
    let mut api = surface_with_point_and_field(TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()));
    sanitize_unknown_types(&mut api);

    let report = validate_api_surface_with_bridged_traits(&api, &AHashSet::default());
    let lossy: Vec<_> = report
        .errors()
        .filter(|d| d.code == ValidationCode::LossySanitizedSurface)
        .collect();

    assert!(
        lossy.is_empty(),
        "expected no lossy_sanitized_surface errors, got {lossy:?}"
    );
}

#[test]
fn an_optional_fixed_size_array_of_a_known_type_lowers_inside_the_option() {
    let mut api = surface_with_point_and_field(TypeRef::Optional(Box::new(TypeRef::Named(
        RESOLVED_POINT_ARRAY.to_string(),
    ))));

    sanitize_unknown_types(&mut api);

    let corners = corners(&api);
    assert_eq!(
        corners.ty,
        TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string())))))
    );
    assert!(!corners.sanitized);
    assert_eq!(corners.original_type.as_deref(), Some("Option<[Point ; 4]>"));
}

#[test]
fn a_fixed_size_array_of_a_known_type_lowers_in_a_method_signature() {
    let mut api = surface_with_point_and_field(TypeRef::String);
    api.types[1].methods = vec![MethodDef {
        name: "translate".to_string(),
        params: vec![ParamDef {
            name: "corners".to_string(),
            ty: TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()),
        ..MethodDef::default()
    }];

    sanitize_unknown_types(&mut api);

    let method = &api.types[1].methods[0];
    let expected = TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string())));
    assert_eq!(method.params[0].ty, expected);
    assert_eq!(method.return_type, expected);
    assert!(!method.params[0].sanitized);
    assert!(!method.sanitized);
    assert_eq!(
        method.params[0].original_type.as_deref(),
        Some(RESOLVED_POINT_ARRAY),
        "a param's declared length is erased by Vec<Point> exactly like a field's, so it must be recorded too"
    );
}

#[test]
fn a_fixed_size_array_of_a_known_type_reports_lossless_sanitization() {
    let mut ty = TypeRef::Named(RESOLVED_POINT_ARRAY.to_string());

    let status = sanitize_type_ref(&mut ty, &known(&["Point"]), &AHashSet::default());

    assert_eq!(status, TypeSanitization::Lossless);
    assert_eq!(ty, TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))));
}

#[test]
fn a_fixed_size_array_of_a_known_enum_lowers_to_a_typed_list() {
    let mut ty = TypeRef::Named("[Corner ; 4]".to_string());

    let status = sanitize_type_ref(&mut ty, &AHashSet::default(), &known(&["Corner"]));

    assert_eq!(status, TypeSanitization::Lossless);
    assert_eq!(ty, TypeRef::Vec(Box::new(TypeRef::Named("Corner".to_string()))));
}

#[test]
fn a_fixed_size_array_of_an_unsupported_type_still_sanitizes_to_a_json_string() {
    let mut api = surface_with_point_and_field(TypeRef::Named("[Offshore ; 3]".to_string()));

    sanitize_unknown_types(&mut api);

    let corners = corners(&api);
    assert_eq!(corners.ty, TypeRef::String);
    assert!(corners.sanitized);
    assert_eq!(corners.original_type.as_deref(), Some("[Offshore ; 3]"));
}

#[test]
fn a_fixed_size_array_of_tuples_keeps_its_json_string_fallback() {
    let mut api = surface_with_point_and_field(TypeRef::Named("[(u32,u32); 4]".to_string()));

    sanitize_unknown_types(&mut api);

    let corners = corners(&api);
    assert_eq!(corners.ty, TypeRef::String);
    assert!(
        corners.sanitized,
        "the wasm backend reconstructs this shape from a sanitized JSON string"
    );
    assert_eq!(corners.original_type.as_deref(), Some("[(u32,u32); 4]"));
}

#[test]
fn a_growable_vec_of_a_known_type_is_untouched() {
    let mut api = surface_with_point_and_field(TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))));

    sanitize_unknown_types(&mut api);

    let corners = corners(&api);
    assert_eq!(corners.ty, TypeRef::Vec(Box::new(TypeRef::Named("Point".to_string()))));
    assert!(!corners.sanitized);
    assert_eq!(corners.original_type, None);
}

/// A nicer IR is worth nothing if the emitted binding is still a placeholder, so drive the
/// sanitized `TypeDef` through the real Java record generator -- the same function `alef build`
/// calls -- and read the rendered source. Go's type mapper is asserted alongside it because the
/// two backends reach the list shape by different routes. ~keep
#[test]
fn a_lowered_fixed_size_array_renders_as_a_typed_list_in_generated_bindings() {
    let mut api = surface_with_point_and_field(TypeRef::Named(RESOLVED_POINT_ARRAY.to_string()));
    sanitize_unknown_types(&mut api);

    let visible: HashSet<&str> = ["Point", "Quad"].into_iter().collect();
    let rendered = test_only_gen_record_type(
        "dev.sample.bindings",
        &api.types[1],
        &AHashSet::new(),
        &AHashSet::new(),
        "",
        &[],
        "",
        JavaBuilderMode::Always,
        &ahash::AHashMap::new(),
        &AHashSet::new(),
        &visible,
    );

    assert!(
        rendered.contains("List<Point> corners"),
        "expected a typed list of the DTO in the generated record, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("String corners") && !rendered.contains("List<Object> corners"),
        "the JSON-string placeholder must be gone from the generated record, got:\n{rendered}"
    );

    assert_eq!(go_type(&corners(&api).ty), "[]Point");
}
