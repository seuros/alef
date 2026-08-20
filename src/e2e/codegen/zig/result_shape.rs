//! IR-derived detection of which Zig e2e calls return a serde-JSON struct.
//!
//! `test_file.rs::render_test_fn` used to decide `result_is_json_struct` purely from an
//! explicit `[overrides.zig] result_is_json_struct = true` or a configured `client_factory`.
//! But `zig_return_type` in `src/backends/zig/gen_bindings/functions.rs` maps EVERY `Named`
//! struct return whose IR type has `has_serde` to `[]u8` (JSON) unconditionally, regardless of
//! whether the e2e call config ever said so. A plain top-level function returning such a struct,
//! with no override and no `client_factory`, therefore took the typed-struct assertion path and
//! emitted `result.<field>` against a byte slice — a defect present for every field on every
//! such call, since the backend and the e2e generator disagreed about the wrapper's return type.
//!
//! This module answers the same question the backend answers, from the same IR fact
//! (`TypeDef::has_serde`, `TypeDef::is_opaque`, `TypeDef::is_trait`), so the two can no longer
//! silently disagree for a call the e2e config never explicitly annotated.

use std::collections::HashSet;

use crate::core::ir::TypeDef;
use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::CallConfig;

/// Names of IR types the Zig backend serializes across the FFI boundary as JSON (`[]u8`),
/// mirroring `zig_struct_names` in `src/backends/zig/gen_bindings/mod.rs` — the exact predicate
/// the Zig backend itself uses to decide `[]u8` vs. a real typed struct for a `Named` return.
/// Kept independent rather than shared: the backend's version reads an `ApiSurface`, while e2e
/// codegen only ever has the IR's `type_defs` slice. ~keep
pub(super) fn json_struct_type_names(type_defs: &[TypeDef]) -> HashSet<String> {
    type_defs
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde)
        .map(|t| t.name.clone())
        .collect()
}

/// Whether the call's declared Rust return type (per the core IR, unwrapped through
/// `Option`/`Vec`) names a type the Zig backend serializes to JSON.
///
/// Additive only: this never turns an already-`true` `result_is_json_struct` (from an override
/// or `client_factory`) back to `false` — callers `||` this in. An absent IR, or a return type
/// the IR does not resolve, answers `false`, which is the pre-existing behaviour for every call
/// this fix does not touch.
pub(super) fn ir_says_json_struct(call: &CallConfig, lang: &str, ir: CallIr<'_>, type_defs: &[TypeDef]) -> bool {
    resolve_declared_result_type(call, lang, ir).is_some_and(|name| json_struct_type_names(type_defs).contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FunctionDef, TypeRef};

    fn json_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            has_serde: true,
            ..TypeDef::default()
        }
    }

    fn opaque_handle_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            is_opaque: true,
            ..TypeDef::default()
        }
    }

    #[test]
    fn a_serde_struct_type_is_a_json_struct_name() {
        let type_defs = vec![json_type("Response")];
        assert_eq!(
            json_struct_type_names(&type_defs),
            ["Response".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn an_opaque_handle_type_is_not_a_json_struct_name() {
        let type_defs = vec![opaque_handle_type("Tree")];
        assert!(json_struct_type_names(&type_defs).is_empty());
    }

    #[test]
    fn a_non_serde_plain_struct_type_is_not_a_json_struct_name() {
        let type_defs = vec![TypeDef {
            name: "Config".to_string(),
            has_serde: false,
            is_opaque: false,
            ..TypeDef::default()
        }];
        assert!(json_struct_type_names(&type_defs).is_empty());
    }

    #[test]
    fn a_free_function_returning_a_serde_struct_is_detected_from_the_ir() {
        let type_defs = vec![json_type("Response")];
        let functions = vec![FunctionDef {
            name: "process".to_string(),
            return_type: TypeRef::Named("Response".to_string()),
            ..FunctionDef::default()
        }];
        let ir = CallIr {
            functions: &functions,
            type_defs: &type_defs,
        };
        let call = CallConfig {
            function: "process".to_string(),
            ..CallConfig::default()
        };
        assert!(ir_says_json_struct(&call, "zig", ir, &type_defs));
    }

    /// Misclassification guard: a free function returning a genuine opaque handle (no
    /// `has_serde`) must NOT be classified as a JSON struct, even though it is a `Named` return
    /// like the positive case above.
    #[test]
    fn a_free_function_returning_an_opaque_handle_is_not_detected_as_json_struct() {
        let type_defs = vec![opaque_handle_type("Tree")];
        let functions = vec![FunctionDef {
            name: "parse".to_string(),
            return_type: TypeRef::Named("Tree".to_string()),
            ..FunctionDef::default()
        }];
        let ir = CallIr {
            functions: &functions,
            type_defs: &type_defs,
        };
        let call = CallConfig {
            function: "parse".to_string(),
            ..CallConfig::default()
        };
        assert!(!ir_says_json_struct(&call, "zig", ir, &type_defs));
    }

    #[test]
    fn an_absent_ir_is_not_detected_as_json_struct() {
        let type_defs = vec![json_type("Response")];
        let call = CallConfig {
            function: "process".to_string(),
            ..CallConfig::default()
        };
        assert!(!ir_says_json_struct(&call, "zig", CallIr::default(), &type_defs));
    }
}
