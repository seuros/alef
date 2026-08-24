use super::*;
use crate::core::validation::{ValidationCode, validate_api_surface};

/// Build a crate whose module declares three `alef(skip)` functions and re-exports
/// them through two `pub use` statements: one plain, one `#[cfg(...)]`-gated.
fn write_gated_reexport_crate(dir_name: &str, gated_fn_signature: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(dir_name);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod inner;

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub use inner::gated_helper;
pub use inner::{plain_helper, other_helper};
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/inner.rs"),
        format!(
            r#"
#[cfg_attr(alef, alef(skip))]
pub fn plain_helper(holder: (String, u32)) -> String {{
    holder.0
}}

#[cfg_attr(alef, alef(skip))]
pub fn other_helper(holder: (String, u32)) -> String {{
    holder.0
}}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
#[cfg_attr(alef, alef(skip))]
{gated_fn_signature}
"#
        ),
    )
    .unwrap();

    tmp
}

/// A signature alef cannot represent: the tuple parameter is sanitized away, so the
/// function raises `lossy_sanitized_surface` unless it is excluded from the surface.
const LOSSY_GATED_FN: &str = r#"pub fn gated_helper(holder: (String, u32)) -> String {
    holder.0
}"#;

fn extract_crate_at(tmp: &std::path::Path) -> ApiSurface {
    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    super::extract(&sources, "demo", "0.1.0", None).unwrap()
}

#[test]
fn cfg_gated_reexport_keeps_the_declared_skip_on_the_source_function() {
    let tmp = write_gated_reexport_crate("alef_test_cfg_gated_reexport_skip", LOSSY_GATED_FN);
    let surface = extract_crate_at(&tmp);

    for name in ["plain_helper", "other_helper", "gated_helper"] {
        let entries: Vec<&_> = surface.functions.iter().filter(|f| f.name == name).collect();
        assert!(!entries.is_empty(), "`{name}` should be present in the surface");
        assert!(
            entries.iter().all(|f| f.binding_excluded),
            "`{name}` declares #[cfg_attr(alef, alef(skip))] and must stay binding_excluded; got {entries:?}"
        );
        assert!(
            entries
                .iter()
                .all(|f| f.binding_exclusion_reason.as_deref() == Some("alef(skip)")),
            "`{name}` must keep the alef(skip) exclusion reason; got {entries:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_skipped_function_with_a_lossy_signature_does_not_fail_validation() {
    let tmp = write_gated_reexport_crate("alef_test_cfg_gated_reexport_lossy_skipped", LOSSY_GATED_FN);
    let surface = extract_crate_at(&tmp);

    let report = validate_api_surface(&surface);
    let lossy: Vec<&_> = report
        .errors()
        .filter(|diagnostic| diagnostic.code == ValidationCode::LossySanitizedSurface)
        .collect();
    assert!(
        lossy.is_empty(),
        "every function is skipped, so no lossy_sanitized_surface error may be raised; got {lossy:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn an_unskipped_function_with_the_same_lossy_signature_still_fails_validation() {
    let tmp = std::env::temp_dir().join("alef_test_cfg_gated_reexport_lossy_unskipped");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
pub mod inner;

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub use inner::gated_helper;
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/inner.rs"),
        format!(
            r#"
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
{LOSSY_GATED_FN}
"#
        ),
    )
    .unwrap();

    let surface = extract_crate_at(&tmp);
    let report = validate_api_surface(&surface);
    let lossy: Vec<&_> = report
        .errors()
        .filter(|diagnostic| diagnostic.code == ValidationCode::LossySanitizedSurface)
        .collect();
    assert!(
        !lossy.is_empty(),
        "an unskipped function whose signature is sanitized must still be a fatal error"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
