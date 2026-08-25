//! Two-target SwiftPM compile regression for the `FunctionParam` trait-box generator.
//!
//! Every other Swift trait-bridge test asserts on emitted *strings*, which is how alef #258
//! shipped twice: a string can be spelled exactly as intended and still not compile. This test
//! writes the generated `Sources/RustBridge/*.swift` into a real SwiftPM package whose target
//! graph matches what alef scaffolds -- `Client -> RustBridge`, never the reverse -- and runs
//! `swift build` on it.
//!
//! The dependency direction is the whole point. Public `Codable` DTOs live in the downstream
//! `Client` target, so the `RustBridge` target where the box and the bridge protocol are emitted
//! cannot name them at all. Any generated code that mentions a DTO type, encodes one, or
//! constructs a case of one is a compile error here even though it reads fine as a string.

#![allow(clippy::print_stderr)]

use crate::backends::swift::gen_bindings::boxes::emit_function_param_box_files;
use crate::backends::swift::gen_bindings::trait_bridge::gen_trait_bridge_files;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimal stand-in for the swift-bridge runtime types that ship inside the real `RustBridge`
/// target (`SwiftBridgeCore.swift`). Only the surface the generated box glue actually touches is
/// modelled, and each member keeps swift-bridge's own signature so a box method that type-checks
/// here type-checks against the real runtime.
const RUST_BRIDGE_RUNTIME_STUB: &str = r#"import Foundation

public class RustStr {
    public init() {}
    public func toString() -> String { return "" }
}

public class RustString {
    public init(_ value: String) { self.value = value }
    public let value: String
    public func toString() -> String { return value }
    public func as_str() -> RustStr { return RustStr() }
}

public class RustVec<T> {
    private var storage: [T] = []
    public init() {}
    public func push(value: T) { storage.append(value) }
    public func len() -> UInt { return UInt(storage.count) }
    public func get(index: UInt) -> T? { return storage[Int(index)] }
}
"#;

/// The public `Codable` DTOs, emitted into the **downstream** target exactly as alef emits
/// first-class types into `Sources/<Module>/`. `RustBridge` does not depend on `Client`, so these
/// names are unresolvable from any generated file under `Sources/RustBridge/`.
const CLIENT_DTOS: &str = r#"import Foundation
import RustBridge

public enum PageLayout: String, Codable, Sendable {
    case single
    case facing
}

public struct SinkStats: Codable, Sendable {
    public var accepted: UInt32
    public init(accepted: UInt32) { self.accepted = accepted }
}
"#;

/// A conforming implementation compiled in the downstream target.
///
/// This pins the protocol's *public* shape: it only compiles when every bridged `Named` type is
/// declared as a JSON `String` at the boundary, and it only compiles when alef stops shipping
/// invented default stubs -- a defaulted method alef no longer implements for the consumer must
/// be satisfied here or conformance fails.
const CLIENT_CONFORMER: &str = r#"import Foundation
import RustBridge

public final class FixtureSink: SwiftDocumentSinkBridge {
    public init() {}

    public var name: String { return "fixture" }
    public func version() -> String { return "0.0.0" }
    public func initialize() throws {}
    public func shutdown() throws {}

    public func accept(chunk: String) throws {}

    public func pageLayout() -> String {
        return String(data: try! JSONEncoder().encode(PageLayout.facing), encoding: .utf8)!
    }

    public func stats() -> String {
        return String(data: try! JSONEncoder().encode(SinkStats(accepted: 0)), encoding: .utf8)!
    }

    public func isReady() -> Bool { return false }

    public func describe(layout: String) -> String { return layout }

    public func statsHistory() -> [String] { return [] }

    public func lastStats() -> String? { return nil }

    public func record(entries: [String]) {}

    public func sinkTotals() -> String {
        return "{}"
    }
}
"#;

const PACKAGE_MANIFEST: &str = r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "AlefTraitBoxFixture",
    products: [
        .library(name: "Client", targets: ["Client"])
    ],
    targets: [
        .target(name: "RustBridge"),
        .target(name: "Client", dependencies: ["RustBridge"])
    ]
)
"#;

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        error_type: error_type.map(str::to_string),
        ..Default::default()
    }
}

fn defaulted(mut m: MethodDef) -> MethodDef {
    m.has_default_impl = true;
    m
}

