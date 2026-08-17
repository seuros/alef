use super::helpers::is_test_gated;
use crate::core::ir::{DefaultValue, FieldDef};
use ahash::AHashMap;
use quote::ToTokens;
use syn;

/// Every associated function of every inherent `impl` block in one module, keyed by
/// `(type name, function name)`.
///
/// Exists so [`extract_default_values`] can follow a `fn default()` that delegates to one of
/// its own constructors instead of spelling a struct literal. Scoped to a single module for
/// the same reason [`collect_string_consts`] is: `impl Default` and the `fn new` it calls sit
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
/// `string_consts` resolves a field initializer that references a sibling
/// `pub const NAME: &str = "literal";` declared in the same module (e.g.
/// `NAME.to_string()`, or a bare `NAME`) to that constant's actual literal value.
/// See [`collect_string_consts`].
pub(crate) fn extract_default_values(
    item: &syn::ItemImpl,
    self_type: &str,
    fields: &mut [FieldDef],
    string_consts: &AHashMap<String, String>,
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

    let scope = EvalScope::new(string_consts);

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

/// Collects `pub const NAME: &str = "literal";` (and private `const` items, which
/// are visible module-wide including to `impl` blocks in the same file) declared
/// alongside an `impl Default` block, so [`extract_default_values`] can resolve a
/// field initializer that references the constant instead of collapsing it to
/// `DefaultValue::Empty`.
///
/// Deliberately scoped to the items of a single module/file: `impl Default` and
/// the const it references are the overwhelmingly common shape (`refresh.rs`'s
/// `DEFAULT_CATALOG_URL` alongside `CatalogRefreshConfig`'s `impl Default`), and
/// resolving a `use`-imported const from another module would need a full
/// crate-wide const index. Only string-literal-valued consts are collected —
/// anything else is out of scope for a field default. ~keep
pub(crate) fn collect_string_consts(items: &[syn::Item]) -> AHashMap<String, String> {
    let mut consts = AHashMap::new();
    for item in items {
        if let syn::Item::Const(item_const) = item
            && is_str_type(&item_const.ty)
            && let syn::Expr::Lit(lit) = item_const.expr.as_ref()
            && let syn::Lit::Str(s) = &lit.lit
        {
            consts.insert(item_const.ident.to_string(), s.value());
        }
    }
    consts
}

/// True for `str` or `&str` (any lifetime) types, the only const shapes
/// [`collect_string_consts`] resolves.
fn is_str_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => is_str_type(&r.elem),
        syn::Type::Path(p) => p.path.is_ident("str"),
        _ => false,
    }
}

/// Everything a field initializer can be resolved against.
///
/// `string_consts` is module-wide and constant. `params` is populated only while reading a
/// constructor's body on behalf of a delegating `fn default()`: it binds that constructor's
/// parameters to the literal arguments the delegation passed, which is what turns
/// `fn default() { Self::new("en") }` plus `fn new(lang: &str) { Self { lang: lang.into(), .. } }`
/// into `lang = "en"` rather than a guess. ~keep
struct EvalScope<'a> {
    string_consts: &'a AHashMap<String, String>,
    params: AHashMap<String, DefaultValue>,
}

impl<'a> EvalScope<'a> {
    fn new(string_consts: &'a AHashMap<String, String>) -> Self {
        Self {
            string_consts,
            params: AHashMap::new(),
        }
    }

    fn with_params(&self, params: AHashMap<String, DefaultValue>) -> EvalScope<'a> {
        EvalScope {
            string_consts: self.string_consts,
            params,
        }
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
        let value = expr_to_default_value(argument, scope);
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
        defaults.insert(ident.to_string(), expr_to_default_value(&field.expr, scope));
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

/// Convert an expression to a `DefaultValue`.
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
/// - `SomeEnum::Variant` → `EnumVariant("Variant")`
/// - `CONST_NAME.to_string()` / `.to_owned()` / `.into()`, or a bare `CONST_NAME`,
///   where `CONST_NAME` resolves via `scope.string_consts` → `StringLiteral` of the
///   constant's value
/// - a bare constructor parameter, or `param.to_string()` / `.to_owned()` / `.into()`, where
///   `param` is bound in `scope.params` → the value the delegation passed for it
/// - Anything else → `Empty`
///
/// Note the last line: a field initializer this function cannot read still collapses to
/// `Empty`, not [`DefaultValue::Unresolved`]. Only a whole unreadable `fn default()` body is
/// reported as unresolved so far — see the module-level note in
/// [`extract_default_values`]. ~keep
fn expr_to_default_value(expr: &syn::Expr, scope: &EvalScope<'_>) -> DefaultValue {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Bool(b) => DefaultValue::BoolLiteral(b.value),
            syn::Lit::Int(i) => {
                if let Ok(val) = i.base10_parse::<i64>() {
                    DefaultValue::IntLiteral(val)
                } else {
                    DefaultValue::Empty
                }
            }
            syn::Lit::Float(f) => {
                if let Ok(val) = f.base10_parse::<f64>() {
                    DefaultValue::FloatLiteral(val)
                } else {
                    DefaultValue::Empty
                }
            }
            syn::Lit::Char(c) => DefaultValue::StringLiteral(c.value().to_string()),
            syn::Lit::Str(s) => DefaultValue::StringLiteral(s.value()),
            _ => DefaultValue::Empty,
        },

