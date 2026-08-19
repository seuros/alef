//! `extendr_module!` rejects a `#[cfg(...)]` on its entries, so R cannot gate a registration the
//! way Magnus gates its `define_module_function` call. The gate therefore has to be resolved
//! before generation: an enabled function reaches R unconditionally, a disabled one is removed.

use super::super::ExtendrBackend;
use super::{make_config, resolved_one};
use crate::core::backend::Backend;
use crate::core::ir::*;

fn gated_function(name: &str, cfg: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn generate_r(api: &ApiSurface, config: &crate::core::config::ResolvedCrateConfig) -> String {
    ExtendrBackend
        .generate_bindings(api, config)
        .expect("extendr generation")
        .iter()
        .map(|f| format!("// ==== {} ====\n{}", f.path.display(), f.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The regression this module exists for. `always_registered` used to drop any genuinely
/// cfg-gated function from both the `extendr_module!` block and the R wrapper surface
/// unconditionally, so a crate whose feature was on by default still never got the function.
#[test]
fn a_function_gated_on_an_enabled_feature_reaches_the_r_surface() {
    let api = ApiSurface {
        functions: vec![
            gated_function("count_tokens", Some(r#"feature = "tokenizer""#)),
            gated_function("always_there", None),
        ],
        ..Default::default()
    };

    let out = generate_r(&api, &make_config());

    // Precondition: the ungated function must be present, otherwise the assertions below could
    // pass against output that emitted no functions at all.
    assert!(
        out.contains("fn always_there;"),
        "ungated function missing, fixture no longer exercises registration:\n{out}"
    );
    assert!(
        out.contains("fn count_tokens;"),
        "a function gated on an enabled feature must still be registered in extendr_module!:\n{out}"
    );
    assert!(
        !out.contains(r#"#[cfg(feature = "tokenizer")]"#),
        "the gate must be discharged before generation, not copied into the binding crate:\n{out}"
    );
}

/// The other half of the policy: a feature the R build genuinely does not enable must not leave
/// an `extendr_module!` entry or an R wrapper naming a symbol the crate never compiled.
#[test]
fn a_function_gated_on_a_disabled_feature_is_removed_entirely() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
default_features = false
features = ["other"]
"#,
    );
    let api = ApiSurface {
        functions: vec![
            gated_function("count_tokens", Some(r#"feature = "tokenizer""#)),
            gated_function("always_there", None),
        ],
        ..Default::default()
    };

    let out = generate_r(&api, &config);

    assert!(
        out.contains("fn always_there;"),
        "ungated function missing, fixture no longer exercises registration:\n{out}"
    );
    assert!(
        !out.contains("count_tokens"),
        "a function gated on a disabled feature must not be named anywhere in the R surface:\n{out}"
    );
}
