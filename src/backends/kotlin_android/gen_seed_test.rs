//! Seed unit test for the AAR module's JVM `test` source set.
//!
//! `gen_build_gradle` has always wired `testImplementation("junit:junit:…")` and a
//! `tasks.withType<Test>` block, while no emitter ever produced a single test file — so
//! `gradle test` ran a suite of zero tests and reported success by construction. This module
//! seeds exactly one real test so that wiring has something to execute from day one.
//!
//! **How create-once is enforced here differs from the zig/dart/swift/ruby seeds.** Those are
//! scaffold files, and `write_scaffold_files_report` skips any `generated_header: false` path
//! that already exists. This backend has no scaffold arm at all
//! (`scaffold::scaffold_language` returns an empty vec for `Language::KotlinAndroid`), so the
//! seed rides the *bindings* writer, `crate::cli::pipeline::generate::write::write_files_report`,
//! which by design has no create-only concept. What protects the file there is that writer's
//! ownership guard: `.kt` is a markable extension, this seed deliberately carries no alef
//! marker, and a markable file with no marker is never eligible for the cache-backed ownership
//! fallback — so once the file exists, every later run that would change it refuses and logs a
//! warning instead. The file is therefore never clobbered, but unlike a scaffold seed it is
//! *loud* about being left alone rather than silent. Emitting it with a marker (or from a
//! writer with no such guard) would silently replace whatever suite it has grown into, so the
//! absence of that marker is load-bearing, not an oversight — `seed_content_carries_no_alef_marker`
//! is the test that pins it. ~keep

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::backends::kotlin::to_lower_camel_unescaped;
use crate::backends::kotlin_android::gen_bindings::effective_excluded_type_names;
use crate::backends::kotlin_android::naming::{kotlin_package, package_path};
use crate::codegen::naming::{kotlin_android_wrapper_object_name, wire_variant_value};
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, PrimitiveType, TypeDef, TypeRef};
use crate::core::jni::bridge_class_name;

/// Literal the seed passes for a `String` field, and looks for in the serialized payload.
const SEED_STRING_LITERAL: &str = "alef-scaffold";

/// Emit the seed test file for the AAR module.
///
/// `package_root` is the Gradle project root (the directory holding `build.gradle.kts`), so
/// the file lands on `src/test/kotlin/<dotted_package_as_path>/` — the JVM unit-test source
/// set the Kotlin Android plugin compiles, *not* `src/androidTest/` (that needs a device or
/// emulator, and nothing in CI would ever run it).
pub(super) fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, package_root: &Path) -> GeneratedFile {
    let class_name = format!("{}ScaffoldTest", kotlin_android_wrapper_object_name(&config.name));
    let path: PathBuf = package_root
        .join("src/test/kotlin")
        .join(package_path(config))
        .join(format!("{class_name}.kt"));
    GeneratedFile {
        path,
        content: render(api, config, &class_name),
        generated_header: false,
    }
}

