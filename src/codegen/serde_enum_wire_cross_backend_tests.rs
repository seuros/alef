//! Cross-backend guard: the JSON shape of an adjacently tagged Rust enum
//! (`#[serde(tag = "...", content = "...")]`) is decided by serde, not by each backend's own
//! guess at a tagging scheme.
//!
//! Nothing here is a hand-written expected string. [`SampleOutcome`] is a real Rust enum carrying
//! the same serde attributes as the IR fixture, so `serde_json` produces the wire form every
//! backend is measured against — the only authority there is. Swift's trait-bridge result encoder
//! used to emit serde's *external* default (`"Proceed"`) for an adjacently tagged enum and Rust
//! rejected every callback with `invalid type: string "...", expected adjacently tagged enum ...`;
//! this test rejects that at generation time instead.
//!
//! [`ADJACENT_TAGGING_SUPPORT`] is the load-bearing part. Every backend that hand-writes JSON for
//! this enum is listed there, and backends still carrying the same divergence Swift had are listed
//! as [`AdjacentSupport::KnownDivergent`] with their divergence asserted rather than waived —
//! fixing one turns this test red until it is moved to [`AdjacentSupport::Correct`], where the
//! exact-shape assertions take over. [`BACKENDS_THAT_SEE_THE_TAG_KEY`] closes the other side: a
//! backend that newly starts consuming the serde tag forces a decision instead of slipping past.

use crate::backends::csharp::CsharpBackend;
use crate::backends::dart::DartBackend;
use crate::backends::extendr::ExtendrBackend;
use crate::backends::ffi::FfiBackend;
use crate::backends::gleam::GleamBackend;
use crate::backends::go::GoBackend;
use crate::backends::java::JavaBackend;
use crate::backends::jni::JniBackend;
use crate::backends::kotlin::KotlinBackend;
use crate::backends::kotlin_android::KotlinAndroidBackend;
use crate::backends::magnus::MagnusBackend;
use crate::backends::napi::NapiBackend;
use crate::backends::php::PhpBackend;
use crate::backends::pyo3::Pyo3Backend;
use crate::backends::rustler::RustlerBackend;
use crate::backends::swift::SwiftBackend;
use crate::backends::wasm::WasmBackend;
use crate::backends::zig::ZigBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, TypeDef, TypeRef};

/// The ground truth. Its serde attributes are mirrored field-for-field by [`fixture_enum`], so
/// `serde_json` answers what every backend must emit.
#[derive(serde::Serialize)]
#[serde(tag = "wire_tag", content = "wire_content", rename_all = "snake_case")]
enum SampleOutcome {
    Proceed,
    Replace(String),
    SkipSubtree,
}

const ENUM_NAME: &str = "SampleOutcome";
/// Deliberately unusual key names: they must appear in generated output only because a backend
/// read them out of the IR, never because they happen to be common words in prose or in another
/// generated construct.
const TAG_KEY: &str = "wire_tag";
const CONTENT_KEY: &str = "wire_content";
const SAMPLE_PAYLOAD: &str = "X";
const TRAIT_NAME: &str = "SampleVisitor";
const OPTIONS_TYPE: &str = "SampleOptions";
const OPTIONS_FIELD: &str = "visitor";

fn serde_json_of(value: &SampleOutcome) -> String {
    serde_json::to_string(value).expect("fixture enum serializes")
}

/// Every wire form serde produces for the fixture, in variant order.
fn serde_wire_forms() -> Vec<String> {
    vec![
        serde_json_of(&SampleOutcome::Proceed),
        serde_json_of(&SampleOutcome::Replace(SAMPLE_PAYLOAD.to_string())),
        serde_json_of(&SampleOutcome::SkipSubtree),
    ]
}

/// The variant tag serde writes under [`TAG_KEY`], read back out of serde's own output rather
/// than restated.
fn serde_variant_tags() -> Vec<String> {
    serde_wire_forms()
        .iter()
        .map(|json| {
            let value: serde_json::Value = serde_json::from_str(json).expect("serde output is JSON");
            value[TAG_KEY].as_str().expect("tag key holds a string").to_string()
        })
        .collect()
}

