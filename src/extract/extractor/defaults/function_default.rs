//! Folding a `#[serde(default = "path")]` field's [`DefaultValue::FunctionCall`] down to the
//! concrete value its named function's body computes, when that body is a single
//! constant-foldable expression.
//!
//! `helpers::fields::extract_field` records every `#[serde(default = "path")]` field as
//! `FunctionCall(path)` unconditionally — it never reads what `path` points to. That is correct
//! as a starting point: `path` may be a private function, may be feature-gated, or may compute
//! something no generated binding crate could reproduce. But the common real-world shape is
//! `fn default_page_number() -> u32 { 1 }` — a private, unconditional, literal-returning helper
//! next to the struct it defaults for — and refusing to read a value this plain forces every
//! `#[serde(default = "path")]` field in a struct with such a sibling to fail generation, even
//! though the value is right there in the same module.
//!
//! This pass runs the same constant-fold [`expr_to_default_value`] applies to `impl Default`
//! bodies over the *tail expression* of the named function's body, when that body is exactly one
//! statement. A body of more than one statement is refused outright, for the same reason
//! `mutation::read_struct_body` refuses anything but the narrow shapes it can prove: a second
//! statement could branch, early-return, or depend on something this pass has no way to see, and
//! guessing past that is the exact fabrication this module exists to avoid.
//!
//! Deliberately applied to a function regardless of its own visibility or whether it resolves to
//! a [`DefaultValue::PublicFunctionCall`] elsewhere (`postprocess::resolve_public_default_functions`,
//! which only recognizes public *associated methods*, never free functions). A folded literal
//! renders correctly in every target language via `codegen::config_gen::default_value_for_field`;
//! `PublicFunctionCall` only ever produces a real value for the "rust"-emitting backends (Magnus,
//! NAPI, PHP, Rustler, PyO3) that generate a Rust bridge and call the function directly — every
//! other language still renders `None`/`nil`/`null` for it. So when a function's value can be
//! proven at all, folding it is strictly more useful than leaving it a function call, public or
//! not: a literal is portable, a call is Rust-only. A function whose body cannot be proven keeps
//! today's behavior unchanged, `FunctionCall`/`PublicFunctionCall` and all. ~keep

use super::{ConstructorIndex, DefaultValue, EvalScope, FieldDef, TypeRef, expr_to_default_value, is_test_gated};
use ahash::AHashMap;

/// Every free (module-level) function declared directly in one module, keyed by name.
///
/// Scoped to a single module for the same reason [`super::collect_literal_consts`] and
/// [`super::collect_constructors`] are: `#[serde(default = "path")]` overwhelmingly names a
/// helper declared next to the struct it defaults for, and resolving a `use`-imported function
/// from another module would need a crate-wide index this pass does not build. `#[cfg(test)]`
/// functions are excluded — they do not exist in a normal build, so treating a test helper's
/// value as a normal build's default would be the same class of fabrication `mutation`'s
/// attributed-statement refusal exists to avoid. ~keep
pub(crate) type FreeFunctionIndex<'a> = AHashMap<String, &'a syn::ItemFn>;

pub(crate) fn collect_free_functions(items: &[syn::Item]) -> FreeFunctionIndex<'_> {
    let mut index = FreeFunctionIndex::new();
    for item in items {
        if let syn::Item::Fn(item_fn) = item
            && !is_test_gated(&item_fn.attrs)
        {
            index.insert(item_fn.sig.ident.to_string(), item_fn);
        }
    }
    index
}

/// Attempt to fold every [`DefaultValue::FunctionCall`] among `fields` down to the concrete
/// value its named function computes. A field whose function cannot be resolved or whose body
/// is not constant-foldable is left exactly as `extract_field` recorded it. ~keep
pub(crate) fn fold_constant_default_functions(
    fields: &mut [FieldDef],
    free_functions: &FreeFunctionIndex<'_>,
    constructors: &ConstructorIndex<'_>,
    literal_consts: &AHashMap<String, DefaultValue>,
) {
    for field in fields.iter_mut() {
        let Some(DefaultValue::FunctionCall(path)) = &field.typed_default else {
            continue;
        };
        if let Some(folded) = fold_function_call(path, &field.ty, free_functions, constructors, literal_consts) {
            field.typed_default = Some(folded);
        }
    }
}