/// Build the seed's Kotlin source.
///
/// The file is written once and then left alone (see the module doc for exactly which guard
/// enforces that here), so this content only ever reaches a fresh path — it never overwrites
/// a suite someone has since written.
///
/// The seed must not be vacuous: `assertEquals(1, 1)` passes no matter what alef generated,
/// which is strictly worse than the empty lane it replaces because it manufactures confidence.
/// Every tier below therefore asserts against the *real*, currently-generated API surface, and
/// every tier fails to compile if the type it names stops being emitted.
///
/// Tiers, strongest first:
///
/// 1. A visible DTO whose every binding-visible field is a plain primitive/`String` is
///    literal-constructed and round-tripped through the Jackson mapping the JNI bridge itself
///    marshals values with. Proves: the data class compiles with that constructor arity and
///    those parameter types, Jackson can serialize it, the serialized payload carries the
///    value that was passed in (checked against the literal, so a constructor that dropped it
///    fails), and deserializing rebuilds an equal value. Does not prove: anything native.
/// 2. Otherwise a visible unit-only enum's `toWire()` is asserted to return the exact serde
///    discriminator. Proves: the enum, that entry, and its `@JsonValue` wire mapping are
///    generated as the Rust `#[serde(rename…)]` attributes specify — a falsifiable claim about
///    the wire contract, since a changed `rename_all` strategy fails here. Does not prove:
///    anything native, nor anything about the rest of the enum.
/// 3. Otherwise a visible type or enum is asserted to exist under the module's declared
///    package. Proves: the class is emitted and lands in the package the AAR namespace
///    declares. Does not prove: its shape, or that anything can be constructed or called.
/// 4. Only when no visible type or enum exists at all (scaffolding before any Rust code) does
///    this fall back to the always-emitted `<Crate>BridgeException`. Proves: the emitted Kotlin
///    compiles and that type carries a message through its generated constructor. Does not
///    prove: any generated API exists, because at this point none does.
///
/// **What no tier proves, deliberately: that the native library loads.** Only the generated
/// `Bridge` object's `init { System.loadLibrary(…) }` would show that, and touching it makes
/// the seed fail with `UnsatisfiedLinkError` on every configuration that legitimately has no
/// host JNI binary — a `-Palef.skipHostJni=true` build, or a repo whose `<crate>-jni` crate is
/// not built. A seed that is red for a reason unrelated to what it claims to check gets
/// deleted, and a deleted seed is the empty lane again. The JSON shape asserted by tier 1 *is*
/// the payload format the bridge marshals over, so it covers the contract the boundary
/// depends on without depending on the boundary being present. ~keep
fn render(api: &ApiSurface, config: &ResolvedCrateConfig, class_name: &str) -> String {
    let package = kotlin_package(config);
    let excluded = effective_excluded_type_names(config);

    let round_trip_candidate = api
        .types
        .iter()
        .filter(|ty| type_is_visible(ty, &excluded))
        .find_map(|ty| round_trip_literals(ty).map(|literals| (ty, literals)));
    if let Some((ty, literals)) = round_trip_candidate {
        // `assertTrue` is imported only when the payload witness is actually emitted: ktlint's
        // `no-unused-imports` rule fails a build on an import nothing references. ~keep
        let mut imports = vec![
            "com.fasterxml.jackson.module.kotlin.jacksonObjectMapper",
            "org.junit.Assert.assertEquals",
        ];
        if has_payload_witness(&literals) {
            imports.push("org.junit.Assert.assertTrue");
        }
        imports.push("org.junit.Test");
        return kotlin_file(
            &package,
            class_name,
            &imports,
            &json_round_trip_case(&ty.name, &literals),
        );
    }

    let wire_candidate = api
        .enums
        .iter()
        .filter(|en| enum_is_visible(en, &excluded, config))
        .find_map(|en| unit_enum_wire_case(en).map(|(entry, wire)| (en, entry, wire)));
    if let Some((en, entry, wire)) = wire_candidate {
        return kotlin_file(
            &package,
            class_name,
            &["org.junit.Assert.assertEquals", "org.junit.Test"],
            &enum_wire_case(&en.name, &entry, &wire),
        );
    }

    let referenceable = api
        .types
        .iter()
        .filter(|ty| type_is_visible(ty, &excluded))
        .map(|ty| ty.name.clone())
        .next()
        .or_else(|| {
            api.enums
                .iter()
                .filter(|en| enum_is_visible(en, &excluded, config))
                .map(|en| en.name.clone())
                .next()
        });
    if let Some(name) = referenceable {
        return kotlin_file(
            &package,
            class_name,
            &["org.junit.Assert.assertEquals", "org.junit.Test"],
            &type_reference_case(&package, &name),
        );
    }

    kotlin_file(
        &package,
        class_name,
        &["org.junit.Assert.assertEquals", "org.junit.Test"],
        &exception_case(&format!("{}Exception", bridge_class_name(&config.name))),
    )
}

/// A type the seed may name, mirroring exactly the skip conditions in
/// [`super::gen_bindings::emit`]'s type loop — anything it skips is never emitted, so a seed
/// naming one would not compile.
fn type_is_visible(ty: &TypeDef, excluded: &HashSet<String>) -> bool {
    !ty.is_opaque && !ty.is_trait && !ty.binding_excluded && !excluded.contains(&ty.name)
}

/// An enum the seed may name. [`super::gen_bindings::emit`]'s enum loop only checks
/// `binding_excluded` because the exclusion sets were already applied to the surface by
/// `KotlinAndroidBackend::generate_bindings`; re-checking them here costs nothing and only
/// ever narrows the choice. Enums listed in `untagged_union_text_types` are skipped because
/// that config re-shapes their emitted form.
fn enum_is_visible(en: &EnumDef, excluded: &HashSet<String>, config: &ResolvedCrateConfig) -> bool {
    !en.binding_excluded && !excluded.contains(&en.name) && !config.untagged_union_text_types.contains(&en.name)
}

