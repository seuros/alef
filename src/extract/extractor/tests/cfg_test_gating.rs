use super::*;

/// `#[cfg(test)]` associated functions (the canonical `HeuristicsConfig::test_config`
/// case) must never reach the binding surface, while normal and feature-gated methods
/// are retained.
#[test]
fn test_cfg_test_method_excluded_feature_method_retained() {
    let source = r#"
        pub struct HeuristicsConfig {
            pub threshold: u32,
        }

        impl HeuristicsConfig {
            pub fn new() -> Self {
                Self { threshold: 100 }
            }

            #[cfg(test)]
            pub fn test_config() -> Self {
                Self { threshold: 1 }
            }

            #[cfg(feature = "x")]
            pub fn feature_config() -> Self {
                Self { threshold: 50 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface
        .types
        .iter()
        .find(|t| t.name == "HeuristicsConfig")
        .expect("HeuristicsConfig should be extracted");

    let method_names: Vec<&str> = config.methods.iter().map(|m| m.name.as_str()).collect();

    assert!(
        !method_names.contains(&"test_config"),
        "#[cfg(test)] method must be excluded, got {method_names:?}"
    );
    assert!(
        method_names.contains(&"feature_config"),
        "#[cfg(feature = \"x\")] method must be retained, got {method_names:?}"
    );

    // Retention used to be the whole contract: the gate itself was read and thrown away, so
    // `feature_config` reached every backend as an unconditional method. That is now reversed —
    // `MethodDef` carries `cfg`, because one `ApiSurface` is extracted once and handed to every
    // backend with its own `features_for_language`, so the gate cannot be resolved at extraction
    // time and must survive into the IR. ~keep
    let feature_config = config
        .methods
        .iter()
        .find(|m| m.name == "feature_config")
        .expect("feature_config should be extracted");
    assert_eq!(
        feature_config.cfg.as_deref(),
        Some("feature = \"x\""),
        "a gated method must carry its own cfg into the IR"
    );
}

/// A method's gate is the conjunction of its own `#[cfg]` and its `impl` block's: the method
/// is compiled only when both hold. The block's gate used to be ignored entirely (only
/// `is_test_gated` was consulted), so a method in a `#[cfg(feature = "x")] impl` block reached
/// backends unconditionally.
#[test]
fn method_cfg_and_combines_with_its_impl_block_cfg() {
    let source = r#"
        pub struct Gauge {
            pub value: u32,
        }

        #[cfg(feature = "metrics")]
        impl Gauge {
            pub fn read(&self) -> u32 {
                self.value
            }

            #[cfg(feature = "histogram")]
            pub fn buckets(&self) -> u32 {
                self.value
            }
        }
    "#;

    let surface = extract_from_source(source);
    let gauge = surface
        .types
        .iter()
        .find(|t| t.name == "Gauge")
        .expect("Gauge should be extracted");

    let read = gauge.methods.iter().find(|m| m.name == "read").expect("read");
    assert_eq!(
        read.cfg.as_deref(),
        Some("feature = \"metrics\""),
        "an ungated method inherits its impl block's gate verbatim"
    );

    let buckets = gauge.methods.iter().find(|m| m.name == "buckets").expect("buckets");
    assert_eq!(
        buckets.cfg.as_deref(),
        Some("all(feature = \"metrics\", feature = \"histogram\")"),
        "a gated method AND-combines with its impl block's gate"
    );
}

/// The two halves of the contract, asserted together: extraction must *retain* the gate, and
/// per-language filtering must then *use* it. Retention without filtering is as wrong as
/// dropping — the gate would reach a backend that cannot express it and be emitted
/// unconditionally, referencing an FFI symbol the linked library never compiled.
#[test]
fn gated_method_survives_extraction_then_drops_for_a_language_without_the_feature() {
    let source = r#"
        pub struct Client {
            pub id: u32,
        }

        impl Client {
            pub fn ping(&self) -> u32 {
                self.id
            }

            #[cfg(feature = "streaming")]
            pub fn stream(&self) -> u32 {
                self.id
            }
        }
    "#;

    let surface = extract_from_source(source);
    let extracted: Vec<&str> = surface.types[0].methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        extracted,
        vec!["ping", "stream"],
        "half one: extraction retains the gated method"
    );

    let with_feature: std::collections::HashSet<&str> = ["streaming"].into_iter().collect();
    let kept = surface.with_cfg_filtered_deep(&with_feature);
    let kept_names: Vec<&str> = kept.types[0].methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        kept_names,
        vec!["ping", "stream"],
        "half two: a language whose feature set satisfies the gate keeps the method"
    );

    let without_feature: std::collections::HashSet<&str> = ["other"].into_iter().collect();
    let dropped = surface.with_cfg_filtered_deep(&without_feature);
    let dropped_names: Vec<&str> = dropped.types[0].methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        dropped_names,
        vec!["ping"],
        "half two: a language whose feature set does not satisfy the gate drops the method, and \
         the ungated control is unaffected"
    );
}

