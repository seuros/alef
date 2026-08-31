//! Reading a `fn default()` whose body builds its value through a mutable local binding.
//!
//! `fn default() -> Self { let mut p = Self { .. }; p.field = ..; p.list.push(..); p }` is an
//! ordinary way to spell a default, and the struct literal in the `let` is only the *starting*
//! value. Reading that literal and stopping — which is what the previous `find_struct_expr`
//! did, by scanning statements in reverse and returning the first struct literal it saw
//! anywhere, including in a `let` it had no reason to believe was returned — records the value
//! the type had before every mutation as if it were the value it has after them.
//!
//! That is worse than an approximation, because of what the neighbouring variants assert.
//! `DefaultValue::Empty` claims *"the default is exactly this type's zero"*, so a backend
//! substituting its own zero is exact; `IntLiteral(0)` claims the default is literally zero.
//! A `Vec::new()` that is pushed to before being returned is neither. Both are read by
//! per-field-literal backends that have no way to tell a read value from an unread one, so a
//! wrong-and-confident answer here ships silently, while [`DefaultValue::Unresolved`] fails
//! loudly through `cli::pipeline::generate::validation`.
//!
//! Hence the rule this module is built around: **read only what can be proven, and answer
//! `Unresolved` for everything else.** Every shape whose final value is not determined by
//! straight-line, locally-visible mutation of one binding — a branch, a loop, an early return,
//! a helper the binding is handed to, a method whose effect is not modelled — is refused, and
//! the refusal reaches the caller as a hard `Unresolved` over every field rather than as a
//! best guess.
//!
//! An *attributed* statement is refused for the same reason and belongs in the same list.
//! `#[cfg(feature = "x")] value.depth = 9;` runs only in builds that enable the feature, and
//! which features a consumer enables is not knowable from the source alef reads — so the
//! mutation's very existence is undetermined, not merely its value. Every attribute is refused,
//! not only `cfg`: `cfg_attr` expands to arbitrary attributes under the same unknown condition,
//! an attribute macro may rewrite or delete the statement outright, and an allowlist of
//! "inert" attributes is the part that silently goes stale. Refusing costs one loud
//! `Unresolved` on an exotic body; admitting costs silent wrong output in every build where
//! the cfg went the other way. ~keep

use super::{DefaultValue, EvalScope, carries_value, expr_to_default_value, struct_expr_defaults};
use ahash::AHashMap;
use quote::ToTokens;

/// A `fn default()` body reduced to the struct literal it returns, plus every mutation applied
/// to that literal before it is returned, in source order.
pub(super) struct StructBody<'a> {
    struct_expr: &'a syn::ExprStruct,
    mutations: Vec<FieldMutation<'a>>,
}

/// One proven mutation of one named field of the returned binding.
struct FieldMutation<'a> {
    field: String,
    kind: MutationKind<'a>,
    /// Source text of the whole statement, so an unreadable one names itself in the diagnostic.
    source: String,
}