/// Resolve `path` — a bare free-function name (`default_page_number`) or a path ending in
/// `Owner::method` (`Settings::default_retry_limit`, `crate::settings::Settings::default_retry_limit`)
/// — to the function it names, and fold that function's body.
///
/// An associated-function match is tried first: `[.., owner, method]` mirrors
/// `postprocess::resolve_public_default_functions`'s own path resolution, taking the last two
/// segments regardless of how many precede them. It is also final — see the check's own comment.
/// The free-function lookup is reached only when no associated function matched, and covers both
/// a truly bare path and a fully qualified free-function path whose leading segments this pass
/// cannot otherwise resolve. ~keep
fn fold_function_call(
    path: &str,
    field_ty: &TypeRef,
    free_functions: &FreeFunctionIndex<'_>,
    constructors: &ConstructorIndex<'_>,
    literal_consts: &AHashMap<String, DefaultValue>,
) -> Option<DefaultValue> {
    let segments: Vec<&str> = path.split("::").collect();
    let last = *segments.last()?;

    // Once the path resolves to an associated function, that IS the function it names, and its
    // fold result is the answer — including `None`. Falling through to the free-function lookup
    // below would try a *different* function that merely shares the last segment (a same-module
    // `fn default_retry` beside an unfoldable `Settings::default_retry`), and substitute its value
    // for one alef could not read, indistinguishably from a value it genuinely read. ~keep
    if let [.., owner, method] = segments.as_slice()
        && let Some(assoc_fn) = constructors.get(&((*owner).to_string(), (*method).to_string()))
    {
        return fold_zero_arg_function(
            owner,
            assoc_fn.sig.inputs.is_empty(),
            assoc_fn.sig.asyncness.is_some(),
            &assoc_fn.block,
            field_ty,
            literal_consts,
        );
    }

    let free_fn = free_functions.get(last)?;
    fold_zero_arg_function(
        "",
        free_fn.sig.inputs.is_empty(),
        free_fn.sig.asyncness.is_some(),
        &free_fn.block,
        field_ty,
        literal_consts,
    )
}

/// Fold a zero-argument function's body to a [`DefaultValue`], or `None` when the function takes
/// arguments, is `async` (serde's `default = "path"` requires a plain `fn() -> T`, so this shape
/// cannot be the real target and reading it would be a coincidence, not a resolution), or its
/// body is not a single constant-foldable statement.
fn fold_zero_arg_function(
    self_type: &str,
    inputs_empty: bool,
    is_async: bool,
    block: &syn::Block,
    field_ty: &TypeRef,
    literal_consts: &AHashMap<String, DefaultValue>,
) -> Option<DefaultValue> {
    if !inputs_empty || is_async {
        return None;
    }
    let tail = single_statement_tail(block)?;
    let field_types = AHashMap::new();
    let scope = EvalScope::new(self_type, literal_consts, &field_types);
    let value = expr_to_default_value(tail, &scope, Some(field_ty));
    is_known_value(&value).then_some(value)
}

/// The lone statement of a single-statement function body, unwrapped to the expression it
/// evaluates to: a bare tail expression (no semicolon) or `return expr;`.
///
/// Refuses any body with more than one statement outright, with no attempt to look past a
/// leading statement the way `mutation::read_mutated_body` does for `impl Default` — that reader
/// earns the extra reach by proving every leading statement is an unescaped, unattributed
/// mutation of one binding. A generic `#[serde(default = "path")]` helper carries no such
/// promise, so a second statement could be an early return, a `cfg`-gated branch, or a
/// side-effecting call this pass cannot see through; refusing the whole body is the safe
/// direction. ~keep
fn single_statement_tail(block: &syn::Block) -> Option<&syn::Expr> {
    let [stmt] = block.stmts.as_slice() else {
        return None;
    };
    match stmt {
        syn::Stmt::Expr(syn::Expr::Return(ret), _) => ret.expr.as_deref(),
        syn::Stmt::Expr(expr, None) => Some(expr),
        _ => None,
    }
}

/// True for every [`DefaultValue`] that represents a value alef actually knows, as opposed to
/// one that records the absence of a reading ([`DefaultValue::Unresolved`]) or an uncalled
/// function ([`DefaultValue::FunctionCall`] / [`DefaultValue::PublicFunctionCall`]).
///
/// Deliberately broader than `carries_value`, which gates a narrower question — "may this be
/// bound to a constructor parameter" — and excludes [`DefaultValue::Empty`] and
/// [`DefaultValue::None`] for that question's own reasons (see its doc comment). Both of those
/// variants *are* a fully known value here: `Empty` asserts the value is the field's own type
/// zero and `None` asserts it is literally `Option::None`, and every backend already renders
/// both correctly with no further context. Only the three variants excluded below assert
/// "unknown" rather than carrying one. ~keep
fn is_known_value(value: &DefaultValue) -> bool {
    !matches!(
        value,
        DefaultValue::Unresolved(_) | DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_)
    )
}
