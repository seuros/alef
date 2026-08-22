//! Cross-backend order-invariance coverage for generated output (task #132).
//!
//! Generating twice in one process does NOT catch ordering leaks: the hash seed is constant within
//! a process, so an unordered lookup yields the same iteration order on both calls, and the two
//! runs agree even on buggy code. The check that discriminates is order *invariance* — feed the
//! SAME logical IR through a backend twice, once with the `ApiSurface` item vectors reversed, and
//! require byte-identical output. Any emission loop that concatenates `types`/`enums`/`functions`/
//! `errors` into one file in raw `Vec` order fails it, and its diff has the misleading shape of
//! content substitution (a positional line diff shows one item's text where another item's text
//! now sits) rather than obvious reordering.
//!
//! Reversal is a stand-in for "some other arrival order". The incoming order reflects
//! source-extraction and pipeline-orchestration order; a backend whose output is a function of IR
//! *content* alone is immune to any instability there, whether or not one exists today.

use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef, VersionAnnotation,
};

/// Languages this test does not yet assert on, each with the reason it is excluded. Every other
/// language with a registered backend is covered. Shrinking this list is the goal; nothing should
/// ever be added to it to make a failure go away. ~keep
const NOT_YET_ORDER_INVARIANT: &[(Language, &str)] = &[(
    Language::Dart,
    "dart's emission loops still iterate api.types/enums/functions/errors in raw Vec order",
)];

fn payload_type(name: &str, field: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        fields: vec![FieldDef {
            name: field.to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: format!("{field}_doubled"),
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }
}

/// A flat data enum: each variant carries one tuple field of a named type. `tag` mirrors an
/// explicit `#[serde(tag = "...")]`; `None` exercises the generic `"type"` discriminator fallback,
/// which is where the consumer report that opened task #132 saw `format_type` and `type` swap.
fn flat_data_enum(name: &str, tag: Option<&str>, variants: &[(&str, &str)]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        variants: variants
            .iter()
            .map(|(variant_name, field_type)| EnumVariant {
                name: (*variant_name).to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named((*field_type).to_string()),
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

fn free_function(name: &str, param_type: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::Named(param_type.to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        ..Default::default()
    }
}

fn error_def(name: &str, variants: &[&str]) -> ErrorDef {
    ErrorDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|variant| ErrorVariant {
                name: (*variant).to_string(),
                message_template: Some(format!("{variant} failed")),
                ..Default::default()
            })
            .collect(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: VersionAnnotation::default(),
    }
}

/// Several entries in every top-level IR vector, so a raw-order concatenation cannot accidentally
/// survive reversal. Three flat data enums — one explicitly tagged, two on the `"type"` fallback —
/// additionally stress any shared-discriminator lookup keyed by an unordered collection.
fn determinism_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            payload_type("PdfMetadata", "pages"),
            payload_type("DocxMetadata", "words"),
            payload_type("NodeMetadata", "depth"),
            payload_type("LeafMetadata", "value"),
            payload_type("AnnotationDataA", "note"),
            payload_type("AnnotationDataB", "tag"),
        ],
        enums: vec![
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
        ],
        functions: vec![
            free_function("summarize_pdf", "PdfMetadata"),
            free_function("summarize_docx", "DocxMetadata"),
            free_function("summarize_node", "NodeMetadata"),
        ],
        errors: vec![
            error_def("ParseError", &["Malformed", "Truncated"]),
            error_def("IoError", &["NotFound", "Denied"]),
        ],
        ..Default::default()
    }
}

fn every_language_config() -> ResolvedCrateConfig {
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = [
    "python", "node", "ruby", "php", "elixir", "wasm", "ffi", "go", "java", "csharp",
    "r", "kotlin", "kotlin_android", "swift", "dart", "gleam", "zig", "jni",
]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
    )
    .expect("every-language test config must parse");
    cfg.resolve()
        .expect("every-language test config must resolve")
        .remove(0)
}