enum MutationKind<'a> {
    Assign(&'a syn::Expr),
    Push(&'a syn::Expr),
    Extend(&'a syn::Expr),
    /// A mutation whose effect [`DefaultValue`] has no way to represent — `insert` on a map or
    /// a set, where the IR has no key/value-carrying variant at all. The field is known to be
    /// *changed* and not known to what, which is exactly `Unresolved`. Approximating a map as
    /// a `ListLiteral` of its values would drop the keys and render a default that differs
    /// from the Rust one. ~keep
    Opaque,
}

/// The struct literal a `fn default()` (or a constructor it delegates to) returns, together
/// with the mutations applied to it, or `None` when the body's final value cannot be proven.
///
/// `None` is the conservative answer and the caller turns it into `Unresolved`, so every shape
/// not explicitly proven below lands there by construction rather than by enumeration.
pub(super) fn read_struct_body(block: &syn::Block) -> Option<StructBody<'_>> {
    if let Some(struct_expr) = tail_struct_expr(block) {
        return Some(StructBody {
            struct_expr,
            mutations: Vec::new(),
        });
    }
    read_mutated_body(block)
}

/// Lower a body to one `DefaultValue` per field: the literal's initializers, then each proven
/// mutation applied over them in source order.
pub(super) fn struct_body_defaults(body: &StructBody<'_>, scope: &EvalScope<'_>) -> AHashMap<String, DefaultValue> {
    let mut defaults = struct_expr_defaults(body.struct_expr, scope);
    for mutation in &body.mutations {
        let field_ty = scope.field_types.get(&mutation.field);
        let updated = match &mutation.kind {
            MutationKind::Assign(value) => expr_to_default_value(value, scope, field_ty),
            MutationKind::Push(value) => {
                let element = expr_to_default_value(value, scope, field_ty);
                pushed(defaults.get(&mutation.field), element, &mutation.source)
            }
            MutationKind::Extend(value) => {
                let addition = expr_to_default_value(value, scope, field_ty);
                extended(defaults.get(&mutation.field), addition, &mutation.source)
            }
            MutationKind::Opaque => DefaultValue::Unresolved(mutation.source.clone()),
        };
        defaults.insert(mutation.field.clone(), updated);
    }
    defaults
}

/// A `push` is readable only when both halves are: the element folds to a real value, and the
/// field's value so far is a collection this pass actually read. Anything else — a computed
/// element, a field that was already unresolved, a scalar — leaves the field unresolved rather
/// than inventing a one-element list. ~keep
fn pushed(current: Option<&DefaultValue>, element: DefaultValue, source: &str) -> DefaultValue {
    if !carries_value(&element) {
        return DefaultValue::Unresolved(source.to_string());
    }
    match current {
        Some(DefaultValue::Empty) => DefaultValue::ListLiteral(vec![element]),
        Some(DefaultValue::ListLiteral(existing)) => {
            let mut elements = existing.clone();
            elements.push(element);
            DefaultValue::ListLiteral(elements)
        }
        _ => DefaultValue::Unresolved(source.to_string()),
    }
}

/// `extend` is `push` over a folded list. An argument that folds to `Empty` adds nothing, so
/// the field keeps whatever the literal gave it; an argument alef could not read (an iterator
/// chain, a call) makes the result unknown. ~keep
fn extended(current: Option<&DefaultValue>, addition: DefaultValue, source: &str) -> DefaultValue {
    let additions = match addition {
        DefaultValue::Empty => Vec::new(),
        DefaultValue::ListLiteral(elements) => elements,
        _ => return DefaultValue::Unresolved(source.to_string()),
    };
    match current {
        Some(DefaultValue::Empty) if additions.is_empty() => DefaultValue::Empty,
        Some(DefaultValue::Empty) => DefaultValue::ListLiteral(additions),
        Some(DefaultValue::ListLiteral(existing)) => {
            let mut elements = existing.clone();
            elements.extend(additions);
            DefaultValue::ListLiteral(elements)
        }
        _ => DefaultValue::Unresolved(source.to_string()),
    }
}

/// The struct literal a block evaluates to *as its tail expression*, looked through nested
/// blocks. Only the tail counts: an earlier statement is not what the function returns, and
/// treating one as if it were is the defect this module exists to close.
///
/// The tail is only *the* answer when nothing before it can return instead. A conditional early
/// return followed by a struct literal has two exits and the literal is one of them, so the
/// preceding statements are checked before the tail is believed. A macro is refused outright
/// here: its expansion is not parsed, so a `return` inside one is invisible to the scan, and a
/// statement this pass reads past yet cannot see through is exactly the shape it must refuse.
/// The mutated-binding reader does not inherit that macro refusal — it folds its statements'
/// value expressions rather than reading past them, so `list.extend(vec![..])` stays readable. ~keep
fn tail_struct_expr(block: &syn::Block) -> Option<&syn::ExprStruct> {
    let (tail, leading) = block.stmts.split_last()?;
    if is_attributed(tail) {
        return None;
    }
    if leading
        .iter()
        .any(|stmt| is_attributed(stmt) || contains_early_return(stmt) || contains_macro(stmt))
    {
        return None;
    }
    let syn::Stmt::Expr(expr, _) = tail else {
        return None;
    };
    unwrap_to_struct_expr(expr)
}

fn unwrap_to_struct_expr(expr: &syn::Expr) -> Option<&syn::ExprStruct> {
    match expr {
        syn::Expr::Struct(s) if s.attrs.is_empty() => Some(s),
        syn::Expr::Block(b) if b.attrs.is_empty() => tail_struct_expr(&b.block),
        _ => None,
    }
}

/// The `let mut binding = Self { .. }; <mutations>; binding` shape, with the block's statements
/// required to be exactly that and nothing else.
///
/// The strictness is the safety argument. Because the only statements admitted between the
/// binding and the tail are mutations of `binding.<field>` whose value expressions provably do
/// not mention `binding`, there is no statement left in which the binding could be aliased,
/// branched on, conditionally returned, or handed to a function. Every other body shape fails
/// one of these checks and becomes `Unresolved`. ~keep
fn read_mutated_body(block: &syn::Block) -> Option<StructBody<'_>> {
    let (tail, leading) = block.stmts.split_last()?;
    if is_attributed(tail) {
        return None;
    }
    // An attribute decides whether a statement exists in this build, and an early return decides
    // whether the ones after it run. Neither is answerable from source alone. ~keep
    if leading
        .iter()
        .any(|stmt| is_attributed(stmt) || contains_early_return(stmt))
    {
        return None;
    }
    let [first, mutation_stmts @ ..] = leading else {
        return None;
    };
    let (binding, struct_expr) = local_struct_binding(first)?;
    if !tail_returns_binding(tail, &binding) {
        return None;
    }
    let mut mutations = Vec::with_capacity(mutation_stmts.len());
    for stmt in mutation_stmts {
        mutations.push(classify_mutation(stmt, &binding)?);
    }
    Some(StructBody { struct_expr, mutations })
}