/// Literal constructor arguments for `ty`, in declaration order, or `None` when any
/// binding-visible field falls outside the safely synthesizable subset.
///
/// Arguments are positional rather than named on purpose: the emitted parameter name is
/// `kotlin_field_name` (lower-camel plus backtick escaping for Kotlin keywords), which is
/// private to the Kotlin backend and not re-exported, so recomputing it here would be a
/// guess that silently stops matching. Declaration order is not a guess — it is the field
/// order, which this function walks in full. Bailing on the whole type rather than skipping
/// an unsupported field is what keeps that alignment true. ~keep
fn round_trip_literals(ty: &TypeDef) -> Option<Vec<String>> {
    let mut literals = Vec::new();
    for field in binding_fields(&ty.fields) {
        if field.optional || field.serde_flatten {
            return None;
        }
        let literal = match &field.ty {
            TypeRef::Primitive(primitive) => kotlin_primitive_literal(primitive).to_string(),
            TypeRef::String => format!("\"{SEED_STRING_LITERAL}\""),
            _ => return None,
        };
        literals.push(literal);
    }
    if literals.is_empty() { None } else { Some(literals) }
}

/// A literal Kotlin value for a primitive field. `Boolean` gets a non-default `true` and the
/// floats a non-integral `1.5`, so a constructor that silently drops the argument and falls
/// back to the field's generated default is still caught. The suffixes are load-bearing:
/// `Byte`/`Short`/`Int` accept a bare integer literal by expected-type inference, `Long`
/// needs `1L`, and `Float` needs `1.5f`.
fn kotlin_primitive_literal(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::Bool => "true",
        PrimitiveType::F32 => "1.5f",
        PrimitiveType::F64 => "1.5",
        PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "1L",
        _ => "1",
    }
}

/// The generated entry name and wire discriminator of `en`'s first variant, or `None` when the
/// enum is not the unit-only shape that gets an `enum class` with a `toWire()`.
fn unit_enum_wire_case(en: &EnumDef) -> Option<(String, String)> {
    if en.variants.iter().any(|variant| !variant.fields.is_empty()) {
        return None;
    }
    let variant = en.variants.iter().find(|variant| !variant.binding_excluded)?;
    Some((
        to_screaming_snake(&variant.name),
        wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        ),
    ))
}

/// `PascalCase` → `SCREAMING_SNAKE_CASE`, byte-for-byte the transform the Kotlin enum emitter
/// applies to variant names (`backends::kotlin::gen_bindings::shared::to_screaming_snake`).
/// Duplicated rather than imported because that module is private to the Kotlin backend and
/// re-exports only `to_lower_camel`/`to_pascal_case`; a seed naming an entry the emitter did
/// not emit would not compile, so the two must agree. ~keep
fn to_screaming_snake(name: &str) -> String {
    let mut out = String::new();
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && index > 0 {
            out.push('_');
        }
        out.extend(ch.to_uppercase());
    }
    out
}

/// Assemble the file: package declaration, alphabetically sorted imports, one test class.
///
/// No `Generated by alef` / `auto-generated by alef` marker appears anywhere, and must not:
/// both writers read that marker as proof of alef authorship, and here it is the only thing
/// standing between a later run and whatever suite this file has grown into. ~keep
fn kotlin_file(package: &str, class_name: &str, imports: &[&str], case: &str) -> String {
    let import_block = imports
        .iter()
        .map(|import| format!("import {import}\n"))
        .collect::<String>();
    format!(
        "package {package}\n\n{import_block}\nclass {class_name} {{\n{case}}}\n",
        package = package,
        import_block = import_block,
        class_name = class_name,
        case = case,
    )
}

/// Whether the constructed value carries a literal the seed can look for in the serialized
/// payload. Only the `String` literal qualifies: a numeric or boolean literal would match
/// incidental characters elsewhere in the JSON (a wire name, another field's value), so
/// asserting on one would be a check that cannot fail rather than one that can.
fn has_payload_witness(literals: &[String]) -> bool {
    literals.iter().any(|literal| literal.contains(SEED_STRING_LITERAL))
}