/// A trait whose defaulted methods mention DTO types in both parameter and return position --
/// the exact combination alef #258 marshalled incorrectly.
///
/// `stats_history`, `last_stats` and `record` extend that to DTO types nested in a container. A
/// bridged `Named` crosses as a JSON `String`, so the protocol declares `[String]` / `String?` /
/// `[String]` for them while the shim declares whatever the Rust extern block declares -- and the
/// two only agree when the marshaller understands that a `Vec<Named>` is a `Vec<String>`. Before
/// this fixture grew those methods the generator emitted `-> RustString { return
/// bridge.statsHistory() }`, which is a `[String]` returned as a `RustString`.
///
/// `sink_totals` is the `Map<_, Named>` counterpart (alef-tasks #309): a bridged `Named` value
/// crosses as one JSON blob, and that rule does not change when the `Named` is a `Map` value --
/// swift-bridge cannot bridge `HashMap<K, V>` at all, Named or not. The protocol must declare a
/// plain `String`, not `[String: String]` (which would double-encode every value), and the box
/// shim's return marshal must wrap the bridge call in `RustString(...)` to match the declared
/// `RustString` FFI return type. ~keep
fn fixture_api() -> (ApiSurface, ResolvedCrateConfig) {
    let trait_def = TypeDef {
        name: "DocumentSink".to_string(),
        rust_path: "fixture_core::DocumentSink".to_string(),
        is_trait: true,
        methods: vec![
            method(
                "accept",
                vec![param("chunk", TypeRef::String)],
                TypeRef::Unit,
                Some("SinkError"),
            ),
            defaulted(method(
                "page_layout",
                vec![],
                TypeRef::Named("PageLayout".to_string()),
                None,
            )),
            defaulted(method("stats", vec![], TypeRef::Named("SinkStats".to_string()), None)),
            defaulted(method(
                "is_ready",
                vec![],
                TypeRef::Primitive(PrimitiveType::Bool),
                None,
            )),
            defaulted(method(
                "describe",
                vec![param("layout", TypeRef::Named("PageLayout".to_string()))],
                TypeRef::String,
                None,
            )),
            defaulted(method(
                "stats_history",
                vec![],
                TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
                None,
            )),
            defaulted(method(
                "last_stats",
                vec![],
                TypeRef::Optional(Box::new(TypeRef::Named("SinkStats".to_string()))),
                None,
            )),
            method(
                "record",
                vec![param(
                    "entries",
                    TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
                )],
                TypeRef::Unit,
                None,
            ),
            defaulted(method(
                "sink_totals",
                vec![],
                TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("SinkStats".to_string())),
                ),
                None,
            )),
        ],
        ..Default::default()
    };

    let stats = TypeDef {
        name: "SinkStats".to_string(),
        rust_path: "fixture_core::SinkStats".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "accepted".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    };

    let page_layout = EnumDef {
        name: "PageLayout".to_string(),
        rust_path: "fixture_core::PageLayout".to_string(),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Single".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Facing".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "fixture_core".to_string(),
        types: vec![trait_def, stats],
        enums: vec![page_layout],
        ..Default::default()
    };

    let config = ResolvedCrateConfig {
        name: "fixture".to_string(),
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "DocumentSink".to_string(),
            register_fn: Some("registerDocumentSink".to_string()),
            bind_via: BridgeBinding::FunctionParam,
            ..Default::default()
        }],
        ..Default::default()
    };

    (api, config)
}

/// Mirror of the `box_exclude` set `gen_bindings::mod` hands both generators: config exclusions
/// plus every serde-carrying struct in the IR. Enums are deliberately absent here because
/// `ApiSurface::enums` is a separate collection that the production code does not walk -- keeping
/// the omission is what lets this fixture reach the enum path under test.
fn box_exclude(api: &ApiSurface) -> HashSet<String> {
    api.types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde)
        .map(|t| t.name.clone())
        .collect()
}