/// `let mut binding = Name { .. };` — the binding's name and the literal it starts from.
fn local_struct_binding(stmt: &syn::Stmt) -> Option<(String, &syn::ExprStruct)> {
    let syn::Stmt::Local(local) = stmt else {
        return None;
    };
    let init = local.init.as_ref()?;
    // `let ... else { .. }` is a branch, and its divergent arm is not read here. ~keep
    if init.diverge.is_some() {
        return None;
    }
    let syn::Expr::Struct(struct_expr) = init.expr.as_ref() else {
        return None;
    };
    // An attribute on the initializer itself sits inside the statement rather than at its front,
    // where [`is_attributed`] would see it. ~keep
    if !struct_expr.attrs.is_empty() {
        return None;
    }
    // `Self { a: 1, ..base() }` carries fields from a base this pass never saw, so the starting
    // value is already unknown and mutating it cannot make it known. ~keep
    if struct_expr.rest.is_some() {
        return None;
    }
    Some((binding_ident(&local.pat)?, struct_expr))
}

/// The single identifier a `let` pattern binds. A destructuring pattern, a `ref` binding, or an
/// `@` subpattern binds something other than the whole struct and is refused.
fn binding_ident(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(pat_ident) if pat_ident.by_ref.is_none() && pat_ident.subpat.is_none() => {
            Some(pat_ident.ident.to_string())
        }
        syn::Pat::Type(pat_type) => binding_ident(&pat_type.pat),
        _ => None,
    }
}

/// The block's tail must be the binding itself — bare, or spelled `return binding;`. Requiring
/// the returned value to be the binding the literal was read from is what rules out a body that
/// returns a *different* value than the one it built. ~keep
fn tail_returns_binding(stmt: &syn::Stmt, binding: &str) -> bool {
    let syn::Stmt::Expr(expr, _) = stmt else {
        return false;
    };
    let returned = match expr {
        syn::Expr::Return(ret) => match ret.expr.as_deref() {
            Some(inner) => inner,
            None => return false,
        },
        other => other,
    };
    matches!(returned, syn::Expr::Path(path) if path.qself.is_none() && path.path.is_ident(binding))
}

/// One statement between the binding and the tail, as a proven mutation — or `None`, which
/// refuses the whole body.
///
/// `None` covers everything not listed: a nested block, an `if`/`match`/loop, an early
/// `return`, a second `let`, a macro statement, a bare call the binding is passed to, a
/// compound assignment, and any method other than the three whose effect on a `DefaultValue`
/// is modelled. ~keep
fn classify_mutation<'a>(stmt: &'a syn::Stmt, binding: &str) -> Option<FieldMutation<'a>> {
    // A mutation is a statement. An expression without a semicolon would be the block's value,
    // and the tail has already been taken. ~keep
    let syn::Stmt::Expr(expr, Some(_)) = stmt else {
        return None;
    };
    let source = expr.to_token_stream().to_string();
    match expr {
        syn::Expr::Assign(assign) => {
            let field = binding_field(&assign.left, binding)?;
            reject_escape(&assign.right, binding)?;
            Some(FieldMutation {
                field,
                kind: MutationKind::Assign(&assign.right),
                source,
            })
        }
        syn::Expr::MethodCall(call) => {
            let field = binding_field(&call.receiver, binding)?;
            for argument in &call.args {
                reject_escape(argument, binding)?;
            }
            let arguments: Vec<&syn::Expr> = call.args.iter().collect();
            let kind = match (call.method.to_string().as_str(), arguments.as_slice()) {
                ("push", [value]) => MutationKind::Push(value),
                ("extend", [value]) => MutationKind::Extend(value),
                // Map and set `insert` both land here, as does `Vec::insert(index, value)`. ~keep
                ("insert", _) => MutationKind::Opaque,
                _ => return None,
            };
            Some(FieldMutation { field, kind, source })
        }
        _ => None,
    }
}

