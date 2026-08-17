use super::helpers::is_test_gated;
use crate::core::ir::{DefaultValue, FieldDef, TypeRef};
use ahash::AHashMap;
use quote::ToTokens;
use syn;

/// Every associated function of every inherent `impl` block in one module, keyed by
/// `(type name, function name)`.
///
/// Exists so [`extract_default_values`] can follow a `fn default()` that delegates to one of
/// its own constructors instead of spelling a struct literal. Scoped to a single module for
/// the same reason [`collect_literal_consts`] is: `impl Default` and the `fn new` it calls sit
/// next to each other in the overwhelmingly common case, and resolving a constructor from
/// another module would need a crate-wide index. ~keep
pub(crate) type ConstructorIndex<'a> = AHashMap<(String, String), &'a syn::ImplItemFn>;

/// How many `Self::a() -> Self::b() -> ...` hops [`extract_default_values`] will follow.
///
/// A bound rather than a visited-set because the bound is also the honest limit of the
/// technique: past a few hops the "constructor" is a pipeline, not a constant. It doubles as
/// the cycle guard, so `fn new() { Self::fresh() }` / `fn fresh() { Self::new() }` terminates
/// instead of recursing forever. ~keep
const MAX_DELEGATION_DEPTH: usize = 4;

/// Extract concrete default values from an `impl Default for T` block.
///
/// Finds the `fn default() -> Self` method and reads its body one of two ways:
///
/// 1. a struct literal (`Self { field: expr, ... }`), each initializer lowered to a
///    [`DefaultValue`]; or
/// 2. a delegation to one of `T`'s own constructors (`Self::new("en")`), whose parameters are
///    bound to the literal arguments the delegation passed and whose struct literal is then
///    read against that binding.
///
/// A body that is neither writes [`DefaultValue::Unresolved`] to every field. That is
/// **not** the same as [`DefaultValue::Empty`]: `Empty` claims the default *is* the type's
/// zero, `Unresolved` records that alef could not read it. Collapsing the two is what let six
/// backends emit their type-zero underneath a doc comment quoting the real Rust value; see
/// [`DefaultValue::Unresolved`] and `cli::pipeline::generate::validation`, which refuses to
/// generate rather than guess. ~keep
///
/// `literal_consts` resolves a field initializer that references a sibling
/// `const NAME: T = <literal>;` declared in the same module (e.g. `NAME`, `NAME.to_string()`,
/// or `Type::NAME`) to that constant's actual value. See [`collect_literal_consts`].
pub(crate) fn extract_default_values(
    item: &syn::ItemImpl,
    self_type: &str,
    fields: &mut [FieldDef],
    literal_consts: &AHashMap<String, DefaultValue>,
    constructors: &ConstructorIndex<'_>,
) {
    let default_fn = item.items.iter().find_map(|impl_item| {
        if let syn::ImplItem::Fn(method) = impl_item
            && method.sig.ident == "default"
        {
            return Some(method);
        }
        None
    });

    let Some(default_fn) = default_fn else {
        mark_unresolved(fields, "impl Default block without a `fn default()` item");
        return;
    };

    // The declared type of each field, so a two-segment path initializer can be checked against
    // it before being lowered to `DefaultValue::EnumVariant`. See [`admits_enum_variant`]. ~keep
    let field_types: AHashMap<String, TypeRef> = fields
        .iter()
        .map(|field| (field.name.clone(), field.ty.clone()))
        .collect();
    let scope = EvalScope::new(self_type, literal_consts, &field_types);

    let defaults = if let Some(struct_expr) = find_struct_expr(&default_fn.block) {
        struct_expr_defaults(struct_expr, &scope)
    } else if let Some(delegated) = follow_delegation(&default_fn.block, self_type, constructors, &scope, 0) {
        delegated
    } else {
        let body = default_fn.block.to_token_stream().to_string();
        tracing::warn!(
            target: "alef::extract::defaults",
            rust_type = self_type,
            body = %body,
            "`impl Default` body is neither a struct literal nor a constant-foldable delegation; \
             field defaults are unresolved"
        );
        mark_unresolved(fields, &body);
        return;
    };

    for field in fields.iter_mut() {
        if let Some(default_val) = defaults.get(&field.name) {
            field.typed_default = Some(default_val.clone());
        } else {
            field.typed_default = Some(DefaultValue::Empty);
        }
    }
}

fn mark_unresolved(fields: &mut [FieldDef], body: &str) {
    for field in fields.iter_mut() {
        field.typed_default = Some(DefaultValue::Unresolved(body.to_string()));
    }
}

/// Collects the associated (receiver-less) functions of every inherent `impl` block in
/// `items`, so a delegating `fn default()` can be followed to the constructor it calls.
///
/// Trait impls are skipped: `impl Default for T` is the caller, and no other trait method is
/// a plausible delegation target. Methods with a `self` receiver are skipped because
/// `Self::name(..)` in a `fn default()` cannot reach one. ~keep
pub(crate) fn collect_constructors(items: &[syn::Item]) -> ConstructorIndex<'_> {
    let mut index = ConstructorIndex::new();
    for item in items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        // A `#[cfg(test)]` impl block is not part of the binding surface — `extractor::mod` skips
        // it when extracting — so a test-only `fn new` must not shadow the real constructor. ~keep
        if item_impl.trait_.is_some() || is_test_gated(&item_impl.attrs) {
            continue;
        }
        let Some(type_name) = path_type_name(&item_impl.self_ty) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if matches!(method.sig.inputs.first(), Some(syn::FnArg::Receiver(_))) {
                continue;
            }
            index.insert((type_name.clone(), method.sig.ident.to_string()), method);
        }
    }
    index
}

fn path_type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// Collects every literal-valued `const` visible to an `impl Default` in the same module, so
/// [`extract_default_values`] can resolve a field initializer that references one instead of
/// collapsing it to `DefaultValue::Empty`.
///
/// Both module-level `const NAME: T = <literal>;` (keyed by the bare identifier) and associated
/// consts of inherent `impl` blocks (keyed `Type::NAME`) are collected. The associated-const key
/// carries the owning type because two types in one module may each declare `const DEFAULT`, and
/// a bare `Self` key would let one silently answer for the other. ~keep
///
/// Every literal kind is collected, not only `&str`. `max_pages_ceiling: DEFAULT_MAX_PAGES`
/// against `const DEFAULT_MAX_PAGES: usize = 500;` is the dominant shape of unreadable field
/// default in the consumer crates, and a numeric const is exactly as readable as a string one —
/// alef was rendering `0` for several of these, underneath a doc comment quoting the real value.
/// A const whose initializer is not a literal (`Duration::from_secs(5)`, a `concat!`) stays out:
/// evaluating it would be interpretation, not reading. ~keep
///
/// Deliberately scoped to the items of a single module/file: `impl Default` and
/// the const it references are the overwhelmingly common shape (`refresh.rs`'s
/// `DEFAULT_CATALOG_URL` alongside `CatalogRefreshConfig`'s `impl Default`), and
/// resolving a `use`-imported const from another module would need a full
/// crate-wide const index. ~keep
pub(crate) fn collect_literal_consts(items: &[syn::Item]) -> AHashMap<String, DefaultValue> {
    let mut consts = AHashMap::new();
    for item in items {
        match item {
            syn::Item::Const(item_const) => {
                if let Some(value) = const_literal_value(&item_const.expr) {
                    consts.insert(item_const.ident.to_string(), value);
                }
            }
            // Trait impls are skipped for the same reason [`collect_constructors`] skips them,
            // and a `#[cfg(test)]` impl is not part of the binding surface. ~keep
            syn::Item::Impl(item_impl) if item_impl.trait_.is_none() && !is_test_gated(&item_impl.attrs) => {
                let Some(type_name) = path_type_name(&item_impl.self_ty) else {
                    continue;
                };
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Const(assoc_const) = impl_item
                        && let Some(value) = const_literal_value(&assoc_const.expr)
                    {
                        consts.insert(format!("{type_name}::{}", assoc_const.ident), value);
                    }
                }
            }
            _ => {}
        }
    }
    consts
}

