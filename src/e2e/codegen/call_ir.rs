//! The shared core-IR seam every e2e backend lowers arguments through.
//!
//! Historically only the C backend consulted the core IR: [`CallIr`], [`IrSignature`],
//! [`TargetParams`] and [`named_type`] all lived inside `c.rs`/`c/assertions.rs`, so every other
//! backend bound its `functions: &[FunctionDef]` parameter as `_functions` and lowered a
//! configured `ArgMapping` purely from its `arg_type` string (which defaults to `"string"`; see
//! `src/core/config/e2e/defaults.rs`). A configured value therefore never met the declared type of
//! the parameter it fills, and every backend's catch-all stringified whatever it was handed.
//!
//! This module holds the *input* half of the fix and nothing more. [`TargetParams`]' three states
//! are what a backend needs in order to ask the question at all; what it should then emit for a
//! given declared type is a per-backend answer, because the languages genuinely disagree — a Go
//! `type X string` enum accepts a bare string literal where a Java enum does not, and a shared
//! "types differ => refuse" verdict would reject snippets that compile today. Backends read
//! [`TargetParams`] and decide for themselves. ~keep

use crate::e2e::config::CallConfig;

/// The two core-IR registries a call resolves its result and argument types from.
///
/// They travel together because a call name can only be answered by consulting both:
/// `functions` is `ApiSurface::functions`, which holds **free `pub fn`s only**, and every
/// inherent or trait method — a client's `chat`, say — is a [`crate::core::ir::MethodDef`]
/// hanging off a [`crate::core::ir::TypeDef`] in `type_defs`. Passing one without the other
/// answers `None` for half the calls in a typical suite, and every `None` in the C backend lands
/// on its `unresolved_result_type_name`, which fails generation rather than inventing a name. ~keep
#[derive(Clone, Copy, Default)]
pub(crate) struct CallIr<'a> {
    pub functions: &'a [crate::core::ir::FunctionDef],
    pub type_defs: &'a [crate::core::ir::TypeDef],
}

impl<'a> CallIr<'a> {
    /// True when neither registry was supplied, i.e. this generator has no IR to consult at
    /// all. Distinct from "the IR was present and the call was not in it", which is a
    /// per-call authoring problem rather than a structural one.
    pub(crate) fn is_absent(self) -> bool {
        self.functions.is_empty() && self.type_defs.is_empty()
    }

    /// The declared signature for a Rust-side call name: the free function of that name if
    /// there is one, otherwise the method of that name declared on an IR type.
    ///
    /// Free functions win because they are unambiguous — a crate has at most one `pub fn` of
    /// a given path. Methods are not: several types can declare `new`, and a type carrying
    /// both an inherent and a trait-sourced `chat` lists both. Rather than pick one, this
    /// answers only when every same-named method agrees on the signature, so the result is
    /// the one the IR actually determines. Disagreement yields `None` and the caller's
    /// fallback runs, which is exactly the behaviour before methods were consulted at all. ~keep
    pub(crate) fn signature(self, name: &str) -> Option<IrSignature<'a>> {
        if let Some(function) = self.functions.iter().find(|function| function.name == name) {
            return Some(IrSignature {
                params: &function.params,
                return_type: &function.return_type,
            });
        }
        let mut methods = self
            .type_defs
            .iter()
            .flat_map(|type_def| type_def.methods.iter())
            .filter(|method| method.name == name);
        let first = methods.next()?;
        if !methods.all(|other| same_signature(first, other)) {
            return None;
        }
        Some(IrSignature {
            params: &first.params,
            return_type: &first.return_type,
        })
    }
}

/// The parts of a declared signature e2e codegen reads, shared by the free-function and
/// method arms of [`CallIr::signature`].
pub(crate) struct IrSignature<'a> {
    pub params: &'a [crate::core::ir::ParamDef],
    pub return_type: &'a crate::core::ir::TypeRef,
}

