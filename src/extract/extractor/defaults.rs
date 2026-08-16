use crate::core::ir::{DefaultValue, FieldDef};
use ahash::AHashMap;
use syn;

/// Extract concrete default values from an `impl Default for T` block.
///
/// Finds the `fn default() -> Self` method, parses its struct literal body,
/// and maps each field initializer expression to a `DefaultValue` variant.
/// Falls back to `DefaultValue::Empty` for expressions that cannot be parsed
/// into a concrete literal (e.g., method calls, complex expressions).
///
/// `string_consts` resolves a field initializer that references a sibling
/// `pub const NAME: &str = "literal";` declared in the same module (e.g.
/// `NAME.to_string()`, or a bare `NAME`) to that constant's actual literal value.
/// Without it, such an initializer collapses to `DefaultValue::Empty`, which every
/// backend then renders as the *type's* zero value (`""` for a Python `str` field)
/// even though the docstring copied from the same field states the real default —
/// a silent, backend-visible drift from the Rust default. See
/// [`collect_string_consts`]. ~keep
pub(crate) fn extract_default_values(
    item: &syn::ItemImpl,
    fields: &mut [FieldDef],
    string_consts: &AHashMap<String, String>,
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
        for field in fields.iter_mut() {
            field.typed_default = Some(DefaultValue::Empty);
        }
        return;
    };

    let defaults = parse_default_body(&default_fn.block, string_consts);

    for field in fields.iter_mut() {
        if let Some(default_val) = defaults.get(&field.name) {
            field.typed_default = Some(default_val.clone());
        } else {
            field.typed_default = Some(DefaultValue::Empty);
        }
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

/// Parse the body of a `fn default()` to extract field → `DefaultValue` mappings.
///
/// Looks for a struct literal (`Self { field: expr, ... }`) in the function body
/// and maps each field initializer to a `DefaultValue`.
fn parse_default_body(block: &syn::Block, string_consts: &AHashMap<String, String>) -> AHashMap<String, DefaultValue> {
    let mut defaults = AHashMap::new();

    let struct_expr = find_struct_expr(block);

    let Some(struct_expr) = struct_expr else {
        return defaults;
    };

    for field in &struct_expr.fields {
        let Some(ident) = &field.member_named() else {
            continue;
        };
        let field_name = ident.to_string();
        let default_val = expr_to_default_value(&field.expr, string_consts);
        defaults.insert(field_name, default_val);
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
///   where `CONST_NAME` resolves via `string_consts` → `StringLiteral` of the
///   constant's value
/// - Anything else → `Empty`
fn expr_to_default_value(expr: &syn::Expr, string_consts: &AHashMap<String, String>) -> DefaultValue {
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

        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            match expr_to_default_value(&unary.expr, string_consts) {
                DefaultValue::IntLiteral(v) => DefaultValue::IntLiteral(-v),
                DefaultValue::FloatLiteral(v) => DefaultValue::FloatLiteral(-v),
                _ => DefaultValue::Empty,
            }
        }

        syn::Expr::Binary(bin) => {
            let lhs = expr_to_default_value(&bin.left, string_consts);
            let rhs = expr_to_default_value(&bin.right, string_consts);
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
                    if let Some(value) = resolve_string_const(&mc.receiver, string_consts) {
                        return DefaultValue::StringLiteral(value);
                    }
                    DefaultValue::Empty
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
                    return expr_to_default_value(inner, string_consts);
                }

                if segments == ["String", "from"] && call.args.len() == 1 {
                    if let Some(syn::Expr::Lit(lit)) = call.args.first()
                        && let syn::Lit::Str(s) = &lit.lit
                    {
                        return DefaultValue::StringLiteral(s.value());
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
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() == 2 {
                return DefaultValue::EnumVariant(segments[1].clone());
            }
            if segments.len() == 1 && segments[0] == "None" {
                return DefaultValue::None;
            }
            if let Some(value) = resolve_string_const(expr, string_consts) {
                return DefaultValue::StringLiteral(value);
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
                match expr_to_default_value(element, string_consts) {
                    // Only self-contained values may sit in an element position. A function-call
                    // default cannot be evaluated at generation time, and `Empty`/`None` carry no
                    // element value at all; any of them makes the whole literal non-representable.
                    // Lowering a partial list would hand a backend a default that silently differs
                    // from the Rust one, which is worse than collapsing to `Empty`. ~keep
                    value @ (DefaultValue::BoolLiteral(_)
                    | DefaultValue::StringLiteral(_)
                    | DefaultValue::IntLiteral(_)
                    | DefaultValue::FloatLiteral(_)
                    | DefaultValue::EnumVariant(_)
                    | DefaultValue::ListLiteral(_)) => lowered.push(value),
                    DefaultValue::Empty
                    | DefaultValue::None
                    | DefaultValue::FunctionCall(_)
                    | DefaultValue::PublicFunctionCall(_) => return DefaultValue::Empty,
                }
            }
            DefaultValue::ListLiteral(lowered)
        }

        _ => DefaultValue::Empty,
    }
}

/// Resolves a single-segment path expression against `string_consts`, returning the
/// referenced constant's literal value.
fn resolve_string_const(expr: &syn::Expr, string_consts: &AHashMap<String, String>) -> Option<String> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    let ident = path.path.get_ident()?;
    string_consts.get(&ident.to_string()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_value_of(expr_src: &str) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        expr_to_default_value(&expr, &AHashMap::new())
    }

    fn default_value_of_with_consts(expr_src: &str, consts: &[(&str, &str)]) -> DefaultValue {
        let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
        let string_consts: AHashMap<String, String> =
            consts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        expr_to_default_value(&expr, &string_consts)
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
                const CACHE_DIR_NAME: &str = "liter-llm";
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
        assert_eq!(consts.get("CACHE_DIR_NAME").map(String::as_str), Some("liter-llm"));
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
}