/// The value of a `const NAME: T = <literal>;`, or `None` for any other const initializer.
fn const_literal_value(expr: &syn::Expr) -> Option<DefaultValue> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(s) => Some(DefaultValue::StringLiteral(s.value())),
            syn::Lit::Char(c) => Some(DefaultValue::StringLiteral(c.value().to_string())),
            syn::Lit::Bool(b) => Some(DefaultValue::BoolLiteral(b.value)),
            syn::Lit::Int(i) => i.base10_parse::<i64>().ok().map(DefaultValue::IntLiteral),
            syn::Lit::Float(f) => f.base10_parse::<f64>().ok().map(DefaultValue::FloatLiteral),
            _ => None,
        },
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => match const_literal_value(&unary.expr)? {
            DefaultValue::IntLiteral(v) => Some(DefaultValue::IntLiteral(-v)),
            DefaultValue::FloatLiteral(v) => Some(DefaultValue::FloatLiteral(-v)),
            _ => None,
        },
        _ => None,
    }
}

/// Everything a field initializer can be resolved against.
///
/// `literal_consts` is module-wide and constant. `params` is populated only while reading a
/// constructor's body on behalf of a delegating `fn default()`: it binds that constructor's
/// parameters to the literal arguments the delegation passed, which is what turns
/// `fn default() { Self::new("en") }` plus `fn new(lang: &str) { Self { lang: lang.into(), .. } }`
/// into `lang = "en"` rather than a guess. ~keep
struct EvalScope<'a> {
    /// The type whose `impl Default` is being read, so `Self::NAME` can be resolved against the
    /// `Type::NAME`-keyed associated consts in `literal_consts`. ~keep
    self_type: &'a str,
    literal_consts: &'a AHashMap<String, DefaultValue>,
    /// Declared type per field name, used only to decide whether a two-segment path initializer
    /// can be an enum variant. Empty while reading a constructor body on behalf of a delegating
    /// `fn default()` would be wrong, so it is carried across `with_params` unchanged. ~keep
    field_types: &'a AHashMap<String, TypeRef>,
    params: AHashMap<String, DefaultValue>,
}

impl<'a> EvalScope<'a> {
    fn new(
        self_type: &'a str,
        literal_consts: &'a AHashMap<String, DefaultValue>,
        field_types: &'a AHashMap<String, TypeRef>,
    ) -> Self {
        Self {
            self_type,
            literal_consts,
            field_types,
            params: AHashMap::new(),
        }
    }

    fn with_params(&self, params: AHashMap<String, DefaultValue>) -> EvalScope<'a> {
        EvalScope {
            self_type: self.self_type,
            literal_consts: self.literal_consts,
            field_types: self.field_types,
            params,
        }
    }

    /// Resolves `Owner::NAME` (and `Self::NAME`, against the type being read) to the value of an
    /// associated literal const declared in the same module.
    fn associated_const(&self, owner: &str, name: &str) -> Option<DefaultValue> {
        let owner = if owner == "Self" { self.self_type } else { owner };
        self.literal_consts.get(&format!("{owner}::{name}")).cloned()
    }
}

/// True for the `DefaultValue`s that carry an actual value, as opposed to recording the
/// absence of one. Only these may be bound to a constructor parameter: binding `Empty` or
/// `Unresolved` would substitute a guess into the callee's body and lose the very
/// distinction this module exists to keep. ~keep
fn carries_value(value: &DefaultValue) -> bool {
    matches!(
        value,
        DefaultValue::BoolLiteral(_)
            | DefaultValue::StringLiteral(_)
            | DefaultValue::IntLiteral(_)
            | DefaultValue::FloatLiteral(_)
            | DefaultValue::EnumVariant(_)
            | DefaultValue::ListLiteral(_)
    )
}

/// Follow a `fn default()` whose body is a call to one of the type's own associated
/// functions — `Self::new("en")`, `PaddleOcrConfig::for_language("en")` — and read the
/// callee's struct literal with its parameters bound to the arguments passed.
///
/// This is a constant fold, not an interpreter, and the boundary is deliberate. A callee that
/// computes a field (`side_len: base * scale_for(lang)`), branches on its argument, or builds
/// through a builder is not followed; those fields stay unresolved and get reported rather
/// than guessed. The technique covers the shape it was written for — a constructor taking
/// literal arguments and returning a struct literal — and nothing beyond it. ~keep
fn follow_delegation(
    block: &syn::Block,
    self_type: &str,
    constructors: &ConstructorIndex<'_>,
    scope: &EvalScope<'_>,
    depth: usize,
) -> Option<AHashMap<String, DefaultValue>> {
    if depth >= MAX_DELEGATION_DEPTH {
        return None;
    }

    let call = tail_call_expr(block)?;
    let syn::Expr::Path(path) = &*call.func else {
        return None;
    };
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    // `Self::new(..)` and `PaddleOcrConfig::new(..)` name the same function. A longer path
    // leaves the module `constructors` indexes, so it cannot be resolved here. ~keep
    let [owner, fn_name] = segments.as_slice() else {
        return None;
    };
    if owner.as_str() != "Self" && owner.as_str() != self_type {
        return None;
    }
    // `Self::default()` inside `fn default()` is unbounded recursion in the source itself.
    if fn_name.as_str() == "default" {
        return None;
    }

    let target = constructors.get(&(self_type.to_string(), fn_name.clone()))?;

    let mut params = AHashMap::new();
    let mut arguments = call.args.iter();
    for input in &target.sig.inputs {
        let syn::FnArg::Typed(pat_type) = input else {
            return None;
        };
        let argument = arguments.next()?;
        let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            continue;
        };
        let value = expr_to_default_value(argument, scope, None);
        if carries_value(&value) {
            params.insert(pat_ident.ident.to_string(), value);
        }
    }
    // An arity mismatch means the index resolved the wrong function (or the source does not
    // compile); either way, reading its body would invent values. ~keep
    if arguments.next().is_some() {
        return None;
    }

    let inner = scope.with_params(params);
    if let Some(struct_expr) = find_struct_expr(&target.block) {
        return Some(struct_expr_defaults(struct_expr, &inner));
    }
    follow_delegation(&target.block, self_type, constructors, &inner, depth + 1)
}

/// The tail expression of a block, unwrapped to a call. Only the tail is considered: an
/// earlier statement is not what the function returns.
fn tail_call_expr(block: &syn::Block) -> Option<&syn::ExprCall> {
    match block.stmts.last()? {
        syn::Stmt::Expr(expr, _) => unwrap_to_call_expr(expr),
        _ => None,
    }
}

fn unwrap_to_call_expr(expr: &syn::Expr) -> Option<&syn::ExprCall> {
    match expr {
        syn::Expr::Call(call) => Some(call),
        syn::Expr::Block(b) => tail_call_expr(&b.block),
        syn::Expr::Return(ret) => ret.expr.as_deref().and_then(unwrap_to_call_expr),
        _ => None,
    }
}