fn json_round_trip_case(type_name: &str, literals: &[String]) -> String {
    let arguments = literals.join(", ");
    let witness = if has_payload_witness(literals) {
        format!(
            "        assertTrue(\n            \"serialized payload lost the constructed value: $json\",\n            \
             json.contains(\"{SEED_STRING_LITERAL}\"),\n        )\n"
        )
    } else {
        String::new()
    };
    format!(
        r#"    // Round-trips the generated `{type_name}` data class through the Jackson mapping the
    // JNI bridge marshals values with: it fails to compile if the generated constructor
    // loses a parameter or changes a type, and fails at runtime if the class stops being
    // serializable or stops rebuilding an equal value. It proves nothing about the native
    // library -- no tier here loads it, deliberately; see the note on the emitter. Seeded
    // once and never regenerated over, so replace it with a real suite. ~keep
    @Test
    fun {test_name}RoundTripsThroughItsGeneratedJsonMapping() {{
        val original = {type_name}({arguments})
        val mapper = jacksonObjectMapper()
        val json = mapper.writeValueAsString(original)
{witness}        assertEquals(original, mapper.readValue(json, {type_name}::class.java))
    }}
"#,
        type_name = type_name,
        test_name = to_lower_camel_unescaped(type_name),
        arguments = arguments,
        witness = witness,
    )
}

fn enum_wire_case(enum_name: &str, entry: &str, wire: &str) -> String {
    format!(
        r#"    // No generated data class is literal-constructible by a seed that cannot synthesize
    // values for its fields, so this asserts the generated `{enum_name}` wire mapping
    // instead: `toWire()` must return exactly the serde discriminator the Rust attributes
    // declare, so a changed `rename_all` strategy or a renamed variant fails here. It says
    // nothing about the rest of the enum, and nothing about the native library. Seeded once
    // and never regenerated over, so replace it with a real suite. ~keep
    @Test
    fun {test_name}ExposesItsGeneratedWireValue() {{
        assertEquals("{wire}", {enum_name}.{entry}.toWire())
    }}
"#,
        enum_name = enum_name,
        entry = entry,
        wire = wire,
        test_name = to_lower_camel_unescaped(enum_name),
    )
}

fn type_reference_case(package: &str, name: &str) -> String {
    format!(
        r#"    // `{name}` is neither literal-constructible nor a unit-only enum, so this only asserts
    // that it is emitted under the package the AAR namespace declares -- a real fact about
    // the generated output (a changed package or output layout fails here), but one that
    // proves nothing about its shape and calls nothing on it. Seeded once and never
    // regenerated over, so replace it with a real suite. ~keep
    @Test
    fun {test_name}IsGeneratedIntoTheModulePackage() {{
        assertEquals("{package}.{name}", {name}::class.java.name)
    }}
"#,
        package = package,
        name = name,
        test_name = to_lower_camel_unescaped(name),
    )
}