/// The IR mirror of [`SampleOutcome`].
fn fixture_enum() -> EnumDef {
    EnumDef {
        name: ENUM_NAME.to_string(),
        rust_path: format!("sample_core::{ENUM_NAME}"),
        has_serde: true,
        serde_tag: Some(TAG_KEY.to_string()),
        serde_content: Some(CONTENT_KEY.to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Proceed".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Replace".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "SkipSubtree".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// The enum on its own — the shape every backend's plain DTO/enum emitter sees.
fn plain_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        enums: vec![fixture_enum()],
        ..ApiSurface::default()
    }
}

/// The enum as a trait bridge's result type, which is what drives Swift's inbound-protocol
/// result encoder (`sample_outcome_toJson`) — the site that emitted the wrong tagging.
fn bridged_api() -> ApiSurface {
    let trait_def = TypeDef {
        name: TRAIT_NAME.to_string(),
        rust_path: format!("sample_core::{TRAIT_NAME}"),
        is_trait: true,
        methods: vec![MethodDef {
            name: "visit".to_string(),
            return_type: TypeRef::Named(ENUM_NAME.to_string()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let options = TypeDef {
        name: OPTIONS_TYPE.to_string(),
        rust_path: format!("sample_core::{OPTIONS_TYPE}"),
        has_serde: true,
        fields: vec![FieldDef {
            name: OPTIONS_FIELD.to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named(TRAIT_NAME.to_string()))),
            optional: true,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![trait_def, options],
        enums: vec![fixture_enum()],
        ..ApiSurface::default()
    }
}

fn bridged_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: TRAIT_NAME.to_string(),
            type_alias: Some("SampleVisitorHandle".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some(OPTIONS_TYPE.to_string()),
            options_field: Some(OPTIONS_FIELD.to_string()),
            result_type: Some(ENUM_NAME.to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn joined_content(files: &[GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
}

fn generate(backend: &dyn Backend, api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_bindings(api, config)
        .map(|files| joined_content(&files))
        .unwrap_or_default()
}

fn swift_bridged_output() -> String {
    generate(&SwiftBackend, &bridged_api(), &bridged_config())
}

fn kotlin_android_union_output() -> String {
    generate(&KotlinAndroidBackend, &plain_api(), &ResolvedCrateConfig::default())
}

/// Isolate one generated Jackson codec class, so an assertion cannot be satisfied by a matching
/// literal in its sibling or elsewhere in the module.
fn kotlin_codec_body(kotlin: &str, codec: &str) -> String {
    let marker = format!("private class {ENUM_NAME}{codec} :");
    let start = kotlin
        .find(&marker)
        .unwrap_or_else(|| panic!("Kotlin-Android emits `{marker}`:\n{kotlin}"));
    let rest = &kotlin[start..];
    let end = rest.find("\n}\n").expect("the codec class is closed");
    rest[..end].to_string()
}

/// The JSON the generated Kotlin serializer writes for each variant, reconstructed from the
/// emitted code: each `when` arm builds one object node, and whether it also sets the content key
/// decides whether the payload reaches the wire.
fn kotlin_emitted_wire_forms(kotlin: &str) -> Vec<String> {
    let body = kotlin_codec_body(kotlin, "Serializer");
    assert!(
        !body.contains("as com.fasterxml.jackson.databind.node.ObjectNode"),
        "the payload must not be cast to ObjectNode — a newtype variant's `String` payload is a \
         TextNode, and the cast throws at runtime instead of putting it on the wire:\n{body}"
    );
    let payload_json = serde_json::to_string(SAMPLE_PAYLOAD).expect("the string payload serializes");
    body.split(&format!("n.put(\"{TAG_KEY}\", \""))
        .skip(1)
        .map(|chunk| {
            let (discriminator, rest) = chunk.split_once('"').expect("each tag put is a closed literal");
            if rest.contains(&format!("(\"{CONTENT_KEY}\",")) {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\",\"{CONTENT_KEY}\":{payload_json}}}")
            } else {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\"}}")
            }
        })
        .collect()
}

#[test]
fn kotlin_android_serializer_emits_exactly_what_serde_writes() {
    let emitted = kotlin_emitted_wire_forms(&kotlin_android_union_output());
    assert_eq!(
        emitted,
        serde_wire_forms(),
        "the generated Jackson serializer must produce serde's adjacent wire form for every \
         variant; injecting the tag into the payload's own object is serde's *internal* form, \
         which Rust rejects, and it cannot represent a scalar payload at all"
    );
}

#[test]
fn kotlin_android_deserializer_reads_the_payload_from_serdes_content_key() {
    let body = kotlin_codec_body(&kotlin_android_union_output(), "Deserializer");
    assert!(
        body.contains(&format!("node.get(\"{CONTENT_KEY}\")")),
        "the deserializer must read the payload from serde's content key:\n{body}"
    );
    assert!(
        !body.contains(&format!("remove(\"{TAG_KEY}\")")),
        "stripping the tag and treating the remainder as the payload is serde's *internal* form; \
         an adjacently tagged document keeps its payload under the content key:\n{body}"
    );
    for tag in serde_variant_tags() {
        assert!(
            body.contains(&format!("\"{tag}\" ->")),
            "the deserializer must dispatch on serde's variant tag {tag:?}:\n{body}"
        );
    }
}

/// Whether a backend already speaks serde's adjacent form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]

enum AdjacentSupport {
    /// Emits both the tag key and the content key, i.e. serde's adjacent shape.
    Correct,
    /// Emits the tag key but no content key — serde's *internal* shape, which Rust refuses for an
    /// adjacently tagged enum. The string records where the divergence lives.
    KnownDivergent(&'static str),
}

/// Every backend that hand-writes a JSON document for this enum, and what shape it writes.
///
/// Backends absent from this list do not hand-write JSON for it: they build a native host value
/// (a NIF term, an R object, a JS object) and cross the boundary through the generated Rust
/// `From` conversion, or they declare a Rust mirror type carrying the serde attributes verbatim
/// and let serde itself do the encoding. Neither can disagree with serde about tagging.
/// [`BACKENDS_THAT_SEE_THE_TAG_KEY`] is what keeps that claim honest as backends change.
const ADJACENT_TAGGING_SUPPORT: &[(&str, AdjacentSupport)] = &[
    ("swift", AdjacentSupport::Correct),
    ("go", AdjacentSupport::Correct),
    ("java", AdjacentSupport::Correct),
    ("csharp", AdjacentSupport::Correct),
    ("kotlin_android", AdjacentSupport::Correct),
    (
        "pyo3",
        AdjacentSupport::KnownDivergent(
            "backends::pyo3::gen_stubs::enums builds `{tag_field: variant}` and merges the payload \
             fields into the same object",
        ),
    ),
];

/// Every backend whose generated output mentions the serde tag key at all, whether or not it
/// hand-writes JSON. Recorded so that a backend which newly starts emitting the tag key — a new
/// backend, or an existing one growing a JSON path — forces a decision about
/// [`ADJACENT_TAGGING_SUPPORT`] instead of slipping past unexamined.
const BACKENDS_THAT_SEE_THE_TAG_KEY: &[&str] = &[
    "swift",
    "go",
    "java",
    "csharp",
    "kotlin_android",
    "pyo3",
    "napi",
    "wasm",
    "php",
    "magnus",
    "rustler",
    "extendr",
];

fn all_backends() -> Vec<(&'static str, Box<dyn Backend>)> {
    vec![
        ("swift", Box::new(SwiftBackend)),
        ("go", Box::new(GoBackend)),
        ("java", Box::new(JavaBackend)),
        ("csharp", Box::new(CsharpBackend)),
        ("kotlin", Box::new(KotlinBackend)),
        ("kotlin_android", Box::new(KotlinAndroidBackend)),
        ("dart", Box::new(DartBackend)),
        ("pyo3", Box::new(Pyo3Backend)),
        ("napi", Box::new(NapiBackend)),
        ("wasm", Box::new(WasmBackend)),
        ("gleam", Box::new(GleamBackend)),
        ("php", Box::new(PhpBackend)),
        ("magnus", Box::new(MagnusBackend)),
        ("rustler", Box::new(RustlerBackend)),
        ("extendr", Box::new(ExtendrBackend)),
        ("zig", Box::new(ZigBackend)),
        ("ffi", Box::new(FfiBackend)),
        ("jni", Box::new(JniBackend)),
    ]
}

/// Everything a backend generated for the fixture, across both the plain and the trait-bridged
/// surfaces, since some backends only reach the enum through one of them.
fn backend_output(backend: &dyn Backend) -> String {
    format!(
        "{}\n{}",
        generate(backend, &plain_api(), &ResolvedCrateConfig::default()),
        generate(backend, &bridged_api(), &bridged_config())
    )
}

/// Isolate the body of the Swift trait-bridge result encoder so the assertion cannot be satisfied
/// by a matching literal somewhere else in the module.
fn swift_to_json_body(swift: &str) -> String {
    let start = swift
        .find("_toJson(_ result:")
        .expect("Swift emits the bridge result encoder");
    let rest = &swift[start..];
    let end = rest.find("\n}\n").expect("bridge result encoder is closed");
    rest[..end].to_string()
}

/// The JSON each `case` of the Swift bridge encoder returns, with the Swift string escaping and
/// the payload interpolation resolved back into plain JSON.
fn swift_emitted_wire_forms(swift: &str) -> Vec<String> {
    swift_to_json_body(swift)
        .lines()
        .filter_map(|line| line.trim().split_once(": return \""))
        .map(|(_, literal)| {
            literal
                .strip_suffix('"')
                .unwrap_or(literal)
                .replace("\\\"", "\"")
                .replace("\\(jsonEscapeStr(v))", SAMPLE_PAYLOAD)
        })
        .collect()
}

#[test]
fn swift_bridge_encoder_emits_exactly_what_serde_writes() {
    let emitted = swift_emitted_wire_forms(&swift_bridged_output());
    assert_eq!(
        emitted,
        serde_wire_forms(),
        "the Swift trait-bridge result encoder must produce serde's adjacent wire form for every \
         variant; anything else is rejected by Rust with `expected adjacently tagged enum`"
    );
}

#[test]
fn swift_codable_decoder_reads_serdes_adjacent_keys() {
    let swift = swift_bridged_output();
    assert!(
        swift.contains("public init(from decoder: Decoder) throws"),
        "an adjacently tagged enum must get a custom Codable conformance; Swift's synthesised one \
         writes its own externally tagged shape:\n{swift}"
    );
    assert!(
        swift.contains(&format!("= \"{TAG_KEY}\"")) && swift.contains(&format!("= \"{CONTENT_KEY}\"")),
        "the Codable CodingKeys must be the serde tag and content keys, not the payload's field \
         names:\n{swift}"
    );
    for tag in serde_variant_tags() {
        assert!(
            swift.contains(&format!("case \"{tag}\":")),
            "the decoder must dispatch on serde's variant tag {tag:?}:\n{swift}"
        );
    }
}

#[test]
fn go_adjacent_struct_uses_serdes_tag_and_content_keys() {
    let go = generate(&GoBackend, &plain_api(), &ResolvedCrateConfig::default());
    assert!(
        go.contains(&format!("json:\"{TAG_KEY}\"")),
        "Go's adjacent struct must tag the discriminator field with serde's tag key:\n{go}"
    );
    assert!(
        go.contains(&format!("json:\"{CONTENT_KEY}")),
        "Go's adjacent struct must tag the payload field with serde's content key:\n{go}"
    );
    for tag in serde_variant_tags() {
        assert!(
            go.contains(&format!("\"{tag}\"")),
            "Go must use serde's variant tag {tag:?}:\n{go}"
        );
    }
}

#[test]
fn the_set_of_backends_reading_the_serde_tag_is_unchanged() {
    let observed: Vec<&str> = all_backends()
        .iter()
        .filter(|(_, backend)| backend_output(backend.as_ref()).contains(TAG_KEY))
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        observed, BACKENDS_THAT_SEE_THE_TAG_KEY,
        "a backend started (or stopped) consuming an adjacently tagged enum's serde tag. If it \
         hand-writes JSON, add it to ADJACENT_TAGGING_SUPPORT so its shape is checked against \
         serde's; if it converts through generated Rust instead, only update this list."
    );
}

#[test]
fn support_table_matches_what_each_backend_actually_emits() {
    let backends = all_backends();
    for (name, support) in ADJACENT_TAGGING_SUPPORT {
        let backend = backends
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, backend)| backend.as_ref())
            .unwrap_or_else(|| panic!("ADJACENT_TAGGING_SUPPORT names an unknown backend: {name}"));
        let output = backend_output(backend);
        assert!(
            output.contains(TAG_KEY),
            "{name} is listed in ADJACENT_TAGGING_SUPPORT but never emits the serde tag key; drop \
             the entry or fix the fixture"
        );
        match support {
            AdjacentSupport::Correct => assert!(
                output.contains(CONTENT_KEY),
                "{name} is listed as Correct but never emits the serde content key `{CONTENT_KEY}`, \
                 so its payload cannot land where Rust looks for it"
            ),
            AdjacentSupport::KnownDivergent(where_) => assert!(
                !output.contains(CONTENT_KEY),
                "{name} now emits the serde content key `{CONTENT_KEY}` — the divergence recorded \
                 for it is gone. Move it to AdjacentSupport::Correct so the exact-shape assertions \
                 start guarding it. Recorded divergence: {where_}"
            ),
        }
    }
}

fn java_union_output() -> String {
    generate(&JavaBackend, &plain_api(), &ResolvedCrateConfig::default())
}

/// Isolate the body of the generated Jackson serializer so an assertion cannot be satisfied by a
/// matching literal elsewhere in the file.
fn java_serialize_body(java: &str) -> String {
    let start = java
        .find(&format!("public void serialize({ENUM_NAME} value"))
        .expect("Java emits a Jackson serializer for the sealed union");
    let rest = &java[start..];
    let end = rest.find("\n  }\n").expect("the serialize method is closed");
    rest[..end].to_string()
}

/// The JSON the generated Java serializer writes for each variant, reconstructed from the emitted
/// code rather than restated: the `instanceof` chain supplies each variant's tag and whether it
/// sets a payload, and the writer block below it supplies where that payload lands.
fn java_emitted_wire_forms(java: &str) -> Vec<String> {
    let body = java_serialize_body(java);
    assert!(
        body.contains(&format!("gen.writeStringField(\"{TAG_KEY}\", tag);")),
        "the serializer must write the variant tag under serde's tag key:\n{body}"
    );
    assert!(
        body.contains(&format!("gen.writeFieldName(\"{CONTENT_KEY}\");"))
            && body.contains("gen.writeTree(MAPPER.valueToTree(inner));"),
        "the serializer must write the payload whole under serde's content key:\n{body}"
    );
    assert!(
        !body.contains("isObject()"),
        "the payload must not be written only when it happens to be a JSON object — a newtype \
         variant hands the serializer a bare scalar, and gating on isObject() discards it \
         silently:\n{body}"
    );
    let payload_json = serde_json::to_string(SAMPLE_PAYLOAD).expect("the string payload serializes");
    body.split("tag = \"")
        .skip(1)
        .map(|chunk| {
            let (discriminator, rest) = chunk.split_once('"').expect("each tag assignment is a closed literal");
            if rest.contains("inner = null;") {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\"}}")
            } else {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\",\"{CONTENT_KEY}\":{payload_json}}}")
            }
        })
        .collect()
}

#[test]
fn java_serializer_emits_exactly_what_serde_writes() {
    let emitted = java_emitted_wire_forms(&java_union_output());
    assert_eq!(
        emitted,
        serde_wire_forms(),
        "the generated Jackson serializer must produce serde's adjacent wire form for every \
         variant; flattening the payload beside the tag is serde's *internal* form, which Rust \
         rejects, and a scalar payload has no fields to flatten so it was dropped outright"
    );
}

fn csharp_union_output() -> String {
    generate(&CsharpBackend, &plain_api(), &ResolvedCrateConfig::default())
}

/// Isolate the body of the generated `Write` method, so an assertion cannot be satisfied by a
/// matching literal in the reader above it.
fn csharp_write_body(csharp: &str) -> String {
    let start = csharp
        .find("public override void Write(")
        .expect("C# emits a JsonConverter for the sealed union");
    let rest = &csharp[start..];
    let end = rest.find("\n    }\n").expect("the Write method is closed");
    rest[..end].to_string()
}

/// The JSON the generated C# converter writes for each variant, reconstructed from the emitted
/// code: the `switch` supplies each variant's tag and whether it sets a payload, and the writer
/// block below it supplies where that payload lands.
fn csharp_emitted_wire_forms(csharp: &str) -> Vec<String> {
    let body = csharp_write_body(csharp);
    assert!(
        body.contains(&format!("writer.WriteString(\"{TAG_KEY}\", tag);")),
        "the converter must write the variant tag under serde's tag key:\n{body}"
    );
    assert!(
        body.contains(&format!("writer.WritePropertyName(\"{CONTENT_KEY}\");"))
            && body.contains("JsonSerializer.Serialize(writer, inner, inner.GetType(), options);"),
        "the converter must write the payload whole under serde's content key:\n{body}"
    );
    assert!(
        !body.contains("JsonValueKind.Object"),
        "the payload must not be written only when it happens to be a JSON object — a newtype \
         variant hands the converter a bare scalar, and gating on ValueKind discarded it \
         silently:\n{body}"
    );
    let payload_json = serde_json::to_string(SAMPLE_PAYLOAD).expect("the string payload serializes");
    body.split("tag = \"")
        .skip(1)
        .map(|chunk| {
            let (discriminator, rest) = chunk.split_once('"').expect("each tag assignment is a closed literal");
            if rest.contains("inner = null;") {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\"}}")
            } else {
                format!("{{\"{TAG_KEY}\":\"{discriminator}\",\"{CONTENT_KEY}\":{payload_json}}}")
            }
        })
        .collect()
}

#[test]
fn csharp_converter_emits_exactly_what_serde_writes() {
    let emitted = csharp_emitted_wire_forms(&csharp_union_output());
    assert_eq!(
        emitted,
        serde_wire_forms(),
        "the generated System.Text.Json converter must produce serde's adjacent wire form for \
         every variant; flattening the payload beside the tag is serde's *internal* form, which \
         Rust rejects, and a scalar payload has no fields to flatten so it was dropped outright"
    );
}

#[test]
fn csharp_converter_reads_the_payload_from_serdes_content_key() {
    let csharp = csharp_union_output();
    assert!(
        csharp.contains(&format!(
            "root.TryGetProperty(\"{CONTENT_KEY}\", out var contentElement)"
        )),
        "the converter must read the payload from serde's content key:\n{csharp}"
    );
    assert!(
        !csharp.contains(&format!("prop.Name != \"{TAG_KEY}\"")),
        "reassembling the payload from every field that is not the tag is serde's *internal* \
         form; an adjacently tagged document keeps its payload under the content key:\n{csharp}"
    );
    for tag in serde_variant_tags() {
        assert!(
            csharp.contains(&format!("\"{tag}\" =>")),
            "the converter must dispatch on serde's variant tag {tag:?}:\n{csharp}"
        );
    }
}

#[test]
fn java_deserializer_reads_the_payload_from_serdes_content_key() {
    let java = java_union_output();
    assert!(
        java.contains(&format!("node.get(\"{CONTENT_KEY}\")")),
        "the deserializer must read the payload from serde's content key:\n{java}"
    );
    assert!(
        !java.contains(&format!("node.remove(\"{TAG_KEY}\")")),
        "stripping the tag and reading the remainder as the payload is serde's *internal* form; \
         an adjacently tagged document keeps its payload under the content key:\n{java}"
    );
    for tag in serde_variant_tags() {
        assert!(
            java.contains(&format!("case \"{tag}\" ->")),
            "the deserializer must dispatch on serde's variant tag {tag:?}:\n{java}"
        );
    }
}