/// Whether two same-named methods declare the same thing, for the purposes of the three
/// questions codegen asks a signature: what it returns, and what its parameters are named
/// and typed. `ParamDef` has no `PartialEq`, and the fields beyond name and type (defaults,
/// `is_ref`, newtype wrappers) do not change any answer here.
fn same_signature(left: &crate::core::ir::MethodDef, right: &crate::core::ir::MethodDef) -> bool {
    left.return_type == right.return_type
        && left.params.len() == right.params.len()
        && left
            .params
            .iter()
            .zip(right.params.iter())
            .all(|(left, right)| left.name == right.name && left.ty == right.ty)
}

/// The named type reached through any number of `Option`/`Vec` wrappers, or `None` for a type
/// that names nothing (a primitive, a tuple, a map).
pub(crate) fn named_type(type_ref: &crate::core::ir::TypeRef) -> Option<&str> {
    match type_ref {
        crate::core::ir::TypeRef::Named(name) => Some(name),
        crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => named_type(inner),
        _ => None,
    }
}

/// What the emitter knows about the *target* function's declared parameters.
///
/// An empty `args` list is ambiguous between "this call genuinely takes zero arguments" and
/// "nobody configured `args` for it yet", and the two need opposite renderings: `()` for one,
/// a refusal for the other. Mirrors `ResultTypeName`'s shape in `c.rs` for the same reason --
/// the state that tells the two apart cannot be collapsed into a `bool` without losing the
/// case that must fail loudly. ~keep
///
/// `Known` is also the only state that can answer the *other* question a rendered argument
/// raises: whether the value's lowering matches the type of the parameter it lands in. An
/// argument list of the right length is not an argument list of the right types, and only a
/// resolved signature can tell those apart. ~keep
#[derive(Clone, Copy)]
pub(crate) enum TargetParams<'a> {
    /// The IR resolved a signature for the call's target (a free function, or a method every
    /// same-named IR method agrees on) -- these are its declared parameters, in order. An
    /// empty slice means the function is genuinely zero-argument.
    Known(&'a [crate::core::ir::ParamDef]),
    /// There is no core IR in scope at all, so nothing was consulted and nothing can be
    /// concluded. This is a legitimate, common state -- the main e2e test-file emitter has no
    /// `CallIr`, and several snippet entry points render without one -- so it keeps the
    /// pre-existing behaviour rather than refusing.
    ///
    /// Refusing here instead would fail every call on every IR-less path, which is a far larger
    /// blast radius than the defect being fixed, and it would contradict the sibling
    /// result-type resolution: `unresolved_result_type_name` treats an absent IR as
    /// `Unverified` for exactly this reason. The two halves of one fix must agree on what an
    /// absent IR licenses. ~keep
    IrAbsent,
    /// The IR was there to consult and the target still did not resolve -- an unresolvable name
    /// or disagreeing same-named methods. That is an authoring gap, and it is the case that
    /// produced a whole fixture `input` JSON spliced against a typed parameter, so it refuses.
    Unresolvable,
}

impl<'a> TargetParams<'a> {
    /// Resolve a call's declared parameters against the core IR for `language`.
    ///
    /// The lookup key is [`CallConfig::core_lookup_name`], the *Rust-side* identity: a
    /// per-language `overrides.<lang>.function` names the generated binding export
    /// (`samplellm_chat`, `chatAsync`), never the Rust function the IR indexes, so keying on it
    /// would miss every overridden call and answer [`Self::Unresolvable`] for calls the IR
    /// plainly knows. This is the same key `resolve_call_info` uses for result types, and the
    /// two must agree about which IR entry a call refers to. ~keep
    pub(crate) fn resolve(call: &CallConfig, language: &str, ir: CallIr<'a>) -> Self {
        if ir.is_absent() {
            return Self::IrAbsent;
        }
        let lookup_name = call.core_lookup_name(language);
        lookup_name
            .as_deref()
            .and_then(|name| ir.signature(name))
            .map_or(Self::Unresolvable, |signature| Self::Known(signature.params))
    }