/// Write the generated `RustBridge` sources plus the fixture's hand-written targets into a
/// throwaway SwiftPM package and return its root.
fn materialize_package(root: &Path) -> std::io::Result<()> {
    let (api, config) = fixture_api();
    let exclude = box_exclude(&api);

    let rust_bridge = root.join("Sources").join("RustBridge");
    let client = root.join("Sources").join("Client");
    std::fs::create_dir_all(&rust_bridge)?;
    std::fs::create_dir_all(&client)?;

    std::fs::write(root.join("Package.swift"), PACKAGE_MANIFEST)?;
    std::fs::write(rust_bridge.join("SwiftBridgeCoreStub.swift"), RUST_BRIDGE_RUNTIME_STUB)?;
    std::fs::write(client.join("ClientDTOs.swift"), CLIENT_DTOS)?;
    std::fs::write(client.join("FixtureSink.swift"), CLIENT_CONFORMER)?;

    let trait_defs: Vec<_> = config
        .trait_bridges
        .iter()
        .filter_map(|b| {
            api.types
                .iter()
                .find(|t| t.is_trait && t.name == b.trait_name)
                .map(|t| (b.trait_name.clone(), b, t))
        })
        .collect();

    for (filename, content) in gen_trait_bridge_files(&trait_defs, &exclude, &HashSet::new()) {
        std::fs::write(rust_bridge.join(filename), content)?;
    }
    for file in emit_function_param_box_files(&api, &config, &rust_bridge, &exclude) {
        std::fs::write(&file.path, &file.content)?;
    }

    Ok(())
}

/// Locate a `swift` driver, or explain loudly why the compile gate is not running.
///
/// On macOS a missing `swift` is an environment fault, not a portability concern -- failing there
/// is deliberate, because a silently skipped compile gate is how this defect shipped in the first
/// place. Elsewhere the toolchain genuinely may be absent, so the skip is allowed but shouted.
fn swift_driver() -> Option<PathBuf> {
    if let Ok(path) = which::which("swift") {
        return Some(path);
    }
    if cfg!(target_os = "macos") {
        panic!(
            "`swift` is not on PATH but this is macOS, where the Swift toolchain ships with Xcode. \
             The two-target SwiftPM compile gate for the trait-box generator cannot run, and it is \
             the only test that proves the generated Swift compiles. Install the toolchain rather \
             than letting this gate lapse."
        );
    }
    eprintln!(
        "\n\
         ================================================================\n\
         SKIPPED: generated_trait_box_package_compiles\n\
         No `swift` driver on PATH, so the generated Swift was NOT compiled.\n\
         String-level assertions alone cannot catch alef #258; this run\n\
         provides NO evidence that the trait-box output builds.\n\
         ================================================================\n"
    );
    None
}

/// The gate: alef's generated trait-box output must build in the two-target layout it is
/// generated for.
///
/// Sabotage checks for this test:
/// - restoring the `has_default_impl` skip in `excluded_named_type_bridge_policy` reproduces the
///   shipped 0.67.5 emission and this test must fail with `cannot find type 'PageLayout' in
///   scope` from `Sources/RustBridge/`.
/// - reverting `swift_type_name`'s `Map` arm to recurse (alef-tasks #309) makes `sinkTotals()`
///   declare `[String: String]`, which does not match `String` on `FixtureSink`, and this test
///   must fail with a protocol-conformance error naming `sinkTotals`.
/// - reverting the `swift_shim_return_marshal` catch-all wrap makes the box shim `return
///   bridge.sinkTotals()` without `RustString(...)`, and this test must fail with a type
///   mismatch naming `RustString` in `Swift{Trait}Box.swift`.
#[test]
fn generated_trait_box_package_compiles() {
    let Some(swift) = swift_driver() else { return };

    // `swift build` reports errors against the generated files, but a failing run deletes the
    // temp dir before anyone can read them. Point this at a real path to keep the package. ~keep
    let keep = std::env::var_os("ALEF_SWIFT_FIXTURE_DIR").map(PathBuf::from);
    let tmp = tempfile::tempdir().expect("create fixture package dir");
    let root = keep.as_deref().unwrap_or_else(|| tmp.path());
    materialize_package(root).expect("write fixture SwiftPM package");

    let output = Command::new(&swift)
        .arg("build")
        .arg("--package-path")
        .arg(root)
        .arg("--scratch-path")
        .arg(root.join(".build"))
        .output()
        .expect("run swift build");

    assert!(
        output.status.success(),
        "generated trait-box package failed to compile in the Client -> RustBridge layout.\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