        // `&"en"` and `&CONST` reach a constructor parameter unchanged; the reference is not
        // part of the value. ~keep
        syn::Expr::Reference(reference) => expr_to_default_value(&reference.expr, scope),

        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            match expr_to_default_value(&unary.expr, scope) {
                DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
                DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
                _ => DefaultValue::Empty,
            }
        }

        syn::Expr::Binary(bin) => {
            let lhs = expr_to_default_value(&bin.left, scope);
            let rhs = expr_to_default_value(&bin.right, scope);
            match (lhs, rhs) {
                (DefaultValue::IntLiteral(a), DefaultValue::IntLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => a
                        .checked_add(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Sub(_) => a
                        .checked_sub(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Mul(_) => a
                        .checked_mul(b)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Div(_) if b != 0 => DefaultValue::IntLiteral(a / b),
                    syn::BinOp::Rem(_) if b != 0 => DefaultValue::IntLiteral(a % b),
                    syn::BinOp::Shl(_) if (0..63).contains(&b) => a
                        .checked_shl(b as u32)
                        .map(DefaultValue::IntLiteral)
                        .unwrap_or(DefaultValue::Empty),
                    syn::BinOp::Shr(_) if (0..63).contains(&b) => DefaultValue::IntLiteral(a >> (b as u32)),
                    syn::BinOp::BitOr(_) => DefaultValue::IntLiteral(a | b),
                    syn::BinOp::BitAnd(_) => DefaultValue::IntLiteral(a & b),
                    syn::BinOp::BitXor(_) => DefaultValue::IntLiteral(a ^ b),
                    _ => DefaultValue::Empty,
                },
                (DefaultValue::FloatLiteral(a), DefaultValue::FloatLiteral(b)) => match bin.op {
                    syn::BinOp::Add(_) => DefaultValue::FloatLiteral(a + b),
                    syn::BinOp::Sub(_) => DefaultValue::FloatLiteral(a - b),
                    syn::BinOp::Mul(_) => DefaultValue::FloatLiteral(a * b),
                    syn::BinOp::Div(_) if b != 0.0 => DefaultValue::FloatLiteral(a / b),
                    _ => DefaultValue::Empty,
                },
                _ => DefaultValue::Empty,
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
                        _ => DefaultValue::Empty,
                    }
                }
                _ => DefaultValue::Empty,
            }
        }

        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = &*call.func {
                let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();

                if (segments == ["Some"] || segments == ["Option", "Some"])
                    && call.args.len() == 1
                    && let Some(inner) = call.args.first()
                {
                    return expr_to_default_value(inner, scope);
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
                    return DefaultValue::Empty;
                }

                if segments == ["String", "new"] && call.args.is_empty() {
                    return DefaultValue::StringLiteral(String::new());
                }

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
                    return DefaultValue::Empty;
                }

                if segments == ["Duration", "from_millis"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Int(i) = &lit.lit
                        && let Ok(val) = i.base10_parse::<i64>()
                    {
                        return DefaultValue::IntLiteral(val);
                    }
                    return DefaultValue::Empty;
                }

                if segments.last().is_some_and(|s| s == "default") {
                    return DefaultValue::Empty;
                }

                if call.args.is_empty() {
                    return DefaultValue::FunctionCall(segments.join("::"));
                }
            }
            DefaultValue::Empty
        }

        syn::Expr::Path(path) => {
            if let Some(value) = resolve_ident(expr, scope) {
                return value;
            }
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 2 {
                return DefaultValue::EnumVariant(segments[1].clone());
            }
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            DefaultValue::Empty
        }

        syn::Expr::Macro(mac) => {
            let macro_name = mac
                .mac
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if !matches!(macro_name.as_str(), "vec" | "hashmap" | "hashset") || mac.mac.tokens.is_empty() {
                return DefaultValue::Empty;
            }
            // Only `vec!` is destructured. `hashmap!`/`hashset!` carry key-value and set
            // semantics `DefaultValue` cannot represent, so they keep collapsing to `Empty`
            // rather than being flattened into a list that would render wrongly. ~keep
            if macro_name != "vec" {
                return DefaultValue::Empty;
            }
            let Ok(elements) = mac
                .mac
                .parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
            else {
                // `vec![expr; N]` is not a comma list and fails to parse as one. ~keep
                return DefaultValue::Empty;
            };
            if elements.is_empty() {
                return DefaultValue::Empty;
            }
            let mut lowered = Vec::with_capacity(elements.len());
            for element in &elements {
                let value = expr_to_default_value(element, scope);
                // Only self-contained values may sit in an element position. A function-call
                // default cannot be evaluated at generation time, and `Empty`/`None`/
                // `Unresolved` carry no element value at all; any of them makes the whole
                // literal non-representable. Lowering a partial list would hand a backend a
                // default that silently differs from the Rust one, which is worse than
                // collapsing to `Empty`. ~keep
                if !carries_value(&value) {
                    return DefaultValue::Empty;
                }
                lowered.push(value);
            }
            DefaultValue::ListLiteral(lowered)
        }

        _ => DefaultValue::Empty,
    }
}