    /// The declared parameters when one was resolved, else `None`.
    ///
    /// [`Self::IrAbsent`] and [`Self::Unresolvable`] learned nothing about the target, so they
    /// license no type claim -- a backend must fall back to its pre-IR lowering for both rather
    /// than treat "no parameters resolved" as "zero parameters declared". ~keep
    pub(crate) fn known(self) -> Option<&'a [crate::core::ir::ParamDef]> {
        match self {
            Self::Known(params) => Some(params),
            Self::IrAbsent | Self::Unresolvable => None,
        }
    }

    /// The declared parameter an `args` entry fills: by name, else positionally.
    ///
    /// Every backend must match parameters this way, because `resolve_call_info`'s
    /// `element_type` backfill in `c.rs` already does -- two rules would have two backends
    /// reasoning about different parameters for the same `args` entry. ~keep
    pub(crate) fn param_for(self, arg_name: &str, index: usize) -> Option<&'a crate::core::ir::ParamDef> {
        let params = self.known()?;
        params
            .iter()
            .find(|param| param.name == arg_name)
            .or_else(|| params.get(index))
    }

    /// The IR type name declared for the parameter an `args` entry fills, unwrapped through
    /// `Option`/`Vec`. `None` when the parameter is unresolved or its type names nothing (a
    /// primitive, a map, a tuple) -- in either case the backend has no named type to lower to
    /// and keeps its existing rendering.
    pub(crate) fn declared_type_name(self, arg_name: &str, index: usize) -> Option<&'a str> {
        named_type(&self.param_for(arg_name, index)?.ty)
    }
}