/// All four file-producing `Backend` entry points, keyed by path and ordered by it, so the
/// comparison is over content only and never over the order the backend happened to return in.
fn generated_files_sorted(
    backend: &dyn Backend,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();
    let produced: [anyhow::Result<Vec<GeneratedFile>>; 4] = [
        backend.generate_bindings(api, config),
        backend.generate_public_api(api, config),
        backend.generate_type_stubs(api, config),
        backend.generate_service_api(api, config),
    ];
    for result in produced {
        let generated = result.unwrap_or_else(|error| panic!("{} generation failed: {error}", backend.name()));
        files.extend(
            generated
                .into_iter()
                .map(|file| (file.path.to_string_lossy().into_owned(), file.content)),
        );
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Reports the first differing file and the first differing line inside it, rather than dumping
/// two whole generated trees into the failure output.
fn describe_first_difference(left: &[(String, String)], right: &[(String, String)]) -> Option<String> {
    let left_paths: Vec<&str> = left.iter().map(|(path, _)| path.as_str()).collect();
    let right_paths: Vec<&str> = right.iter().map(|(path, _)| path.as_str()).collect();
    if left_paths != right_paths {
        return Some(format!(
            "generated file sets differ:\n  forward:  {left_paths:?}\n  reversed: {right_paths:?}"
        ));
    }

    let (path, left_content, right_content) = left
        .iter()
        .zip(right)
        .find(|((_, left_content), (_, right_content))| left_content != right_content)
        .map(|((path, left_content), (_, right_content))| (path, left_content, right_content))?;

    let left_lines: Vec<&str> = left_content.lines().collect();
    let right_lines: Vec<&str> = right_content.lines().collect();
    let first_diff = left_lines
        .iter()
        .zip(&right_lines)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| left_lines.len().min(right_lines.len()));
    let window = first_diff.saturating_sub(2)..(first_diff + 5);
    let render = |lines: &[&str]| {
        lines
            .iter()
            .enumerate()
            .filter(|(index, _)| window.contains(index))
            .map(|(index, line)| format!("{:>5} | {line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Some(format!(
        "file: {path}\nfirst difference at line {}\n--- forward ---\n{}\n--- reversed ---\n{}",
        first_diff + 1,
        render(&left_lines),
        render(&right_lines),
    ))
}

fn reversed(api: &ApiSurface) -> ApiSurface {
    let mut reversed = api.clone();
    reversed.types.reverse();
    reversed.enums.reverse();
    reversed.functions.reverse();
    reversed.errors.reverse();
    reversed
}

/// Compares forward-order output against reversed-order output.
///
/// A backend whose output is a pure function of (IR, config) agrees on every call regardless of
/// what else is running concurrently. This used to retry across a "stable environment" bracket to
/// tolerate the swift backend picking its output path prefix by probing the filesystem for a
/// `Sources/` layout on disk -- nondeterministic under the rest of the test suite churning those
/// directories (observed: swift emitting `Sources/RustBridge/...` on one call and
/// `packages/swift/Sources/RustBridge/...` on the next). Swift's package root is now derived only
/// from config (see `swift_package_root` in `backends::swift::gen_bindings`), so a direct
/// comparison is the real check again: any remaining flake here means some backend still reads
/// state outside the IR, and that is exactly the bug this test exists to catch. ~keep
fn describe_order_difference(
    backend: &dyn Backend,
    forward: &ApiSurface,
    backwards: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> Option<String> {
    let forward_files = generated_files_sorted(backend, forward, config);
    let reversed_files = generated_files_sorted(backend, backwards, config);
    describe_first_difference(&forward_files, &reversed_files)
}

fn covered_languages() -> Vec<Language> {
    Language::ALL
        .iter()
        .copied()
        .filter(|lang| crate::cli::registry::try_get_backend(*lang).is_some())
        .filter(|lang| !NOT_YET_ORDER_INVARIANT.iter().any(|(excluded, _)| excluded == lang))
        .collect()
}

#[test]
fn every_covered_backend_is_invariant_to_ir_collection_order() {
    // Several backends resolve `version_from` (default "Cargo.toml") with a relative-path
    // `std::fs::read_to_string` -- resolved against the process's current directory, which is
    // shared mutable state across every test thread in this binary. Without this lock, a sibling
    // test that legitimately chdirs mid-run (via `test_support::CwdGuard`) can make the forward
    // and reversed calls below observe two different `Cargo.toml`s -- or the repo's real one vs.
    // none at all -- producing a spurious content diff that has nothing to do with IR order. Hold
    // the one lock every cwd-mutating test already shares so no other test can move the process
    // cwd out from under these two calls. See `test_support` module docs. ~keep
    let _cwd_lock = crate::test_support::CWD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let config = every_language_config();
    let forward = determinism_api();
    let backwards = reversed(&forward);

    let mut failures: Vec<String> = Vec::new();
    for language in covered_languages() {
        let backend = crate::cli::registry::try_get_backend(language).expect("filtered to registered backends");
        if let Some(difference) = describe_order_difference(backend.as_ref(), &forward, &backwards, &config) {
            failures.push(format!("=== {language:?} ===\n{difference}"));
        }
    }

    assert!(
        failures.is_empty(),
        "reversing api.types/enums/functions/errors must not change any generated file. A diff \
         here means the backend leaks IR Vec ordering into emitted text, so two `alef` runs over \
         an unchanged tree can disagree.\n\n{}",
        failures.join("\n\n")
    );
}

/// Guards the exclusion list against becoming stale in the other direction: once a backend is
/// fixed it must be moved out of `NOT_YET_ORDER_INVARIANT` and into the asserted set.
#[test]
fn excluded_backends_are_still_actually_failing() {
    // Same cwd-race guard as `every_covered_backend_is_invariant_to_ir_collection_order` above --
    // see that test's comment. ~keep
    let _cwd_lock = crate::test_support::CWD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let config = every_language_config();
    let forward = determinism_api();
    let backwards = reversed(&forward);

    for (language, reason) in NOT_YET_ORDER_INVARIANT {
        let backend = crate::cli::registry::try_get_backend(*language).expect("excluded languages have backends");
        let forward_files = generated_files_sorted(backend.as_ref(), &forward, &config);
        let reversed_files = generated_files_sorted(backend.as_ref(), &backwards, &config);
        assert!(
            describe_first_difference(&forward_files, &reversed_files).is_some(),
            "{language:?} is now order-invariant, so it must be removed from \
             NOT_YET_ORDER_INVARIANT (recorded reason: {reason})"
        );
    }
}