fn exception_case(exception_name: &str) -> String {
    format!(
        r#"    // No generated API surface exists yet for this crate, so there is nothing to assert
    // against beyond the one type alef always emits. This proves the generated Kotlin
    // compiles and that `{exception_name}` carries a message through its generated
    // constructor; it proves no generated API exists, because at this point none does, and
    // it proves nothing about the native library. Seeded once and never regenerated over,
    // so replace it with a real suite. ~keep
    @Test
    fun theGeneratedExceptionTypeCarriesItsMessage() {{
        assertEquals("scaffold", {exception_name}("scaffold").message)
    }}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::{EnumVariant, FieldDef};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "my-lib"
sources = []

[crates.kotlin_android]
package = "dev.example"
"#,
        )
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..Default::default()
        }
    }

    fn dto(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields,
            ..Default::default()
        }
    }

    fn unit_enum(name: &str, variants: &[&str]) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            variants: variants
                .iter()
                .map(|variant| EnumVariant {
                    name: (*variant).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The strongest tier: a literal-constructible DTO is actually built and round-tripped
    /// through the Jackson mapping, with the constructed value checked in the payload.
    #[test]
    fn round_trips_a_simple_dto_through_jackson() {
        let api = ApiSurface {
            types: vec![dto(
                "Widget",
                vec![
                    field("label", TypeRef::String),
                    field("count", TypeRef::Primitive(PrimitiveType::U32)),
                ],
            )],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(out.starts_with("package dev.example\n"), "got:\n{out}");
        assert!(
            out.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper\n"),
            "got:\n{out}"
        );
        assert!(
            out.contains("        val original = Widget(\"alef-scaffold\", 1)\n"),
            "got:\n{out}"
        );
        assert!(
            out.contains("            json.contains(\"alef-scaffold\"),\n"),
            "got:\n{out}"
        );
        assert!(
            out.contains("        assertEquals(original, mapper.readValue(json, Widget::class.java))\n"),
            "got:\n{out}"
        );
    }

    /// Each primitive needs the literal suffix its Kotlin type demands; a bare `1` for a
    /// `Long` parameter or a bare `1.5` for a `Float` one does not compile.
    #[test]
    fn emits_a_type_correct_literal_for_each_primitive() {
        let cases = [
            (PrimitiveType::Bool, "true"),
            (PrimitiveType::I32, "1"),
            (PrimitiveType::U64, "1L"),
            (PrimitiveType::Usize, "1L"),
            (PrimitiveType::F32, "1.5f"),
            (PrimitiveType::F64, "1.5"),
        ];
        for (primitive, expected) in cases {
            let api = ApiSurface {
                types: vec![dto("Widget", vec![field("value", TypeRef::Primitive(primitive))])],
                ..Default::default()
            };
            let out = render(&api, &minimal_config(), "MyLibScaffoldTest");
            assert!(
                out.contains(&format!("val original = Widget({expected})\n")),
                "expected literal `{expected}`, got:\n{out}"
            );
        }
    }

    /// A DTO with no `String` field still round-trips, just without the payload witness --
    /// the seed must not assert a literal it did not pass in.
    #[test]
    fn omits_the_payload_witness_when_no_string_field_exists() {
        let api = ApiSurface {
            types: vec![dto(
                "Widget",
                vec![field("count", TypeRef::Primitive(PrimitiveType::U32))],
            )],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(!out.contains("alef-scaffold"), "got:\n{out}");
        assert!(!out.contains("assertTrue"), "got:\n{out}");
        assert!(
            out.contains("        assertEquals(original, mapper.readValue(json, Widget::class.java))\n"),
            "got:\n{out}"
        );
    }

    /// Constructor arguments are positional, so a field the seed cannot synthesize must
    /// disqualify the whole type -- skipping it would misalign every later argument.
    #[test]
    fn falls_back_when_any_field_is_not_synthesizable() {
        let api = ApiSurface {
            types: vec![dto(
                "Widget",
                vec![
                    field("label", TypeRef::String),
                    field("nested", TypeRef::Named("Other".to_string())),
                ],
            )],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(!out.contains("val original ="), "got:\n{out}");
        assert!(
            out.contains("        assertEquals(\"dev.example.Widget\", Widget::class.java.name)\n"),
            "got:\n{out}"
        );
    }

    /// An optional field is emitted as a nullable parameter with a `null` default; the seed
    /// does not model those, so it degrades rather than guessing.
    #[test]
    fn falls_back_when_a_field_is_optional() {
        let api = ApiSurface {
            types: vec![dto(
                "Widget",
                vec![FieldDef {
                    optional: true,
                    ..field("label", TypeRef::String)
                }],
            )],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(!out.contains("val original ="), "got:\n{out}");
        assert!(out.contains("Widget::class.java.name"), "got:\n{out}");
    }

    /// With no constructible DTO, a unit-only enum's wire discriminator is asserted -- and it
    /// must be the serde value, not the Kotlin entry name.
    #[test]
    fn asserts_the_serde_wire_value_of_a_unit_enum() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                serde_rename_all: Some("snake_case".to_string()),
                ..unit_enum("Colour", &["DarkRed", "Blue"])
            }],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(
            out.contains("        assertEquals(\"dark_red\", Colour.DARK_RED.toWire())\n"),
            "got:\n{out}"
        );
    }

    /// A per-variant `#[serde(rename)]` wins over the enum-level strategy, exactly as the
    /// enum emitter resolves it.
    #[test]
    fn honours_a_per_variant_serde_rename() {
        let mut colour = unit_enum("Colour", &["DarkRed"]);
        colour.variants[0].serde_rename = Some("crimson".to_string());
        let api = ApiSurface {
            enums: vec![colour],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(
            out.contains("        assertEquals(\"crimson\", Colour.DARK_RED.toWire())\n"),
            "got:\n{out}"
        );
    }

    /// An enum with data-carrying variants is emitted as a sealed class with no `toWire()`,
    /// so the wire tier must not fire for it.
    #[test]
    fn skips_enums_with_data_carrying_variants() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                variants: vec![
                    EnumVariant {
                        name: "Plain".to_string(),
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Rich".to_string(),
                        fields: vec![field("value", TypeRef::String)],
                        ..Default::default()
                    },
                ],
                ..unit_enum("Payload", &[])
            }],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        assert!(!out.contains("toWire()"), "got:\n{out}");
        assert!(
            out.contains("        assertEquals(\"dev.example.Payload\", Payload::class.java.name)\n"),
            "got:\n{out}"
        );
    }

    /// `[crates.kotlin_android] exclude_types` removes the class from the emitted source set,
    /// so a seed naming it would not compile.
    #[test]
    fn skips_types_excluded_by_kotlin_android_config() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "my-lib"
sources = []

[crates.kotlin_android]
package = "dev.example"
exclude_types = ["Excluded"]
"#,
        );
        let api = ApiSurface {
            types: vec![
                dto("Excluded", vec![field("label", TypeRef::String)]),
                dto("Visible", vec![field("label", TypeRef::String)]),
            ],
            ..Default::default()
        };
        let out = render(&api, &config, "MyLibScaffoldTest");

        assert!(!out.contains("Excluded"), "got:\n{out}");
        assert!(
            out.contains("val original = Visible(\"alef-scaffold\")\n"),
            "got:\n{out}"
        );
    }

    /// Opaque, trait and `binding_excluded` types are all skipped by the emitter's own type
    /// loop, so none of them may be named here either.
    #[test]
    fn skips_opaque_trait_and_binding_excluded_types() {
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    is_opaque: true,
                    ..dto("Handle", vec![field("label", TypeRef::String)])
                },
                TypeDef {
                    is_trait: true,
                    ..dto("Backend", vec![field("label", TypeRef::String)])
                },
                TypeDef {
                    binding_excluded: true,
                    ..dto("Hidden", vec![field("label", TypeRef::String)])
                },
            ],
            ..Default::default()
        };
        let out = render(&api, &minimal_config(), "MyLibScaffoldTest");

        for skipped in ["Handle", "Backend", "Hidden"] {
            assert!(!out.contains(skipped), "`{skipped}` must not be named, got:\n{out}");
        }
        assert!(out.contains("MyLibBridgeException(\"scaffold\")"), "got:\n{out}");
    }

    /// An empty API surface still gets a falsifiable example against the one type alef always
    /// emits for this backend.
    #[test]
    fn falls_back_to_the_generated_exception_type() {
        let out = render(&ApiSurface::default(), &minimal_config(), "MyLibScaffoldTest");

        assert!(
            out.contains("        assertEquals(\"scaffold\", MyLibBridgeException(\"scaffold\").message)\n"),
            "got:\n{out}"
        );
    }

    /// No tier may emit a tautology, and every tier must name a generated symbol -- that is
    /// what makes even the weakest one falsifiable rather than decorative.
    #[test]
    fn no_tier_emits_a_vacuous_example() {
        let surfaces = [
            ApiSurface {
                types: vec![dto("Widget", vec![field("label", TypeRef::String)])],
                ..Default::default()
            },
            ApiSurface {
                enums: vec![unit_enum("Colour", &["Blue"])],
                ..Default::default()
            },
            ApiSurface {
                types: vec![dto(
                    "Widget",
                    vec![field("nested", TypeRef::Named("Other".to_string()))],
                )],
                ..Default::default()
            },
            ApiSurface::default(),
        ];
        for api in surfaces {
            let out = render(&api, &minimal_config(), "MyLibScaffoldTest");
            assert_eq!(out.matches("    @Test\n").count(), 1, "exactly one test, got:\n{out}");
            assert!(out.contains("import org.junit.Test\n"), "got:\n{out}");
            for tautology in ["assertEquals(1, 1)", "assertTrue(true)", "assertNotNull(null)"] {
                assert!(!out.contains(tautology), "vacuous assertion `{tautology}` in:\n{out}");
            }
        }
    }

    /// The seed must never look alef-authored: `write_scaffold_files_report` reads the header
    /// marker as ownership and would let an overwrite run replace a hand-written suite.
    #[test]
    fn seed_content_carries_no_alef_marker() {
        let out = render(&ApiSurface::default(), &minimal_config(), "MyLibScaffoldTest");

        assert!(
            !crate::core::hash::content_has_alef_marker(&out),
            "seed must stay unmarked so it is never reclaimed by an overwrite run, got:\n{out}"
        );
    }

    /// The seed lands on the JVM unit-test source set Gradle already compiles and runs, and
    /// is create-only so a real suite is never overwritten.
    #[test]
    fn seed_is_emitted_create_only_onto_the_jvm_test_source_set() {
        let file = emit(
            &ApiSurface::default(),
            &minimal_config(),
            Path::new("packages/kotlin-android"),
        );

        assert_eq!(
            file.path,
            PathBuf::from("packages/kotlin-android/src/test/kotlin/dev/example/MyLibScaffoldTest.kt")
        );
        assert!(!file.generated_header, "the seed must stay create-only");
    }
}
