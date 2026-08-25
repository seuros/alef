//! Task #385: "IR duplicates identical items" -- extraction reportedly emitted the same item
//! twice into `ApiSurface` where the source declared it once. These tests pin the shapes named
//! as likely triggers (an item reachable through two `use` paths, a re-export, a type referenced
//! from more than one module, and a `#[cfg]`-gated pair) against today's extractor.
//!
//! Every shape below produces exactly one entry per genuinely-single declaration -- the
//! candidate reproductions did not reproduce. The one place the extractor deliberately keeps two
//! entries for the same name is a `#[cfg]`-disjoint pair (mirroring the documented behavior for
//! functions in `codegen::fn_dedup`); `test_cfg_disjoint_same_name_structs_are_not_collapsed_here`
//! pins that intentional non-dedup so a future change doesn't accidentally special-case it away
//! here and break the cfg-merge logic in `cli::pipeline::extract::type_helpers::dedup_api_surface`,
//! which is the layer that actually collapses same-name entries. ~keep
use super::*;

#[test]
fn test_pub_use_mirror_of_locally_declared_type_is_not_duplicated() {
    let surface = extract_from_source(
        r#"
        pub mod inner {
            pub struct Widget {
                pub id: i32,
            }
        }
        pub use inner::Widget;
        "#,
    );
    assert_eq!(surface.types.len(), 1, "Widget must appear once, not once per use path");
    assert_eq!(surface.types[0].name, "Widget");
    assert_eq!(surface.types[0].rust_path, "test_crate::Widget");
}

#[test]
fn test_type_referenced_from_two_modules_is_extracted_once() {
    let surface = extract_from_source(
        r#"
        pub struct Widget {
            pub id: i32,
        }
        pub mod a {
            pub fn make_a(w: super::Widget) -> super::Widget { w }
        }
        pub mod b {
            pub fn make_b(w: super::Widget) -> super::Widget { w }
        }
        "#,
    );
    assert_eq!(
        surface.types.len(),
        1,
        "a type referenced from multiple modules must not be re-extracted per referencing module"
    );
    assert_eq!(surface.types[0].name, "Widget");
    assert_eq!(surface.functions.len(), 2);
}

#[test]
fn test_glob_reexport_of_local_module_does_not_duplicate_its_type() {
    let surface = extract_from_source(
        r#"
        pub use inner::*;
        pub mod inner {
            pub struct Widget {
                pub id: i32,
            }
        }
        "#,
    );
    assert_eq!(surface.types.len(), 1, "a glob-reexported local type must appear once");
    assert_eq!(surface.types[0].name, "Widget");
    assert_eq!(surface.types[0].rust_path, "test_crate::Widget");
}

#[test]
fn test_named_reexport_of_private_module_keeps_single_entry_and_filters_others() {
    let surface = extract_from_source(
        r#"
        mod inner {
            pub struct Widget {
                pub id: i32,
            }
            pub struct Other {
                pub value: i32,
            }
        }
        pub use inner::Widget;
        "#,
    );
    assert_eq!(
        surface.types.len(),
        1,
        "only the named-reexported type from a private module should survive, exactly once"
    );
    assert_eq!(surface.types[0].name, "Widget");
    assert_eq!(surface.types[0].rust_path, "test_crate::Widget");
}

/// Pins the extractor's intentional, documented non-dedup of `#[cfg]`-disjoint same-name items
/// (see module docs above). This is NOT the reported bug: both entries carry different, mutually
/// exclusive `cfg` gates, matching the accepted stub/real-impl pattern already documented for
/// functions in `src/codegen/fn_dedup.rs` and for types in
/// `extract::extractor::disambiguation::compute_renames`'s shadow/stub dedup-by-path comment.
#[test]
fn test_cfg_disjoint_same_name_structs_are_not_collapsed_here() {
    let surface = extract_from_source(
        r#"
        #[cfg(feature = "x")]
        pub struct Config {
            pub timeout: u32,
        }

        #[cfg(not(feature = "x"))]
        pub struct Config {
            pub timeout: u32,
            pub retries: u32,
        }
        "#,
    );
    assert_eq!(
        surface.types.len(),
        2,
        "cfg-disjoint same-name structs are preserved here for downstream dedup, same as functions"
    );
    assert!(surface.types.iter().all(|t| t.name == "Config"));
    assert!(surface.types.iter().all(|t| t.rust_path == "test_crate::Config"));
    assert!(surface.types.iter().all(|t| !t.binding_excluded));
    let cfgs: std::collections::BTreeSet<_> = surface.types.iter().map(|t| t.cfg.clone()).collect();
    assert_eq!(
        cfgs.len(),
        2,
        "the two entries must carry the two distinct cfg gates, not collapse to one"
    );
}