/// A whole `#[cfg(test)]` impl block must be skipped, while its sibling normal impl
/// block is fully extracted.
#[test]
fn test_cfg_test_impl_block_excluded() {
    let source = r#"
        pub struct Widget {
            pub size: u32,
        }

        impl Widget {
            pub fn real_method(&self) -> u32 {
                self.size
            }
        }

        #[cfg(test)]
        impl Widget {
            pub fn fixture() -> Self {
                Self { size: 7 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let widget = surface
        .types
        .iter()
        .find(|t| t.name == "Widget")
        .expect("Widget should be extracted");

    let method_names: Vec<&str> = widget.methods.iter().map(|m| m.name.as_str()).collect();
    assert!(
        method_names.contains(&"real_method"),
        "method from normal impl must be retained, got {method_names:?}"
    );
    assert!(
        !method_names.contains(&"fixture"),
        "method from #[cfg(test)] impl must be excluded, got {method_names:?}"
    );
}

/// Top-level `#[cfg(test)]` items (struct, enum, free function) are excluded while a
/// `#[cfg(feature = "x")]` item and a plain item are retained.
#[test]
fn test_cfg_test_top_level_items_excluded() {
    let source = r#"
        pub struct NormalType {
            pub value: u32,
        }

        #[cfg(test)]
        pub struct TestOnlyType {
            pub value: u32,
        }

        #[cfg(feature = "x")]
        pub struct FeatureType {
            pub value: u32,
        }

        pub fn normal_fn() -> u32 {
            1
        }

        #[cfg(test)]
        pub fn test_only_fn() -> u32 {
            2
        }

        #[cfg(all(test, feature = "x"))]
        pub fn nested_test_fn() -> u32 {
            3
        }

        #[cfg(not(test))]
        pub fn non_test_fn() -> u32 {
            4
        }
    "#;

    let surface = extract_from_source(source);

    let type_names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    assert!(type_names.contains(&"NormalType"), "got {type_names:?}");
    assert!(type_names.contains(&"FeatureType"), "got {type_names:?}");
    assert!(
        !type_names.contains(&"TestOnlyType"),
        "#[cfg(test)] struct must be excluded, got {type_names:?}"
    );

    let fn_names: Vec<&str> = surface.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"normal_fn"), "got {fn_names:?}");
    assert!(
        fn_names.contains(&"non_test_fn"),
        "#[cfg(not(test))] fn must be retained, got {fn_names:?}"
    );
    assert!(
        !fn_names.contains(&"test_only_fn"),
        "#[cfg(test)] fn must be excluded, got {fn_names:?}"
    );
    assert!(
        !fn_names.contains(&"nested_test_fn"),
        "#[cfg(all(test, ...))] fn must be excluded, got {fn_names:?}"
    );
}

/// A `#[cfg(feature = "…")] pub mod` gate must reach the items inside it no matter which
/// source file declares the module. `sources` is an author-ordered list, so the file holding
/// the gated `pub mod` is frequently not the first entry; when only the first source was
/// scanned, every item under the gate was recorded with `cfg: None` and backends that skip
/// items on `cfg` emitted calls into modules their feature set does not compile.
#[test]
fn module_cfg_applies_when_the_gated_module_is_not_the_first_source() {
    let dir = tempfile::tempdir().expect("tempdir");

    let other_rs = dir.path().join("other.rs");
    std::fs::write(&other_rs, "pub fn unrelated() -> u32 { 0 }\n").expect("write other.rs");

    let lib_rs = dir.path().join("lib.rs");
    std::fs::write(
        &lib_rs,
        r#"
        #[cfg(feature = "metrics")]
        pub mod metrics {
            pub fn record_cost(system: &str) -> u32 { system.len() as u32 }
        }
        "#,
    )
    .expect("write lib.rs");

    // `lib.rs` deliberately second: this is the ordering that regressed.
    let surface =
        super::extract(&[other_rs.as_path(), lib_rs.as_path()], "my_crate", "0.0.0", None).expect("extract failed");

    let record_cost = surface
        .functions
        .iter()
        .find(|f| f.name == "record_cost")
        .expect("record_cost should be extracted");

    assert_eq!(
        record_cost.cfg.as_deref(),
        Some("feature = \"metrics\""),
        "item under a gated module must carry the module's cfg regardless of source order"
    );

    let unrelated = surface
        .functions
        .iter()
        .find(|f| f.name == "unrelated")
        .expect("unrelated should be extracted");
    assert_eq!(
        unrelated.cfg, None,
        "an ungated item must not inherit an unrelated module's cfg"
    );
}