/// Whether a statement carries any attribute at all.
///
/// A rendered-token test rather than a `syn` field read, and the difference is the whole point:
/// `syn` does **not** park a statement attribute uniformly on the statement's own expression
/// node. For `#[cfg(feature = "x")] value.field = 9;` the attribute lands on the *left operand*
/// (`ExprAssign.left`, an `Expr::Field`) and `ExprAssign.attrs` is empty, while for
/// `#[cfg(..)] value.list.push(..)` it lands on `ExprMethodCall.attrs` as expected. Verified
/// against `syn` 3.0.4. A check written as `!assign.attrs.is_empty()` therefore examines
/// nothing for the assignment case — it reads clean on precisely the shape it exists to catch.
/// An attribute always renders first, so testing the rendered prefix is uniform across every
/// statement kind and independent of where the parser chose to hang it. ~keep
fn is_attributed(stmt: &syn::Stmt) -> bool {
    stmt.to_token_stream().to_string().trim_start().starts_with('#')
}

/// Whether a `return` that would exit `fn default()` appears anywhere in a statement.
///
/// A `return` inside a closure exits the closure, not the function, so closure bodies are
/// skipped rather than refused. ~keep
fn contains_early_return(stmt: &syn::Stmt) -> bool {
    struct Scan {
        found: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Scan {
        fn visit_expr_return(&mut self, _node: &'ast syn::ExprReturn) {
            self.found = true;
        }
        fn visit_expr_closure(&mut self, _node: &'ast syn::ExprClosure) {}
    }
    let mut scan = Scan { found: false };
    syn::visit::Visit::visit_stmt(&mut scan, stmt);
    scan.found
}

/// Whether a macro invocation appears anywhere in a statement.
///
/// A macro's token stream is not parsed, so [`contains_early_return`] cannot see through one.
/// Used only where a statement is read *past* rather than folded. ~keep
fn contains_macro(stmt: &syn::Stmt) -> bool {
    struct Scan {
        found: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Scan {
        fn visit_macro(&mut self, _node: &'ast syn::Macro) {
            self.found = true;
        }
    }
    let mut scan = Scan { found: false };
    syn::visit::Visit::visit_stmt(&mut scan, stmt);
    scan.found
}

/// `binding.field` and nothing else. A nested access (`binding.inner.field`) mutates a value
/// whose own shape this pass never read, and a tuple index has no field name to key by.
fn binding_field(expr: &syn::Expr, binding: &str) -> Option<String> {
    let syn::Expr::Field(field) = expr else {
        return None;
    };
    let syn::Expr::Path(path) = field.base.as_ref() else {
        return None;
    };
    if path.qself.is_some() || !path.path.is_ident(binding) {
        return None;
    }
    match &field.member {
        syn::Member::Named(ident) => Some(ident.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

/// Refuses a mutation whose value expression mentions the binding at all: `p.a = compute(&p)`
/// reads a partially-built value, and `p.v.push(p.seed)` makes the result depend on an ordering
/// this pass does not model. Naming the binding anywhere in a value position is the cheapest
/// sound proxy for "the binding escaped", and erring toward refusal is the intended direction.
fn reject_escape(expr: &syn::Expr, binding: &str) -> Option<()> {
    (!mentions_binding(expr, binding)).then_some(())
}

/// Whether an identifier token equal to `binding` appears anywhere in an expression, macro
/// bodies included.
///
/// Deliberately a token scan rather than an AST walk: `syn`'s visitors do not descend into
/// unparsed macro token streams, so `p.a = build!(p)` would walk clean while still reading the
/// binding. Splitting the rendered token text on non-identifier characters over-matches (a
/// string literal containing the name refuses the body) and never under-matches, which is the
/// safe direction for a check whose whole job is to refuse. ~keep
fn mentions_binding(expr: &syn::Expr, binding: &str) -> bool {
    expr.to_token_stream()
        .to_string()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|token| token == binding)
}