/// Map each initializer of a struct literal to a `DefaultValue`.
fn struct_expr_defaults(struct_expr: &syn::ExprStruct, scope: &EvalScope<'_>) -> AHashMap<String, DefaultValue> {
    let mut defaults = AHashMap::new();
    for field in &struct_expr.fields {
        let Some(ident) = &field.member_named() else {
            continue;
        };
        let name = ident.to_string();
        let value = expr_to_default_value(&field.expr, scope, scope.field_types.get(&name));
        if let DefaultValue::Unresolved(source) = &value {
            tracing::debug!(
                target: "alef::extract::defaults",
                rust_type = scope.self_type,
                field = %name,
                initializer = %source,
                "field initializer is not constant-foldable; its default is unresolved"
            );
        }
        defaults.insert(name, value);
    }
    defaults
}

/// Recursively search a block for a struct expression (`Self { ... }` or `Name { ... }`).
fn find_struct_expr(block: &syn::Block) -> Option<&syn::ExprStruct> {
    for stmt in block.stmts.iter().rev() {
        match stmt {
            syn::Stmt::Expr(expr, _) => {
                if let Some(s) = unwrap_to_struct_expr(expr) {
                    return Some(s);
                }
            }
            syn::Stmt::Local(local) => {
                if let Some(init) = &local.init
                    && let Some(s) = unwrap_to_struct_expr(&init.expr)
                {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

/// Try to unwrap an expression to a struct expression, looking through blocks.
fn unwrap_to_struct_expr(expr: &syn::Expr) -> Option<&syn::ExprStruct> {
    match expr {
        syn::Expr::Struct(s) => Some(s),
        syn::Expr::Block(b) => find_struct_expr(&b.block),
        _ => None,
    }
}

/// Helper trait to extract the named member from a `FieldValue`.
trait FieldMemberExt {
    fn member_named(&self) -> Option<&syn::Ident>;
}

impl FieldMemberExt for syn::FieldValue {
    fn member_named(&self) -> Option<&syn::Ident> {
        match &self.member {
            syn::Member::Named(ident) => Some(ident),
            syn::Member::Unnamed(_) => None,
        }
    }
}

/// Records that this initializer could not be read, carrying its source text so the diagnostic
/// in `cli::pipeline::generate::validation` can name the expression it refused.
fn unreadable(expr: &syn::Expr) -> DefaultValue {
    DefaultValue::Unresolved(expr.to_token_stream().to_string())
}

/// Whether a field of this declared type could hold an enum variant.
///
/// `Expr::Path` with two segments is ambiguous in Rust source: `Mode::Fast` names an enum
/// variant, `Self::DEFAULT_MODEL` names an associated const, `Duration::ZERO` names an
/// associated const of a struct. Lowering all three to [`DefaultValue::EnumVariant`] let
/// `codegen::config_gen::shared` render the *snake-cased variant name* as a string literal
/// whenever the field's type was `String`, so `model: Self::DEFAULT_MODEL` shipped as
/// `"default_model"` — a value that appears nowhere in the source crate and looks entirely
/// plausible in generated output.
///
/// `Named` is admitted rather than checked against an enum index because the enum is frequently
/// declared in a different module from the `impl Default` that names one of its variants, and
/// this pass is module-scoped by construction (see [`collect_literal_consts`]). Every non-`Named`
/// type is refused, which is where the fabrication lived.
///
/// `None` means the expression is not in a field position — a constructor argument being bound
/// by [`follow_delegation`], where no declared type is in reach — and keeps the prior reading. ~keep
fn admits_enum_variant(field_ty: Option<&TypeRef>) -> bool {
    match field_ty {
        None | Some(TypeRef::Named(_)) => true,
        Some(TypeRef::Optional(inner) | TypeRef::Vec(inner)) => admits_enum_variant(Some(&**inner)),
        Some(_) => false,
    }
}

/// Convert an expression to a `DefaultValue`.
///
/// `field_ty` is the declared type of the field being initialized, or `None` where the
/// expression is not a field initializer. It is consulted only by [`admits_enum_variant`].
///
/// Recognizes:
/// - `true` / `false` → `BoolLiteral`
/// - Integer literals → `IntLiteral`
/// - Float literals → `FloatLiteral`
/// - `"str".to_string()`, `String::from("str")`, `"str".into()` → `StringLiteral`
/// - `String::new()` → `StringLiteral("")`
/// - `'c'` (char literal) → `StringLiteral("c")`
/// - `Vec::new()`, `vec![]` → `Empty`
/// - `SomeType::default()`, `Default::default()` → `Empty`
/// - `SomeEnum::Variant`, where the field's declared type can hold one → `EnumVariant("Variant")`
/// - `CONST_NAME.to_string()` / `.to_owned()` / `.into()`, or a bare `CONST_NAME`,
///   where `CONST_NAME` resolves via `scope.literal_consts` → the constant's value
/// - `Self::CONST_NAME` / `Type::CONST_NAME`, where the associated literal const is declared in
///   the same module → the constant's value
/// - a bare constructor parameter, or `param.to_string()` / `.to_owned()` / `.into()`, where
///   `param` is bound in `scope.params` → the value the delegation passed for it
/// - Anything else → [`DefaultValue::Unresolved`]
///
/// Note the last line. `Empty` is reserved for the initializers that are *known* to be the
/// type's zero — `Vec::new()`, `Default::default()`, `vec![]` — and asserts as much. Every
/// other shape records that alef could not read the value, which is the same distinction
/// [`extract_default_values`] draws for a whole unreadable `fn default()` body, applied one
/// level down. Before this, an unreadable initializer inside a readable struct literal wrote
/// `Empty` and every backend rendered its own type-zero for it. ~keep
fn expr_to_default_value(expr: &syn::Expr, scope: &EvalScope<'_>, field_ty: Option<&TypeRef>) -> DefaultValue {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Bool(b) => DefaultValue::BoolLiteral(b.value),
            syn::Lit::Int(i) => {
                if let Ok(val) = i.base10_parse::<i64>() {
                    DefaultValue::IntLiteral(val)
                } else {
                    unreadable(expr)
                }
            }
            syn::Lit::Float(f) => {
                if let Ok(val) = f.base10_parse::<f64>() {
                    DefaultValue::FloatLiteral(val)
                } else {
                    unreadable(expr)
                }
            }
            syn::Lit::Char(c) => DefaultValue::StringLiteral(c.value().to_string()),
            syn::Lit::Str(s) => DefaultValue::StringLiteral(s.value()),
            _ => unreadable(expr),
        },

        // `&"en"` and `&CONST` reach a constructor parameter unchanged; the reference is not
        // part of the value. Parentheses and macro-expansion groups are likewise not part of
        // the value, and refusing to see through them would refuse a readable `(0.5)`. ~keep
        syn::Expr::Reference(syn::ExprReference { expr: inner, .. })
        | syn::Expr::Paren(syn::ExprParen { expr: inner, .. })
        | syn::Expr::Group(syn::ExprGroup { expr: inner, .. }) => expr_to_default_value(inner, scope, field_ty),

        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            match expr_to_default_value(&unary.expr, scope, field_ty) {
                DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
                DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
                _ => unreadable(expr),
            }
        }

        syn::Expr::Binary(bin) => {
            let lhs = expr_to_default_value(&bin.left, scope, field_ty);
            let rhs = expr_to_default_value(&bin.right, scope, field_ty);
            match (lhs, rhs) {
                (DefaultValue::IntLiteral(a), DefaultValue::IntLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => a
                        .checked_add(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Sub(_) => a
                        .checked_sub(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Mul(_) => a
                        .checked_mul(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Div(_) if b != 0 => DefaultValue::IntLiteral(a / b),
                    syn::BinOp::Rem(_) if b != 0 => DefaultValue::IntLiteral(a % b),
                    syn::BinOp::Shl(_) if (0..63).contains(&b) => a
                        .checked_shl(b as u32)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or_else(|| unreadable(expr)),
                    syn::BinOp::Shr(_) if (0..63).contains(&b) => DefaultValue::IntLiteral(a >> (b as u32)),
                    syn::BinOp::BitOr(_) => DefaultValue::IntLiteral(a | b),
                    syn::BinOp::BitAnd(_) => DefaultValue::IntLiteral(a & b),
                    syn::BinOp::BitXor(_) => DefaultValue::IntLiteral(a ^ b),
                    _ => unreadable(expr),
                },
                (DefaultValue::FloatLiteral(a), DefaultValue::FloatLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => DefaultValue::FloatLiteral(a + b),
                    syn::BinOp::Sub(_) => DefaultValue::FloatLiteral(a - b),
                    syn::BinOp::Mul(_) => DefaultValue::FloatLiteral(a * b),
                    syn::BinOp::Div(_) if b != 0.0 => DefaultValue::FloatLiteral(a / b),
                    _ => unreadable(expr),
                },
                _ => unreadable(expr),
            }
        }

        syn::Expr::MethodCall(mc) => {
            let method_name = mc.method.to_string();
            match method_name.as_str() {
                "to_string" | "to_owned" | "into" => {
                    if let syn::Expr::Lit(lit) = &*mc.receiver
                        && let syn::Lit::Str(s) = &lit.lit
                    {
                        return DefaultValue::StringLiteral(s.value());
                    }
                    match resolve_ident(&mc.receiver, scope) {
                        // `.to_string()` / `.to_owned()` on a non-string is a *conversion*, so
                        // only a string receiver survives them unchanged. `.into()` is
                        // identity-preserving for every value kind alef can represent. ~keep
                        Some(value @ DefaultValue::StringLiteral(_)) => value,
                        Some(value) if method_name == "into" => value,
                        _ => unreadable(expr),
                    }
                }
                _ => unreadable(expr),
            }
        }

        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = &*call.func {
                let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();

                if (segments == ["Some"] || segments == ["Option", "Some"])
                    && call.args.len() == 1
                    && let Some(inner) = call.args.first()
                {
                    return expr_to_default_value(inner, scope, field_ty);
                }

                if segments == ["String", "from"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Str(s) = &lit.lit
                    {
                        return DefaultValue::StringLiteral(s.value());
                    }
                    if let Some(argument) = call.args.first()
                        && let Some(value @ DefaultValue::StringLiteral(_)) = resolve_ident(argument, scope)
                    {
                        return value;
                    }
                    return unreadable(expr);
                }

                if segments == ["String", "new"] && call.args.is_empty() {
                    return DefaultValue::StringLiteral(String::new());
                }

                // `Cow::Borrowed("")` carries the value in its argument; the `Cow` itself is a
                // representation the binding layer already erases (`FieldDef::core_wrapper`), so
                // reading through it is not a guess. ~keep
                if let [.., owner, variant] = segments.as_slice()
                    && owner == "Cow"
                    && matches!(variant.as_str(), "Borrowed" | "Owned")
                    && call.args.len() == 1
                    && let Some(inner) = call.args.first()
                {
                    // The boundary the erasure argument does not cross: it holds for a value alef
                    // actually read. `Cow::Owned(detect_language())` names a core-private function,
                    // and a binding that cannot call it would render the name as the default. ~keep
                    return match expr_to_default_value(inner, scope, field_ty) {
                        DefaultValue::Unresolved(_) | DefaultValue::FunctionCall(_) => unreadable(expr),
                        resolved => resolved,
                    };
                }

                // The one family of calls whose result really is the type's zero, so `Empty` is an
                // assertion here rather than a fallback. ~keep
                if segments.len() == 2 && segments[1] == "new" && call.args.is_empty() {
                    let type_name = &segments[0];
                    if matches!(
                        type_name.as_str(),
                        "Vec" | "HashMap" | "HashSet" | "BTreeMap" | "BTreeSet" | "AHashMap" | "AHashSet"
                    ) {
                        return DefaultValue::Empty;
                    }
                }

                if segments == ["Duration", "from_secs"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Int(i) = &lit.lit
                        && let Ok(val) = i.base10_parse::<i64>()
                    {
                        return DefaultValue::IntLiteral(val * 1000);
                    }
                    return unreadable(expr);
                }

                if segments == ["Duration", "from_millis"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Int(i) = &lit.lit
                        && let Ok(val) = i.base10_parse::<i64>()
                    {
                        return DefaultValue::IntLiteral(val);
                    }
                    return unreadable(expr);
                }

                // `T::default()` / `Default::default()` is the type's zero by definition. ~keep
                if segments.last().is_some_and(|s| s == "default") {
                    return DefaultValue::Empty;
                }

                if call.args.is_empty() {
                    return DefaultValue::FunctionCall(segments.join("::"));
                }
            }
            unreadable(expr)
        }

        syn::Expr::Path(path) => {
            // `resolve_ident` covers the *readable* paths, including the associated const that
            // makes `model: Self::DEFAULT_MODEL` a string rather than a variant named
            // `DEFAULT_MODEL`. A value alef can resolve always beats a classification it can
            // only infer, so this runs before the enum reading below. ~keep
            if let Some(value) = resolve_ident(expr, scope) {
                return value;
            }
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            // A variant may be named through any number of module segments
            // (`crate::types::ResultFormat::Unified`), and only the last one is the variant. A
            // single segment is a bare identifier `resolve_ident` already failed to bind. ~keep
            if segments.len() >= 2
                && admits_enum_variant(field_ty)
                && let Some(name) = segments.last()
            {
                return DefaultValue::EnumVariant(name.clone());
            }
            unreadable(expr)
        }

        syn::Expr::Macro(mac) => {
            let macro_name = mac
                .mac
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if !matches!(macro_name.as_str(), "vec" | "hashmap" | "hashset") {
                return unreadable(expr);
            }
            // An empty collection macro really is the type's zero. ~keep
            if mac.mac.tokens.is_empty() {
                return DefaultValue::Empty;
            }
            // Only `vec!` is destructured. `hashmap!`/`hashset!` carry key-value and set
            // semantics `DefaultValue` cannot represent, so a populated one is unreadable rather
            // than flattened into a list that would render wrongly. ~keep
            if macro_name != "vec" {
                return unreadable(expr);
            }
            let Ok(elements) = mac
                .mac
                .parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
            else {
                // `vec![expr; N]` is not a comma list and fails to parse as one. ~keep
                return unreadable(expr);
            };
            if elements.is_empty() {
                return DefaultValue::Empty;
            }
            let mut lowered = Vec::with_capacity(elements.len());
            for element in &elements {
                let value = expr_to_default_value(element, scope, field_ty);
                // Only self-contained values may sit in an element position. A function-call
                // default cannot be evaluated at generation time, and `Empty`/`None`/
                // `Unresolved` carry no element value at all; any of them makes the whole
                // literal non-representable. Lowering a partial list would hand a backend a
                // default that silently differs from the Rust one. ~keep
                if !carries_value(&value) {
                    return unreadable(expr);
                }
                lowered.push(value);
            }
            DefaultValue::ListLiteral(lowered)
        }

        _ => unreadable(expr),
    }
}

/// Resolves a path expression against the scope: a bound constructor parameter first, then a
/// module-level string constant, then — for a two-segment path — an associated `&str` const of
/// the named type.
///
/// Parameters win because they are the narrower binding — inside a constructor body an
/// identifier that shadows a module const refers to the parameter. ~keep
///
/// The two-segment case has to live here rather than only in the `Expr::Path` arm of
/// [`expr_to_default_value`], because `Self::DEFAULT_MODEL.to_string()` reaches the extractor as
/// a *method call* whose receiver is the path, and the method-call arm resolves its receiver
/// through this function. ~keep
fn resolve_ident(expr: &syn::Expr, scope: &EvalScope<'_>) -> Option<DefaultValue> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    match segments.as_slice() {
        [ident] => {
            if let Some(value) = scope.params.get(ident) {
                return Some(value.clone());
            }
            scope.literal_consts.get(ident).cloned()
        }
        [.., owner, name] => scope.associated_const(owner, name),
        [] => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_value_of(expr_src: &str) -> DefaultValue {
        default_value_of_with_consts(expr_src, &[])
    }

    fn default_value_of_with_consts(expr_src: &str, consts: &[(&str, &str)]) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        let literal_consts: AHashMap<String, DefaultValue> = consts
            .iter()
            .map(|(k, v)| (k.to_string(), DefaultValue::StringLiteral(v.to_string())))
            .collect();
        let field_types = AHashMap::new();
        expr_to_default_value(&expr, &EvalScope::new("Subject", &literal_consts, &field_types), None)
    }

    #[test]
    fn some_int_literal_unwraps_to_inner_int() {
        assert_eq!(
            default_value_of("Some(50 * 1024 * 1024)"),
            DefaultValue::IntLiteral(52_428_800)
        );
    }

    #[test]
    fn some_string_literal_unwraps_to_inner_string() {
        assert_eq!(
            default_value_of(r#"Some("hi".to_string())"#),
            DefaultValue::StringLiteral("hi".to_string())
        );
    }

    #[test]
    fn qualified_option_some_unwraps() {
        assert_eq!(default_value_of("Option::Some(5)"), DefaultValue::IntLiteral(5));
    }

    #[test]
    fn bare_none_stays_none() {
        assert_eq!(default_value_of("None"), DefaultValue::None);
    }

    #[test]
    fn zero_argument_function_call_preserves_its_path() {
        assert_eq!(
            default_value_of("defaults::retry_limit()"),
            DefaultValue::FunctionCall("defaults::retry_limit".to_string())
        );
    }

    #[test]
    fn const_to_string_resolves_to_the_consts_literal_value() {
        assert_eq!(
            default_value_of_with_consts(
                "DEFAULT_CATALOG_URL.to_string()",
                &[("DEFAULT_CATALOG_URL", "https://example.com/catalog.json")]
            ),
            DefaultValue::StringLiteral("https://example.com/catalog.json".to_string())
        );
    }

    #[test]
    fn const_into_resolves_to_the_consts_literal_value() {
        assert_eq!(
            default_value_of_with_consts("HOST.into()", &[("HOST", "localhost")]),
            DefaultValue::StringLiteral("localhost".to_string())
        );
    }

    #[test]
    fn bare_const_path_resolves_to_the_consts_literal_value() {
        assert_eq!(
            default_value_of_with_consts("HOST", &[("HOST", "localhost")]),
            DefaultValue::StringLiteral("localhost".to_string())
        );
    }

    #[test]
    fn unresolvable_const_reference_is_unresolved_not_empty() {
        // No matching entry in `literal_consts`: alef does not know the value. `Empty` would
        // assert the default *is* the empty string, which for a const named `UNKNOWN_CONST` it
        // demonstrably is not.
        assert!(
            matches!(
                default_value_of("UNKNOWN_CONST.to_string()"),
                DefaultValue::Unresolved(_)
            ),
            "an unresolvable const reference must be reported, not silently zeroed"
        );
    }

    #[test]
    fn collect_literal_consts_collects_every_literal_kind_and_nothing_computed() {
        let file: syn::File = syn::parse_str(
            r#"
                pub const DEFAULT_CATALOG_URL: &str = "https://example.com/catalog.json";
                const CACHE_DIR_NAME: &str = "sample-crate";
                const RETRY_LIMIT: u32 = 3;
                const DET_DB_THRESH: f32 = 0.3;
                const VERBOSE: bool = false;
                const MIN_OFFSET: i32 = -5;
                const COMPUTED: &str = some_fn();
                const WINDOW: Duration = Duration::from_secs(5);
            "#,
        )
        .expect("valid file");

        let consts = collect_literal_consts(&file.items);

        assert_eq!(
            consts.get("DEFAULT_CATALOG_URL"),
            Some(&DefaultValue::StringLiteral(
                "https://example.com/catalog.json".to_string()
            ))
        );
        assert_eq!(
            consts.get("CACHE_DIR_NAME"),
            Some(&DefaultValue::StringLiteral("sample-crate".to_string()))
        );
        // A numeric const is exactly as readable as a string one, and leaving it out made alef
        // render `0` for the single most common unreadable-default shape in the consumer crates.
        assert_eq!(consts.get("RETRY_LIMIT"), Some(&DefaultValue::IntLiteral(3)));
        assert_eq!(consts.get("DET_DB_THRESH"), Some(&DefaultValue::FloatLiteral(0.3)));
        assert_eq!(consts.get("VERBOSE"), Some(&DefaultValue::BoolLiteral(false)));
        assert_eq!(consts.get("MIN_OFFSET"), Some(&DefaultValue::IntLiteral(-5)));
        assert_eq!(
            consts.get("COMPUTED"),
            None,
            "non-literal initializers must not be collected"
        );
        assert_eq!(
            consts.get("WINDOW"),
            None,
            "evaluating a const-fn initializer would be interpretation, not reading"
        );
    }

    /// Drive the whole extractor over a module source, returning the resolved defaults for
    /// the named type's `impl Default`. Reproduces exactly what `extractor::mod` does: build
    /// the const and constructor indexes from the module's items, then read the `impl Default`
    /// against them.
    fn defaults_for(source: &str, type_name: &str, field_names: &[&str]) -> Vec<(String, DefaultValue)> {
        let fields: Vec<(&str, TypeRef)> = field_names.iter().map(|name| (*name, TypeRef::Unit)).collect();
        defaults_for_typed(source, type_name, &fields)
    }

    /// As [`defaults_for`], but with each field's declared type spelled out. Only the two-segment
    /// path lowering consults it, so every other case can keep using the untyped helper. ~keep
    fn defaults_for_typed(source: &str, type_name: &str, fields: &[(&str, TypeRef)]) -> Vec<(String, DefaultValue)> {
        let file: syn::File = syn::parse_str(source).expect("valid module source");
        let literal_consts = collect_literal_consts(&file.items);
        let constructors = collect_constructors(&file.items);

        let default_impl = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(item_impl)
                    if item_impl
                        .trait_
                        .as_ref()
                        .is_some_and(|(path, _)| path.segments.last().is_some_and(|s| s.ident == "Default"))
                        && path_type_name(&item_impl.self_ty).as_deref() == Some(type_name) =>
                {
                    Some(item_impl)
                }
                _ => None,
            })
            .expect("module declares `impl Default` for the type");

        let mut fields: Vec<FieldDef> = fields
            .iter()
            .map(|(name, ty)| FieldDef {
                name: (*name).to_string(),
                ty: ty.clone(),
                ..Default::default()
            })
            .collect();

        extract_default_values(default_impl, type_name, &mut fields, &literal_consts, &constructors);

        fields
            .into_iter()
            .map(|field| {
                let value = field.typed_default.expect("every field is assigned a default");
                (field.name, value)
            })
            .collect()
    }

    /// The reported defect, reduced. `PaddleOcrConfig` really is
    /// `impl Default { fn default() -> Self { Self::new("en") } }`, and before this the
    /// extractor wrote `Empty` to all seven fields, which C#, Java, Kotlin, Swift, Python and
    /// Go each rendered as their own type-zero — `0.0f` for `det_db_thresh`, sitting under a
    /// generated doc comment reading "(default: 0.3)". ~keep
    #[test]
    fn a_default_delegating_to_a_constructor_recovers_the_constructors_literals() {
        let resolved = defaults_for(
            r#"
                pub struct PaddleOcrConfig {
                    pub language: String,
                    pub det_db_thresh: f32,
                    pub det_limit_side_len: u32,
                    pub use_angle_cls: bool,
                }

                impl PaddleOcrConfig {
                    pub fn new(language: &str) -> Self {
                        Self {
                            language: language.to_string(),
                            det_db_thresh: 0.3,
                            det_limit_side_len: 1024,
                            use_angle_cls: true,
                        }
                    }
                }

                impl Default for PaddleOcrConfig {
                    fn default() -> Self {
                        Self::new("en")
                    }
                }
            "#,
            "PaddleOcrConfig",
            &["language", "det_db_thresh", "det_limit_side_len", "use_angle_cls"],
        );

        assert_eq!(
            resolved,
            vec![
                ("language".to_string(), DefaultValue::StringLiteral("en".to_string())),
                ("det_db_thresh".to_string(), DefaultValue::FloatLiteral(0.3)),
                ("det_limit_side_len".to_string(), DefaultValue::IntLiteral(1024)),
                ("use_angle_cls".to_string(), DefaultValue::BoolLiteral(true)),
            ],
            "a delegating `fn default()` must yield the constructor's literals, never a type-zero"
        );
    }

    /// The same recovery through the type's own name rather than `Self`, and through a
    /// constructor whose parameter is consumed by `.into()` rather than `.to_string()`.
    #[test]
    fn a_delegation_named_by_the_type_and_consumed_by_into_also_recovers() {
        let resolved = defaults_for(
            r#"
                pub struct Client { pub endpoint: String, pub retries: u32 }

                impl Client {
                    pub fn for_endpoint(endpoint: &str) -> Self {
                        Self { endpoint: endpoint.into(), retries: 5 }
                    }
                }

                impl Default for Client {
                    fn default() -> Self {
                        Client::for_endpoint("https://api.example.com")
                    }
                }
            "#,
            "Client",
            &["endpoint", "retries"],
        );

        assert_eq!(
            resolved,
            vec![
                (
                    "endpoint".to_string(),
                    DefaultValue::StringLiteral("https://api.example.com".to_string())
                ),
                ("retries".to_string(), DefaultValue::IntLiteral(5)),
            ]
        );
    }

    /// A delegation whose argument is a module const, resolved through the same const index
    /// the direct path already uses.
    #[test]
    fn a_delegation_passing_a_module_const_resolves_it() {
        let resolved = defaults_for(
            r#"
                const DEFAULT_LANG: &str = "en";

                pub struct Ocr { pub language: String }

                impl Ocr {
                    pub fn new(language: &str) -> Self {
                        Self { language: language.to_string() }
                    }
                }

                impl Default for Ocr {
                    fn default() -> Self { Self::new(DEFAULT_LANG) }
                }
            "#,
            "Ocr",
            &["language"],
        );

        assert_eq!(
            resolved,
            vec![("language".to_string(), DefaultValue::StringLiteral("en".to_string()))]
        );
    }

    /// Two hops, which the delegation follower is bounded to allow.
    #[test]
    fn a_delegation_chained_through_a_second_constructor_still_resolves() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new() -> Self { Self::with_level(7) }
                    pub fn with_level(level: u32) -> Self { Self { level } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
            "Cfg",
            &["level"],
        );

        assert_eq!(resolved, vec![("level".to_string(), DefaultValue::IntLiteral(7))]);
    }

    /// A mutually recursive constructor pair must terminate rather than blow the stack, and
    /// must report the failure instead of inventing values.
    #[test]
    fn a_cyclic_delegation_terminates_and_reports_unresolved() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new() -> Self { Self::fresh() }
                    pub fn fresh() -> Self { Self::new() }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
            "Cfg",
            &["level"],
        );

        assert!(
            matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
            "a cycle must resolve to `Unresolved`, got {resolved:?}"
        );
    }

    /// The honest boundary of the technique, pinned so nobody mistakes the fold for an
    /// interpreter. A constructor that *computes* a field is not followed, and the outcome is
    /// `Unresolved` — reported — rather than a type-zero.
    #[test]
    fn a_default_delegating_to_a_builder_is_unresolved_not_a_type_zero() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn builder() -> CfgBuilder { CfgBuilder::new() }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::builder().level(9).build() }
                }
            "#,
            "Cfg",
            &["level"],
        );

        assert!(
            matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
            "an unfollowable body must be reported, not silently zeroed; got {resolved:?}"
        );
        assert_ne!(
            resolved[0].1,
            DefaultValue::Empty,
            "`Empty` would claim the default *is* the type-zero, which is the conflation this fixes"
        );
    }

    /// The direct path is untouched: a `fn default()` that spells its own struct literal still
    /// reads exactly as before, including the per-field `Empty` for an initializer that is
    /// genuinely the type's zero.
    #[test]
    fn a_struct_literal_default_is_unchanged_and_keeps_empty_for_genuine_zeros() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub level: u32, pub tags: Vec<String> }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { level: 3, tags: Vec::new() }
                    }
                }
            "#,
            "Cfg",
            &["level", "tags"],
        );

        assert_eq!(
            resolved,
            vec![
                ("level".to_string(), DefaultValue::IntLiteral(3)),
                ("tags".to_string(), DefaultValue::Empty),
            ]
        );
    }

    /// An arity mismatch means the constructor index resolved something other than the
    /// function actually called; reading its body would invent values.
    #[test]
    fn a_delegation_with_mismatched_arity_is_unresolved() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub level: u32 }

                impl Cfg {
                    pub fn new(level: u32, name: &str) -> Self { Self { level } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new(4) }
                }
            "#,
            "Cfg",
            &["level"],
        );

        assert!(
            matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
            "got {resolved:?}"
        );
    }

    /// A constructor parameter that is not a foldable literal must not be bound: binding a
    /// placeholder would put a guessed value in a field that reads the parameter. The field that
    /// *does* read it is reported unresolved rather than zeroed; its sibling is untouched.
    #[test]
    fn a_delegation_with_an_unfoldable_argument_reports_only_the_field_that_reads_it() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg { pub name: String, pub level: u32 }

                impl Cfg {
                    pub fn new(name: &str) -> Self { Self { name: name.to_string(), level: 2 } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new(compute_name()) }
                }
            "#,
            "Cfg",
            &["name", "level"],
        );

        assert!(
            matches!(
                resolved.as_slice(),
                [
                    (name, DefaultValue::Unresolved(_)),
                    (level, DefaultValue::IntLiteral(2)),
                ] if name == "name" && level == "level"
            ),
            "the unfoldable argument must not poison the sibling field it does not reach, and the \
             field it does reach must be reported rather than zeroed; got {resolved:?}"
        );
    }

    #[test]
    fn collect_constructors_indexes_associated_fns_and_skips_methods_and_trait_impls() {
        let file: syn::File = syn::parse_str(
            r#"
                impl Cfg {
                    pub fn new() -> Self { Self {} }
                    pub fn tweak(&self) -> Self { Self {} }
                }
                impl Default for Cfg {
                    fn default() -> Self { Self::new() }
                }
            "#,
        )
        .expect("valid file");

        let constructors = collect_constructors(&file.items);

        assert!(constructors.contains_key(&("Cfg".to_string(), "new".to_string())));
        assert!(
            !constructors.contains_key(&("Cfg".to_string(), "tweak".to_string())),
            "a `&self` method cannot be reached by `Self::name(..)` in `fn default()`"
        );
        assert!(
            !constructors.contains_key(&("Cfg".to_string(), "default".to_string())),
            "trait impls must not be indexed as constructors"
        );
    }

    /// The parameter binding must not leak past the constructor it belongs to: a module const
    /// with the same name as a parameter is shadowed inside the callee, and the parameter's
    /// bound value is the one that applies.
    #[test]
    fn a_constructor_parameter_shadows_a_module_const_of_the_same_name() {
        let resolved = defaults_for(
            r#"
                const language: &str = "shadowed";

                pub struct Cfg { pub language: String }

                impl Cfg {
                    pub fn new(language: &str) -> Self { Self { language: language.to_string() } }
                }

                impl Default for Cfg {
                    fn default() -> Self { Self::new("en") }
                }
            "#,
            "Cfg",
            &["language"],
        );

        assert_eq!(
            resolved,
            vec![("language".to_string(), DefaultValue::StringLiteral("en".to_string()))]
        );
    }

    /// What `codegen::config_gen::shared` writes into a generated binding for this field, which
    /// is where the fabrication was observable: an `EnumVariant` on a `String`-typed field is
    /// rendered as the *snake-cased variant name*, so `EnumVariant("DEFAULT_MODEL")` shipped the
    /// string `"default_model"` — a value that appears nowhere in the source crate. ~keep
    fn rendered_python_default(name: &str, ty: TypeRef, value: &DefaultValue) -> String {
        let field = FieldDef {
            name: name.to_string(),
            ty,
            typed_default: Some(value.clone()),
            ..Default::default()
        };
        crate::codegen::config_gen::default_value_for_field(&field, "python")
    }

    /// The reported defect. `model: Self::DEFAULT_MODEL` is a two-segment path just like
    /// `Mode::Fast`, and the extractor lowered both to `EnumVariant`. On a `String` field that
    /// rendered as `"default_model"`.
    ///
    /// The const is readable, so the honest answer is not "unresolved" but the const's own value.
    #[test]
    fn an_associated_const_default_on_a_string_field_resolves_to_the_consts_value() {
        let resolved = defaults_for_typed(
            r#"
                pub struct LlmConfig { pub model: String }

                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
            "LlmConfig",
            &[("model", TypeRef::String)],
        );

        assert_eq!(
            resolved,
            vec![(
                "model".to_string(),
                DefaultValue::StringLiteral("claude-sonnet-4-5".to_string())
            )]
        );
        assert_ne!(
            rendered_python_default("model", TypeRef::String, &resolved[0].1),
            "\"default_model\"",
            "the snake-cased const name is a fabricated value; it must not reach a binding"
        );
    }

    /// A bare `Self::CONST` — no `.to_string()` — takes the same route.
    #[test]
    fn a_bare_associated_const_path_resolves_through_the_owning_type() {
        let resolved = defaults_for_typed(
            r#"
                pub struct LlmConfig { pub base_url: String }

                impl LlmConfig {
                    const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { base_url: LlmConfig::DEFAULT_BASE_URL.into() }
                    }
                }
            "#,
            "LlmConfig",
            &[("base_url", TypeRef::String)],
        );

        assert_eq!(
            resolved,
            vec![(
                "base_url".to_string(),
                DefaultValue::StringLiteral("https://api.anthropic.com".to_string())
            )]
        );
    }

    /// The same shape with the const out of reach — declared in another module, or not a string
    /// literal at all. There is no value to recover, so the answer is `Unresolved`. What it must
    /// never be is an `EnumVariant`, because the field's declared type cannot hold one and the
    /// renderer would invent `"default_model"` from the const's name.
    #[test]
    fn an_unreachable_associated_const_on_a_string_field_is_unresolved_not_an_enum_variant() {
        let resolved = defaults_for_typed(
            r#"
                pub struct LlmConfig { pub model: String }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
            "LlmConfig",
            &[("model", TypeRef::String)],
        );

        let value = &resolved[0].1;
        assert!(
            matches!(value, DefaultValue::Unresolved(_)),
            "an unreadable initializer must be reported, got {value:?}"
        );
        assert_ne!(
            value,
            &DefaultValue::EnumVariant("DEFAULT_MODEL".to_string()),
            "a `String` field cannot hold an enum variant, so this lowering was never sound"
        );
        assert_ne!(
            rendered_python_default("model", TypeRef::String, value),
            "\"default_model\"",
            "the fabricated snake-cased const name must be absent from generated output"
        );
    }

    /// The control for the fix: a two-segment path on a field that really is enum-typed must
    /// still lower to `EnumVariant`. Breaking this would make every genuine enum default
    /// unresolved and arm the refusal across the fleet.
    #[test]
    fn a_genuine_enum_variant_default_still_lowers_to_an_enum_variant() {
        let resolved = defaults_for_typed(
            r#"
                pub struct Cfg { pub mode: Mode, pub fallback: Option<Mode>, pub stages: Vec<Mode> }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            mode: Mode::Fast,
                            fallback: Some(Mode::Slow),
                            stages: vec![Mode::Fast, Mode::Slow],
                        }
                    }
                }
            "#,
            "Cfg",
            &[
                ("mode", TypeRef::Named("Mode".to_string())),
                (
                    "fallback",
                    TypeRef::Optional(Box::new(TypeRef::Named("Mode".to_string()))),
                ),
                ("stages", TypeRef::Vec(Box::new(TypeRef::Named("Mode".to_string())))),
            ],
        );

        assert_eq!(
            resolved,
            vec![
                ("mode".to_string(), DefaultValue::EnumVariant("Fast".to_string())),
                ("fallback".to_string(), DefaultValue::EnumVariant("Slow".to_string())),
                (
                    "stages".to_string(),
                    DefaultValue::ListLiteral(vec![
                        DefaultValue::EnumVariant("Fast".to_string()),
                        DefaultValue::EnumVariant("Slow".to_string()),
                    ])
                ),
            ],
            "an enum-typed field — bare, optional or in a list — must keep its variant default"
        );
    }

    /// Two types in one module may each declare a const of the same name. Keying the index by
    /// the owning type is what stops one from answering for the other, which would substitute a
    /// value that is wrong rather than merely missing.
    #[test]
    fn an_associated_const_of_another_type_does_not_answer_for_this_one() {
        let resolved = defaults_for_typed(
            r#"
                pub struct Other { pub model: String }
                pub struct LlmConfig { pub model: String }

                impl Other {
                    pub const DEFAULT_MODEL: &str = "not-this-one";
                }

                impl Default for LlmConfig {
                    fn default() -> Self {
                        Self { model: Self::DEFAULT_MODEL.to_string() }
                    }
                }
            "#,
            "LlmConfig",
            &[("model", TypeRef::String)],
        );

        assert!(
            matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
            "a same-named const on a different type must not be substituted; got {resolved:?}"
        );
    }

    /// The field-granular half of the `Empty`/`Unresolved` split: an initializer alef cannot read
    /// inside an otherwise-readable struct literal. Each of these previously wrote `Empty`, which
    /// licensed every backend to emit its own type-zero for a value it had never read.
    #[test]
    fn an_unreadable_field_initializer_is_unresolved_not_empty() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg {
                    pub threshold: f32,
                    pub name: String,
                    pub root: PathBuf,
                    pub window: [u32; 2],
                    pub mode: u8,
                }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            threshold: compute().clamp(0.0, 1.0),
                            name: make_name(1, 2),
                            root: PathBuf::from("/tmp"),
                            window: [1, 2],
                            mode: if cfg!(unix) { 1 } else { 2 },
                        }
                    }
                }
            "#,
            "Cfg",
            &["threshold", "name", "root", "window", "mode"],
        );

        for (name, value) in &resolved {
            assert!(
                matches!(value, DefaultValue::Unresolved(_)),
                "`{name}` is not readable, so it must be reported rather than zeroed; got {value:?}"
            );
        }
    }

    /// The control that must survive the relabelling. `Empty` still means "the default *is* this
    /// type's zero", and these three initializers still assert exactly that. Widening
    /// `Unresolved` over them would arm the refusal on every crate in the fleet.
    #[test]
    fn genuine_type_zero_initializers_stay_empty() {
        let resolved = defaults_for(
            r#"
                pub struct Cfg {
                    pub tags: Vec<String>,
                    pub index: AHashMap<String, u32>,
                    pub count: u32,
                    pub stages: Vec<String>,
                }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self {
                            tags: Vec::new(),
                            index: AHashMap::new(),
                            count: u32::default(),
                            stages: vec![],
                        }
                    }
                }
            "#,
            "Cfg",
            &["tags", "index", "count", "stages"],
        );

        assert_eq!(
            resolved,
            vec![
                ("tags".to_string(), DefaultValue::Empty),
                ("index".to_string(), DefaultValue::Empty),
                ("count".to_string(), DefaultValue::Empty),
                ("stages".to_string(), DefaultValue::Empty),
            ],
            "a known type-zero must stay `Empty`; only an unread value becomes `Unresolved`"
        );
    }

    /// The dominant unreadable-default shape in the consumer crates, measured across every repo
    /// with an `alef.toml`: a field initialized from a module-level const of non-`&str` type.
    /// Nine of the eighteen would-be-unresolved fields fleet-wide are exactly this, and two of
    /// them already ship as `0` in generated Python against Rust values of `1024` and `6`. ~keep
    #[test]
    fn a_module_const_of_any_literal_type_resolves_to_its_value() {
        let resolved = defaults_for(
            r#"
                const DEFAULT_DETECTION_LIMIT_SIDE_LEN: u32 = 1024;
                const DEFAULT_RECOGNITION_BATCH_SIZE: usize = 6;
                const DEFAULT_DB_THRESH: f32 = 0.3;
                const DEFAULT_VERBOSE: bool = true;

                pub struct PaddleOcrConfig {
                    pub det_limit_side_len: u32,
                    pub rec_batch_num: usize,
                    pub det_db_thresh: f32,
                    pub verbose: bool,
                }

                impl Default for PaddleOcrConfig {
                    fn default() -> Self {
                        Self {
                            det_limit_side_len: DEFAULT_DETECTION_LIMIT_SIDE_LEN,
                            rec_batch_num: DEFAULT_RECOGNITION_BATCH_SIZE,
                            det_db_thresh: DEFAULT_DB_THRESH,
                            verbose: DEFAULT_VERBOSE,
                        }
                    }
                }
            "#,
            "PaddleOcrConfig",
            &["det_limit_side_len", "rec_batch_num", "det_db_thresh", "verbose"],
        );

        assert_eq!(
            resolved,
            vec![
                ("det_limit_side_len".to_string(), DefaultValue::IntLiteral(1024)),
                ("rec_batch_num".to_string(), DefaultValue::IntLiteral(6)),
                ("det_db_thresh".to_string(), DefaultValue::FloatLiteral(0.3)),
                ("verbose".to_string(), DefaultValue::BoolLiteral(true)),
            ],
            "a numeric module const is readable; substituting the type-zero for it is the same \
             fabrication as substituting one for an unread default"
        );
    }

    /// A variant may be named through any number of module segments. Only the last segment is the
    /// variant, and stopping at exactly two made three fleet-wide enum defaults unreadable.
    #[test]
    fn a_fully_qualified_enum_path_still_lowers_to_its_last_segment() {
        let resolved = defaults_for_typed(
            r#"
                pub struct ExtractionConfig { pub result_format: ResultFormat }

                impl Default for ExtractionConfig {
                    fn default() -> Self {
                        Self { result_format: crate::types::ResultFormat::Unified }
                    }
                }
            "#,
            "ExtractionConfig",
            &[("result_format", TypeRef::Named("ResultFormat".to_string()))],
        );

        assert_eq!(
            resolved,
            vec![(
                "result_format".to_string(),
                DefaultValue::EnumVariant("Unified".to_string())
            )]
        );
    }

    /// `Cow` is a representation the binding layer already erases via `FieldDef::core_wrapper`,
    /// so the value is whatever it wraps. Reading through it is not a guess, and refusing to
    /// would turn a field that generates the correct `""` today into a generation error.
    #[test]
    fn a_cow_wrapped_literal_resolves_to_the_literal_it_wraps() {
        let resolved = defaults_for_typed(
            r#"
                pub struct ProcessConfig { pub language: Cow<'static, str>, pub tag: Cow<'static, str> }

                impl Default for ProcessConfig {
                    fn default() -> Self {
                        Self {
                            language: Cow::Borrowed(""),
                            tag: std::borrow::Cow::Borrowed("stable"),
                        }
                    }
                }
            "#,
            "ProcessConfig",
            &[("language", TypeRef::String), ("tag", TypeRef::String)],
        );

        assert_eq!(
            resolved,
            vec![
                ("language".to_string(), DefaultValue::StringLiteral(String::new())),
                ("tag".to_string(), DefaultValue::StringLiteral("stable".to_string())),
            ]
        );
    }

    /// The boundary the `Cow` reading must not cross: a `Cow` around something alef cannot read
    /// is still unread.
    #[test]
    fn a_cow_wrapping_an_unreadable_expression_stays_unresolved() {
        let resolved = defaults_for_typed(
            r#"
                pub struct ProcessConfig { pub language: Cow<'static, str> }

                impl Default for ProcessConfig {
                    fn default() -> Self {
                        Self { language: Cow::Owned(detect_language()) }
                    }
                }
            "#,
            "ProcessConfig",
            &[("language", TypeRef::String)],
        );

        assert!(
            matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
            "got {resolved:?}"
        );
    }

    #[test]
    fn collect_literal_consts_indexes_associated_consts_under_their_owning_type() {
        let file: syn::File = syn::parse_str(
            r#"
                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
                    pub const MAX_TOKENS: u32 = 4096;
                }
                impl Default for LlmConfig {
                    const NOT_A_CONSTRUCTOR: &str = "trait-impl";
                    fn default() -> Self { Self {} }
                }
                #[cfg(test)]
                impl LlmConfig {
                    pub const DEFAULT_MODEL: &str = "test-only";
                }
            "#,
        )
        .expect("valid file");

        let consts = collect_literal_consts(&file.items);

        assert_eq!(
            consts.get("LlmConfig::DEFAULT_MODEL"),
            Some(&DefaultValue::StringLiteral("claude-sonnet-4-5".to_string())),
            "a `#[cfg(test)]` impl must not shadow the real associated const"
        );
        assert_eq!(
            consts.get("LlmConfig::MAX_TOKENS"),
            Some(&DefaultValue::IntLiteral(4096))
        );
        assert_eq!(
            consts
                .get("Default::NOT_A_CONSTRUCTOR")
                .or(consts.get("LlmConfig::NOT_A_CONSTRUCTOR")),
            None,
            "trait-impl associated consts are not inherent consts of the type"
        );
    }
}
