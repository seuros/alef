//! `[crates.<lang>] rename_fields` must reach every backend that emits a public DTO field
//! surface, or the setting is a silent no-op in that language rather than an error.
//!
//! Audit of all seventeen target languages, emitter by emitter. "Public field surface" means an
//! identifier a consumer of the generated binding spells to read or write a value:
//!
//! | backend        | public DTO field surface                          | honors `rename_fields` |
//! |----------------|---------------------------------------------------|------------------------|
//! | pyo3           | `#[pyo3(name)]` attributes and `.pyi` stubs       | yes (pre-existing)     |
//! | zig            | `pub const T = struct { field: … }` members       | yes (this file)        |
//! | dart (ffi)     | `@freezed` named parameters / properties          | yes (this file)        |
//! | napi           | `#[napi(object)]` props and `.d.ts` members       | no — outstanding       |
//! | wasm           | `js_name` getter/setter property pairs            | no — outstanding       |
//! | go             | exported struct fields                            | no — outstanding       |
//! | java           | `record` components                               | no — outstanding       |
//! | kotlin (mpp)   | `data class` `val` properties                     | no — outstanding       |
//! | kotlin_android | shares the Kotlin `data class` emitter            | no — outstanding       |
//! | csharp         | `init`-only auto-properties                       | no — outstanding       |
//! | swift          | `public let` properties and memberwise init       | no — outstanding       |
//! | gleam          | labeled record constructor fields                 | no — outstanding       |
//! | extendr (R)    | named `conversion_options()` args and list keys   | no — outstanding       |
//! | rustler        | `defstruct` atoms and the `@type` declaration     | no — outstanding       |
//! | php            | `public readonly` props, getters, ctor params     | no — outstanding       |
//! | magnus (Ruby)  | field-named getter methods and kwargs keys        | no — outstanding       |
//!
//! Four emitters have no public DTO field surface at all, and each carries a `~keep` rationale at
//! its emission site rather than an entry here: `backends::ffi` (opaque handle plus a
//! `{prefix}_{type}_{field}()` accessor symbol — a C symbol, not a field), `backends::jni` (only
//! `Java_*` symbols), the Kotlin JVM target (DTOs are `typealias`es onto the Java records, so
//! Java's naming governs), and Dart's default FRB style (flutter_rust_bridge generates the Dart
//! class from alef's Rust mirror, so alef never spells a Dart field name).
//!
//! The control tests matter as much as the positive ones: `rename_fields` resolves to `None` when
//! nothing is configured, so an emitter that ignored the config entirely would still pass the
//! positive assertion if the fixture happened to round-trip. ~keep

use alef::backends::dart::DartBackend;
use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn point_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Point".into(),
            rust_path: "demo::Point".into(),
            fields: vec![FieldDef {
                name: "raw_value".into(),
                ty: TypeRef::Primitive(PrimitiveType::I32),
                ..Default::default()
            }],
            is_clone: true,
            has_serde: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn resolve(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn zig_config(rename: bool) -> ResolvedCrateConfig {
    let rename_line = if rename {
        "\n[crates.zig]\nrename_fields = { \"Point.raw_value\" = \"value\" }\n"
    } else {
        ""
    };
    resolve(&format!(
        "[workspace]\nlanguages = [\"zig\"]\n\n[[crates]]\nname = \"demo\"\nsources = [\"src/lib.rs\"]\n{rename_line}"
    ))
}

fn dart_config(rename: bool) -> ResolvedCrateConfig {
    let rename_line = if rename {
        "rename_fields = { \"Point.raw_value\" = \"value\" }\n"
    } else {
        ""
    };
    resolve(&format!(
        "[workspace]\nlanguages = [\"dart\"]\n\n[[crates]]\nname = \"demo\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.dart]\nstyle = \"ffi\"\n{rename_line}"
    ))
}

fn joined(files: &[alef::core::backend::GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn zig_struct_member_honors_rename_fields() {
    let files = ZigBackend
        .generate_bindings(&point_api(), &zig_config(true))
        .expect("zig bindings must generate");
    let content = joined(&files);

    assert!(
        content.contains("    value: "),
        "the configured rename must reach the Zig struct member; got:\n{content}"
    );
    assert!(
        !content.contains("raw_value"),
        "the Rust field name must not survive a configured rename; got:\n{content}"
    );
}

#[test]
fn zig_struct_member_keeps_the_rust_name_without_a_rename() {
    let files = ZigBackend
        .generate_bindings(&point_api(), &zig_config(false))
        .expect("zig bindings must generate");
    let content = joined(&files);

    assert!(
        content.contains("    raw_value: "),
        "an unconfigured field must keep its Rust name; got:\n{content}"
    );
}

#[test]
fn dart_ffi_freezed_parameter_honors_rename_fields() {
    let files = DartBackend
        .generate_bindings(&point_api(), &dart_config(true))
        .expect("dart bindings must generate");
    let content = joined(&files);

    assert!(
        content.contains(" value,"),
        "the configured rename must reach the Dart parameter; got:\n{content}"
    );
    assert!(
        !content.contains("rawValue"),
        "the camelCased Rust field name must not survive a configured rename; got:\n{content}"
    );
}

#[test]
fn dart_ffi_freezed_parameter_camel_cases_the_rust_name_without_a_rename() {
    let files = DartBackend
        .generate_bindings(&point_api(), &dart_config(false))
        .expect("dart bindings must generate");
    let content = joined(&files);

    assert!(
        content.contains(" rawValue,"),
        "an unconfigured field must keep its camelCased Rust name; got:\n{content}"
    );
}