/// Resolves a single-identifier expression against the scope: a bound constructor parameter
/// first, then a module-level string constant.
///
/// Parameters win because they are the narrower binding — inside a constructor body an
/// identifier that shadows a module const refers to the parameter. ~keep
fn resolve_ident(expr: &syn::Expr, scope: &EvalScope<'_>) -> Option<DefaultValue> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    let ident = path.path.get_ident()?.to_string();
    if let Some(value) = scope.params.get(&ident) {
        return Some(value.clone());
    }
    scope
        .string_consts
        .get(&ident)
        .cloned()
        .map(DefaultValue::StringLiteral)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_value_of(expr_src: &str) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        expr_to_default_value(&expr, &EvalScope::new(&AHashMap::new()))
    }

    fn default_value_of_with_consts(expr_src: &str, consts: &[(&str, &str)]) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        let string_consts: AHashMap<String, String> =
            consts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        expr_to_default_value(&expr, &EvalScope::new(&string_consts))
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
    fn unresolvable_const_reference_falls_back_to_empty() {
        // No matching entry in `string_consts` — must not be mistaken for an enum variant
        // path or crash; the caller loses the literal but stays correct-if-conservative.
        assert_eq!(default_value_of("UNKNOWN_CONST.to_string()"), DefaultValue::Empty);
    }

    #[test]
    fn collect_string_consts_finds_str_typed_consts_only() {
        let file: syn::File = syn::parse_str(
            r#"
                pub const DEFAULT_CATALOG_URL: &str = "https://example.com/catalog.json";
                const CACHE_DIR_NAME: &str = "sample-crate";
                const RETRY_LIMIT: u32 = 3;
                const COMPUTED: &str = some_fn();
            "#,
        )
        .expect("valid file");

        let consts = collect_string_consts(&file.items);

        assert_eq!(
            consts.get("DEFAULT_CATALOG_URL").map(String::as_str),
            Some("https://example.com/catalog.json")
        );
        assert_eq!(consts.get("CACHE_DIR_NAME").map(String::as_str), Some("sample-crate"));
        assert_eq!(
            consts.get("RETRY_LIMIT"),
            None,
            "non-string-typed consts must not be collected"
        );
        assert_eq!(
            consts.get("COMPUTED"),
            None,
            "non-literal initializers must not be collected"
        );
    }

    /// Drive the whole extractor over a module source, returning the resolved defaults for
    /// the named type's `impl Default`. Reproduces exactly what `extractor::mod` does: build
    /// the const and constructor indexes from the module's items, then read the `impl Default`
    /// against them.
    fn defaults_for(source: &str, type_name: &str, field_names: &[&str]) -> Vec<(String, DefaultValue)> {
        let file: syn::File = syn::parse_str(source).expect("valid module source");
        let string_consts = collect_string_consts(&file.items);
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

        let mut fields: Vec<FieldDef> = field_names
            .iter()
            .map(|name| FieldDef {
                name: (*name).to_string(),
                ..Default::default()
            })
            .collect();

        extract_default_values(default_impl, type_name, &mut fields, &string_consts, &constructors);

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
    /// placeholder would put a guessed value in a field that reads the parameter.
    #[test]
    fn a_delegation_with_an_unfoldable_argument_leaves_that_field_empty_not_wrong() {
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

        assert_eq!(
            resolved,
            vec![
                ("name".to_string(), DefaultValue::Empty),
                ("level".to_string(), DefaultValue::IntLiteral(2)),
            ],
            "the unfoldable argument must not poison the sibling field it does not reach"
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
}
