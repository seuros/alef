//! One decision point for the shape of a Python-visible function signature.
//!
//! Two emitters describe the same free function: the `api.py` facade
//! (`gen_bindings::functions::function_wrappers`) and the `<module>.pyi` stub
//! (`gen_stubs::functions`). They must never disagree about which parameters exist or what order
//! they appear in, so both call [`python_signature_params`] instead of re-deriving it.
//!
//! They may disagree about *defaults*: the facade can construct a `Default` for a parameter the
//! native module still requires, which is why the caller supplies its own `facade_defaultable`
//! predicate. The stub passes a predicate that is always false.

use crate::codegen::shared::is_promoted_optional;
use crate::core::ir::ParamDef;

/// One parameter of an emitted Python signature, in emission order.
pub(in crate::backends::pyo3) struct PySignatureParam<'a> {
    pub param: &'a ParamDef,
    /// The emitted signature gives this parameter a `= None` default.
    pub defaulted: bool,
}

/// Decide which parameters carry a `= None` default in the emitted Python signature.
///
/// Parameters are returned in declaration order — never reordered. Python forbids a defaulted
/// parameter before a non-defaulted one, so an extra default is only granted when every parameter
/// after it is already defaulted. Reordering instead would silently rebind every positional call,
/// since the Rust source, the native `#[pyo3(signature = ...)]`, the `.pyi` stub and the generated
/// docs all keep declaration order. ~keep
pub(in crate::backends::pyo3) fn python_signature_params<'a>(
    params: &'a [ParamDef],
    facade_defaultable: impl Fn(&ParamDef) -> bool,
) -> Vec<PySignatureParam<'a>> {
    // `is_promoted_optional` already makes every parameter after an `Option<T>` defaulted, so this
    // base set is suffix-closed; granting extra defaults back-to-front keeps it that way. ~keep
    let mut defaulted: Vec<bool> = params
        .iter()
        .enumerate()
        .map(|(idx, param)| param.optional || is_promoted_optional(params, idx))
        .collect();

    let mut suffix_all_defaulted = true;
    for idx in (0..params.len()).rev() {
        if !defaulted[idx] && suffix_all_defaulted && facade_defaultable(&params[idx]) {
            defaulted[idx] = true;
        }
        suffix_all_defaulted &= defaulted[idx];
    }

    params
        .iter()
        .zip(defaulted)
        .map(|(param, defaulted)| PySignatureParam { param, defaulted })
        .collect()
}

/// The `Named` type a parameter ultimately refers to, looking through one `Option<T>` layer.
/// Both emitters key their "can the facade default this?" lookup on this name.
pub(in crate::backends::pyo3) fn leaf_named_type(param: &ParamDef) -> Option<&str> {
    use crate::core::ir::TypeRef;
    match &param.ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeRef;

    fn param(name: &str, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional,
            ..Default::default()
        }
    }

    fn shape(params: &[ParamDef], defaultable: &[&str]) -> Vec<(String, bool)> {
        python_signature_params(params, |p| defaultable.contains(&p.name.as_str()))
            .into_iter()
            .map(|entry| (entry.param.name.clone(), entry.defaulted))
            .collect()
    }

    #[test]
    fn a_defaultable_param_before_a_required_one_stays_required() {
        let params = vec![param("options", false), param("source", false)];
        assert_eq!(
            shape(&params, &["options"]),
            vec![("options".to_string(), false), ("source".to_string(), false)]
        );
    }

    #[test]
    fn a_trailing_defaultable_param_is_defaulted() {
        let params = vec![param("source", false), param("options", false)];
        assert_eq!(
            shape(&params, &["options"]),
            vec![("source".to_string(), false), ("options".to_string(), true)]
        );
    }

    #[test]
    fn an_optional_param_promotes_every_later_param_to_defaulted() {
        let params = vec![param("source", true), param("limit", false)];
        assert_eq!(
            shape(&params, &[]),
            vec![("source".to_string(), true), ("limit".to_string(), true)]
        );
    }

    #[test]
    fn adjacent_defaultable_params_are_granted_defaults_back_to_front() {
        let params = vec![param("source", false), param("config", false), param("options", false)];
        assert_eq!(
            shape(&params, &["config", "options"]),
            vec![
                ("source".to_string(), false),
                ("config".to_string(), true),
                ("options".to_string(), true),
            ]
        );
    }

    #[test]
    fn leaf_named_type_looks_through_one_option_layer() {
        let mut wrapped = param("options", true);
        wrapped.ty = TypeRef::Optional(Box::new(TypeRef::Named("WidgetOptions".to_string())));
        assert_eq!(leaf_named_type(&wrapped), Some("WidgetOptions"));
        assert_eq!(leaf_named_type(&param("source", false)), None);
    }
}