#[cfg(test)]
mod tests {
    use super::{CallIr, TargetParams, named_type};
    use crate::core::ir::{FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
    use crate::e2e::config::CallConfig;

    fn param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            ..ParamDef::default()
        }
    }

    fn function(name: &str, params: Vec<ParamDef>) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            params,
            return_type: TypeRef::Named("Response".to_string()),
            ..FunctionDef::default()
        }
    }

    fn call_named(function: &str) -> CallConfig {
        CallConfig {
            function: function.to_string(),
            ..CallConfig::default()
        }
    }

    #[test]
    fn resolves_declared_params_for_a_free_function() {
        let functions = vec![function(
            "complete",
            vec![param("request", TypeRef::Named("CompletionRequest".to_string()))],
        )];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let target = TargetParams::resolve(&call_named("complete"), "java", ir);
        assert_eq!(target.declared_type_name("request", 0), Some("CompletionRequest"));
    }

    /// The lookup must use the Rust identity, not the per-language export name: a `java`
    /// override naming `completeAsync` still resolves the IR's `complete`. ~keep
    #[test]
    fn resolves_through_a_per_language_function_override() {
        let functions = vec![function(
            "complete",
            vec![param("request", TypeRef::Named("CompletionRequest".to_string()))],
        )];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let mut call = call_named("complete");
        call.overrides.insert(
            "java".to_string(),
            crate::e2e::config::CallOverride {
                function: Some("completeAsync".to_string()),
                ..crate::e2e::config::CallOverride::default()
            },
        );
        let target = TargetParams::resolve(&call, "java", ir);
        assert_eq!(target.declared_type_name("request", 0), Some("CompletionRequest"));
    }

    /// A method declared on an IR type resolves too -- `ApiSurface::functions` alone answers
    /// `None` for every client method. ~keep
    #[test]
    fn resolves_a_method_declared_on_an_ir_type() {
        let type_defs = vec![TypeDef {
            name: "Client".to_string(),
            methods: vec![MethodDef {
                name: "chat".to_string(),
                params: vec![param("request", TypeRef::Named("ChatRequest".to_string()))],
                return_type: TypeRef::Named("ChatResponse".to_string()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }];
        let ir = CallIr {
            functions: &[],
            type_defs: &type_defs,
        };
        let target = TargetParams::resolve(&call_named("chat"), "swift", ir);
        assert_eq!(target.declared_type_name("request", 0), Some("ChatRequest"));
    }

    /// The state every IR-less caller depends on: no registries at all is `IrAbsent`, which
    /// answers `None` for the declared type so the backend keeps its pre-IR lowering. A
    /// `Known`-only fix would silently regress every one of those callers. ~keep
    #[test]
    fn an_absent_ir_is_ir_absent_and_licenses_no_type_claim() {
        let target = TargetParams::resolve(&call_named("complete"), "java", CallIr::default());
        assert!(matches!(target, TargetParams::IrAbsent));
        assert_eq!(target.declared_type_name("request", 0), None);
        assert!(target.known().is_none());
    }

    /// IR present, call not in it: distinct from `IrAbsent`, and the state that licenses a
    /// backend to refuse rather than splice. ~keep
    #[test]
    fn a_present_ir_missing_the_call_is_unresolvable() {
        let functions = vec![function("complete", vec![])];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let target = TargetParams::resolve(&call_named("mystery"), "java", ir);
        assert!(matches!(target, TargetParams::Unresolvable));
        assert_eq!(target.declared_type_name("request", 0), None);
    }

    /// Disagreeing same-named methods resolve to nothing rather than to an arbitrary winner.
    #[test]
    fn disagreeing_same_named_methods_are_unresolvable() {
        let type_defs = vec![
            TypeDef {
                name: "A".to_string(),
                methods: vec![MethodDef {
                    name: "new".to_string(),
                    params: vec![param("value", TypeRef::Named("Alpha".to_string()))],
                    return_type: TypeRef::Named("A".to_string()),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "B".to_string(),
                methods: vec![MethodDef {
                    name: "new".to_string(),
                    params: vec![param("value", TypeRef::Named("Beta".to_string()))],
                    return_type: TypeRef::Named("B".to_string()),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
        ];
        let ir = CallIr {
            functions: &[],
            type_defs: &type_defs,
        };
        assert!(matches!(
            TargetParams::resolve(&call_named("new"), "kotlin", ir),
            TargetParams::Unresolvable
        ));
    }

    /// A zero-argument target is `Known(&[])`, not `Unresolvable` -- the distinction the whole
    /// three-state shape exists to keep. ~keep
    #[test]
    fn a_zero_argument_target_is_known_and_empty() {
        let functions = vec![function("ping", vec![])];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let target = TargetParams::resolve(&call_named("ping"), "zig", ir);
        assert_eq!(target.known().map(<[ParamDef]>::len), Some(0));
    }

    /// Name match wins over position, and position is the fallback -- the rule
    /// `resolve_call_info`'s `element_type` backfill already follows. ~keep
    #[test]
    fn matches_a_param_by_name_before_position() {
        let functions = vec![function(
            "complete",
            vec![
                param("first", TypeRef::Named("Alpha".to_string())),
                param("second", TypeRef::Named("Beta".to_string())),
            ],
        )];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let target = TargetParams::resolve(&call_named("complete"), "csharp", ir);
        assert_eq!(target.declared_type_name("second", 0), Some("Beta"));
        assert_eq!(target.declared_type_name("unnamed", 1), Some("Beta"));
        assert_eq!(target.declared_type_name("unnamed", 9), None);
    }

    #[test]
    fn named_type_unwraps_option_and_vec() {
        let nested = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Model".to_string())))));
        assert_eq!(named_type(&nested), Some("Model"));
        assert_eq!(named_type(&TypeRef::String), None);
    }

    /// A declared primitive names no type, so a backend keeps its existing lowering rather
    /// than reading `None` as a refusal. ~keep
    #[test]
    fn a_primitive_param_declares_no_named_type() {
        let functions = vec![function(
            "scale",
            vec![param("factor", TypeRef::Primitive(PrimitiveType::F64))],
        )];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        let target = TargetParams::resolve(&call_named("scale"), "dart", ir);
        assert!(target.known().is_some());
        assert_eq!(target.declared_type_name("factor", 0), None);
    }
}
