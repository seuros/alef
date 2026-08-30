//! ABI/coordinate grammar validation for the C-ABI configuration surface.
//!
//! Every generator that reads `[crates.ffi]` splices some piece of that config
//! straight into a target file it does not fully control the syntax of: a Rust
//! string literal in generated `build.rs`, a Makefile variable assignment, a
//! `cbindgen.toml`/Cargo.toml table, or a POSIX shell script. Each of those hosts
//! has its own escaping rules and its own "active" constructs (Make's `$(...)`,
//! Bash's `$()`/backticks even inside double quotes, Rust's string-literal escapes,
//! Cargo's TOML table/key syntax). A single generic "identifier" or "safe string"
//! check cannot protect all of them without either missing a host's active
//! construct or rejecting a currently-valid configuration for a stricter host it
//! doesn't actually reach.
//!
//! This module has one function per target grammar. Each function's doc comment
//! names the grammar it enforces and cites where that grammar comes from, so a
//! future change can tell whether a given field's rule is still the right one
//! instead of re-deriving it from scratch. Callers are expected to wrap the
//! `Err(String)` returned here into a `ResolveError::InvalidConfig` (or an
//! `anyhow` context) that names the offending field and crate.

/// Validate `[ffi] header_name`.
///
/// **Grammar:** exactly one path component, ending in `.h`, drawn from the POSIX
/// portable filename character set (`A-Za-z0-9._-`) restricted further to exclude
/// `/` and `\` entirely so the value cannot smuggle in a second path component.
///
/// **Source:** the field's own doc comment ("used by cbindgen to declare the
/// return... `.h`") and the generic C `#include "header.h"` / cbindgen single-file
/// convention — there is no directory structure to preserve here, only a bare
/// filename. Restricting the charset to the POSIX portable set additionally rules
/// out the two hosts this value reaches raw: a Rust string literal in generated
/// `build.rs` (`Path::new("include/{header_name}")`) and Make `$(wildcard ...)`
/// patterns, both of which treat `"` / `$(` / newlines as active.
///
/// **Active construct beyond quote/backslash/control chars:** none in the
/// filename grammar itself — the risk here is path traversal (`/`, `\`, `..`
/// segments), not code execution, which is why this function forbids separators
/// outright rather than merely rejecting `..`.
pub fn validate_c_header_filename(value: &str) -> Result<(), String> {
    let Some(stem) = value.strip_suffix(".h") else {
        return Err(format!("`{value}` must end in `.h`"));
    };
    if stem.is_empty() {
        return Err("must have a non-empty name before `.h`".to_string());
    }
    if value.contains('/') || value.contains('\\') {
        return Err(format!(
            "`{value}` must be a single filename with no path separators (`/` or `\\`)"
        ));
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
    if !value.chars().all(allowed) {
        return Err(format!(
            "`{value}` may only contain ASCII letters, digits, `_`, `-`, and `.`"
        ));
    }
    Ok(())
}

/// Validate a native artifact basename: `[ffi] lib_name`, the derived
/// release-artifact package name, and the e2e `c` package name.
///
/// **Grammar:** `^[A-Za-z0-9_][A-Za-z0-9_.+-]*$` — first character alphanumeric
/// or `_`, remaining characters alphanumeric, `_`, `-`, `.`, or `+`.
///
/// **Source:** pkg-config's own `.pc` basename convention (pkg-config(1) package
/// names commonly contain `+` and `.`, e.g. `gtk+-3.0`, `libxml-2.0`), which is
/// the strictest of the three hosts this value reaches raw (Rust
/// `cargo:rustc-link-arg` directives, a generated Makefile's `-l`/`$(shell
/// pkg-config ...)` lines, and a release tarball / URL path segment). Adopting
/// pkg-config's own charset is therefore also safe for the other two.
///
/// **Active construct beyond quote/backslash/control chars:** Make's `$(...)`
/// (a `$(shell ...)` call or a bare `$(name)` variable reference) and a raw
/// newline, which starts a new Makefile line/recipe. Excluding `$`, `(`, `)`,
/// and whitespace from the allowed charset blocks both.
pub fn validate_native_artifact_basename(value: &str) -> Result<(), String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("must not be empty".to_string());
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return Err(format!("`{value}` must start with an ASCII letter, digit, or `_`"));
    }
    let rest_allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+');
    if !chars.all(rest_allowed) {
        return Err(format!(
            "`{value}` may only contain ASCII letters, digits, `_`, `-`, `.`, and `+` after the first character"
        ));
    }
    Ok(())
}

/// Validate a bare ASCII ABI identifier: `[ffi] prefix` and a capsule's
/// `c_return_type`.
///
/// **Grammar:** `^[A-Za-z_][A-Za-z0-9_]*$` — a valid C identifier (ISO C
/// §6.4.2.1), restricted to the ASCII subset C's translation-limits guarantee is
/// portable.
///
/// **Source:** both fields become part of, or the entirety of, a `#[no_mangle]
/// extern "C"` symbol name or a cbindgen-declared C type name — every C-ABI
/// backend this project generates for (cbindgen, Go cgo, JNI, C# P/Invoke) only
/// portably links ASCII C identifiers.
///
/// **Active construct beyond quote/backslash/control chars:** none for a bare
/// identifier destined for an identifier position — but `c_return_type` also
/// reaches a Rust string literal in generated `build.rs`
/// (`header.replace("{prefixed}", "{bare}")`), where a `"` would break out of the
/// literal. This grammar excludes `"` (and every other non-identifier character)
/// by construction, which is why the escaping step at that call site is
/// defense-in-depth rather than the only guard.
pub fn validate_ascii_abi_identifier(value: &str) -> Result<(), String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("must not be empty".to_string());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!("`{value}` must start with an ASCII letter or `_`"));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "`{value}` may only contain ASCII letters, digits, and `_` after the first character"
        ));
    }
    Ok(())
}

/// Validate a capsule's `into_raw_type`: a fully-qualified Rust pointee type path
/// with no generic or const-generic arguments.
///
/// **Grammar:** `syn::TypePath`, no `qself` (no `<Foo as Trait>::Bar` syntax), and
/// every path segment's `PathArguments` must be `None` — i.e. `ident(::ident)*`
/// only.
///
/// **Source:** the field's own doc comment: "Fully-qualified Rust pointee type of
/// the `*const {into_raw_type}}` that `value.into_raw()` already returns" — every
/// real example (`tree_sitter::ffi::TSLanguage`) is a bare module path with no
/// generics, so forbidding generics entirely accepts every currently-documented
/// use while closing the injection surface a generic-argument grammar would
/// otherwise admit.
///
/// **Active construct beyond quote/backslash/control chars:** a generic
/// argument list may contain a const-generic block, `Foo<{ EXPR }>`, which is an
/// arbitrary const-evaluated Rust expression — a real code-execution position
/// inside what looks like "just a type." Requiring every segment's
/// `PathArguments` to be `None` rejects any `<...>` outright, so this construct
/// can never appear.
pub fn validate_rust_pointee_type_path(value: &str) -> Result<(), String> {
    let parsed: syn::TypePath =
        syn::parse_str(value).map_err(|error| format!("`{value}` is not a Rust type path: {error}"))?;
    if parsed.qself.is_some() {
        return Err(format!(
            "`{value}` must not use qualified-path syntax (`<Type as Trait>::...`)"
        ));
    }
    for segment in &parsed.path.segments {
        if !matches!(segment.arguments, syn::PathArguments::None) {
            return Err(format!(
                "`{value}` must not carry generic arguments (segment `{}` has some)",
                segment.ident
            ));
        }
    }
    Ok(())
}

/// Validate a `[[crates.ffi.target_dep_overrides]] cfg` expression.
///
/// **Grammar:** a restricted subset of `syn::Expr` mirroring `rustc`'s actual
/// `cfg(...)` attribute grammar (see the Rust reference, "Conditional
/// compilation"): a bare identifier (`unix`), a `key = "string"` predicate whose
/// value is a double-quoted string literal, or an `any(...)` / `all(...)` /
/// `not(...)` call whose arguments are themselves recursively valid under this
/// same grammar.
///
/// **Source:** this is the exact grammar `#[cfg(...)]` accepts; parsing through
/// `syn` (rather than string-matching for `any(`/`all(`/`not(`) means a malformed
/// or hostile value fails to parse instead of silently matching a substring.
///
/// **Active construct beyond quote/backslash/control chars:** a char literal
/// (`'x'`, `syn::Lit::Char`) is a valid `syn::Expr` and a valid right-hand side
/// of a `key = 'x'` assignment — every bit as syntactically legal as `key =
/// "x"` — yet it contains a literal `'` that is completely inert in Rust. This
/// value is spliced into a TOML *literal* string, `[target.'cfg({cfg})'...]`,
/// where TOML forbids `'` inside literal strings entirely, so a raw `'` here
/// breaks out of that table header. Labeled loops/blocks (`'a: loop { ... }`)
/// carry the same raw `'` and are likewise valid, unrelated `syn::Expr`
/// variants. Restricting assignment values to `Lit::Str` and restricting every
/// expression shape to bare-path / assign / any-all-not-call (the wildcard arm
/// rejects everything else, including loops) closes both off; nothing accepted
/// by this grammar can produce a literal `'` in the source text.
pub fn validate_cfg_expression(value: &str) -> Result<(), String> {
    let expr: syn::Expr =
        syn::parse_str(value).map_err(|error| format!("`{value}` is not a valid cfg expression: {error}"))?;
    validate_cfg_expr_shape(&expr)
}

fn validate_cfg_expr_shape(expr: &syn::Expr) -> Result<(), String> {
    match expr {
        syn::Expr::Path(p) if p.qself.is_none() => validate_cfg_bare_path(&p.path),
        syn::Expr::Assign(assign) => {
            let syn::Expr::Path(lhs) = assign.left.as_ref() else {
                return Err("cfg predicate key must be a bare identifier".to_string());
            };
            validate_cfg_bare_path(&lhs.path)?;
            match assign.right.as_ref() {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(_), ..
                }) => Ok(()),
                _ => Err("cfg predicate value must be a double-quoted string literal".to_string()),
            }
        }
        syn::Expr::Call(call) => {
            let syn::Expr::Path(func) = call.func.as_ref() else {
                return Err("cfg combinator must be a bare `any`/`all`/`not`".to_string());
            };
            let Some(name) = func.path.get_ident().map(ToString::to_string) else {
                return Err("cfg combinator must be a single bare identifier".to_string());
            };
            if !matches!(name.as_str(), "any" | "all" | "not") {
                return Err(format!("`{name}` is not a valid cfg combinator (expected any/all/not)"));
            }
            for arg in &call.args {
                validate_cfg_expr_shape(arg)?;
            }
            Ok(())
        }
        _ => Err(
            "unsupported cfg expression shape — expected a bare key, `key = \"value\"`, or any/all/not(...)"
                .to_string(),
        ),
    }
}

fn validate_cfg_bare_path(path: &syn::Path) -> Result<(), String> {
    let Some(ident) = path.get_ident() else {
        return Err("cfg key must be a single bare identifier, not a `::`-separated path".to_string());
    };
    let text = ident.to_string();
    if text.starts_with("r#") {
        return Err(format!("`{text}` must not be a raw identifier"));
    }
    validate_ascii_abi_identifier(&text)
}

/// Validate a capsule's `package` field: a Cargo package name injected as a bare
/// TOML table key (`{package} = "{version}"`).
///
/// **Grammar:** first character an ASCII letter, remaining characters ASCII
/// alphanumeric, `_`, or `-` — the practical shape of every published crates.io
/// package name.
///
/// **Source:** Cargo's package-name rules (the Cargo Book, "The Manifest
/// Format" § `package.name`).
///
/// **Active construct beyond quote/backslash/control chars:** a bare (unquoted)
/// TOML key ends at the first `=`, `.`, `[`, `]`, or newline — any of those
/// characters lets the value terminate the key early and start a new TOML
/// table/key, e.g. injecting a `[patch.crates-io]` table. This grammar excludes
/// all of them.
pub fn validate_cargo_package_name(value: &str) -> Result<(), String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("must not be empty".to_string());
    };
    if !first.is_ascii_alphabetic() {
        return Err(format!("`{value}` must start with an ASCII letter"));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!(
            "`{value}` may only contain ASCII letters, digits, `_`, and `-` after the first character"
        ));
    }
    Ok(())
}

/// Validate a capsule's `package_version` field: a Cargo dependency version
/// requirement injected inside a quoted TOML string (`{package} = "{version}"`).
///
/// **Grammar:** `semver::VersionReq` — Cargo's actual dependency version
/// requirement syntax (`"1"`, `"^1.2"`, `">=1, <2"`, ...), not a bare
/// `semver::Version`, since this is the right-hand side of a `[dependencies]`
/// entry rather than a package's own declared version.
///
/// **Source:** the `semver` crate's `VersionReq`, which is what Cargo itself
/// uses to parse `[dependencies]` version strings.
///
/// **Active construct beyond quote/backslash/control chars:** a raw `"` closes
/// the TOML string early. `VersionReq`'s grammar has no representation for `"`
/// (or any other TOML-active character), so a successful parse already excludes
/// it.
pub fn validate_cargo_version_req(value: &str) -> Result<(), String> {
    semver::VersionReq::parse(value)
        .map(|_| ())
        .map_err(|error| format!("`{value}` is not a valid Cargo version requirement: {error}"))
}

/// Validate a Cargo feature name: `[ffi] features`, `extra_features`,
/// `excluded_default_features`, and `target_dep_overrides[].features` entries.
///
/// **Grammar:** first character ASCII alphanumeric or `_`, remaining characters
/// ASCII alphanumeric, `_`, `-`, or `.` — Cargo's plain feature-name shape.
/// Deliberately excludes `/` and `:`, which are meaningful in a full Cargo
/// *feature specification* (`pkg/feat`, `dep:pkg`) — every one of these fields is
/// spliced as a **plain feature name** on both sides (the FFI crate's own new
/// feature and the forwarded `"{core-crate}/{name}"` dependency feature), so a
/// value containing `/` would forge a second, attacker-chosen dependency feature
/// reference rather than naming a feature.
///
/// **Source:** the Cargo Book, "Features" § feature names.
///
/// **Active construct beyond quote/backslash/control chars:** `"` closes the
/// surrounding TOML string (`features = ["{name}"]` / `{name} = [...]` key)
/// early; `/` reinterprets the name as a `pkg/feature` reference into a
/// different, attacker-named dependency. Both are excluded by the charset.
pub fn validate_cargo_feature_name(value: &str) -> Result<(), String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("must not be empty".to_string());
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return Err(format!("`{value}` must start with an ASCII letter, digit, or `_`"));
    }
    let rest_allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
    if !chars.all(rest_allowed) {
        return Err(format!(
            "`{value}` may only contain ASCII letters, digits, `_`, `-`, and `.` after the first character"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- header filename ----------------------------------------------------

    #[test]
    fn header_filename_accepts_default_shape() {
        assert_eq!(validate_c_header_filename("my_lib.h"), Ok(()));
        assert_eq!(validate_c_header_filename("custom.h"), Ok(()));
    }

    #[test]
    fn header_filename_rejects_path_traversal() {
        assert!(validate_c_header_filename("../../etc/passwd.h").is_err());
        assert!(validate_c_header_filename("sub/dir.h").is_err());
        assert!(validate_c_header_filename("dir\\file.h").is_err());
    }

    #[test]
    fn header_filename_rejects_rust_string_breakout() {
        assert!(validate_c_header_filename("evil\".h").is_err());
        assert!(validate_c_header_filename("evil\nfn x(){}.h").is_err());
    }

    #[test]
    fn header_filename_requires_h_suffix() {
        assert!(validate_c_header_filename("my_lib").is_err());
        assert!(validate_c_header_filename(".h").is_err());
    }

    // -- native artifact basename --------------------------------------------

    #[test]
    fn artifact_basename_accepts_default_shape() {
        assert_eq!(validate_native_artifact_basename("my_lib_ffi"), Ok(()));
        assert_eq!(validate_native_artifact_basename("libmy_custom"), Ok(()));
        assert_eq!(validate_native_artifact_basename("gtk+-3.0"), Ok(()));
    }

    #[test]
    fn artifact_basename_rejects_make_shell_canary() {
        assert!(validate_native_artifact_basename("$(shell rm -rf /)").is_err());
        assert!(validate_native_artifact_basename("foo)\nevil:\n\trm -rf /").is_err());
    }

    #[test]
    fn artifact_basename_rejects_whitespace_and_slash() {
        assert!(validate_native_artifact_basename("my lib").is_err());
        assert!(validate_native_artifact_basename("my/lib").is_err());
    }

    // -- ascii abi identifier -------------------------------------------------

    #[test]
    fn abi_identifier_accepts_default_shape() {
        assert_eq!(validate_ascii_abi_identifier("my_lib"), Ok(()));
        assert_eq!(validate_ascii_abi_identifier("TSLanguage"), Ok(()));
    }

    #[test]
    fn abi_identifier_rejects_quote_breakout() {
        assert!(validate_ascii_abi_identifier("evil\", \"x").is_err());
        assert!(validate_ascii_abi_identifier("evil-prefix").is_err());
        assert!(validate_ascii_abi_identifier("123start").is_err());
    }

    // -- rust pointee type path -----------------------------------------------

    #[test]
    fn pointee_type_path_accepts_documented_example() {
        assert_eq!(validate_rust_pointee_type_path("tree_sitter::ffi::TSLanguage"), Ok(()));
        assert_eq!(validate_rust_pointee_type_path("MyRawType"), Ok(()));
    }

    #[test]
    fn pointee_type_path_rejects_const_generic_block() {
        assert!(validate_rust_pointee_type_path("Foo<{ std::process::exit(1) }>").is_err());
    }

    #[test]
    fn pointee_type_path_rejects_qualified_path() {
        assert!(validate_rust_pointee_type_path("<Foo as Trait>::Bar").is_err());
    }

    #[test]
    fn pointee_type_path_rejects_garbage() {
        assert!(validate_rust_pointee_type_path("not a type; std::process::exit(1)").is_err());
    }

    // -- cfg expression --------------------------------------------------------

    #[test]
    fn cfg_expression_accepts_documented_example() {
        assert_eq!(
            validate_cfg_expression("all(target_os = \"android\", target_arch = \"x86_64\")"),
            Ok(())
        );
        assert_eq!(validate_cfg_expression("unix"), Ok(()));
        assert_eq!(validate_cfg_expression("not(windows)"), Ok(()));
    }

    #[test]
    fn cfg_expression_rejects_char_literal_quote_breakout() {
        // A char literal is valid syn::Expr syntax but contains a raw `'` that
        // would break out of the TOML `'cfg(...)'` literal-string table header.
        assert!(validate_cfg_expression("target_os = 'x'").is_err());
    }

    #[test]
    fn cfg_expression_rejects_labeled_loop_quote_breakout() {
        // A labeled loop is a valid, unrelated `syn::Expr` variant that still
        // carries a raw `'` in its source text — the wildcard arm must catch it.
        assert!(validate_cfg_expression("'a: loop { break }").is_err());
    }

    #[test]
    fn cfg_expression_rejects_non_string_and_unknown_combinator() {
        assert!(validate_cfg_expression("target_os = 1").is_err());
        assert!(validate_cfg_expression("evil(target_os = \"x\")").is_err());
    }

    // -- cargo package name ------------------------------------------------------

    #[test]
    fn cargo_package_name_accepts_typical_names() {
        assert_eq!(validate_cargo_package_name("tree-sitter"), Ok(()));
        assert_eq!(validate_cargo_package_name("serde_json"), Ok(()));
    }

    #[test]
    fn cargo_package_name_rejects_toml_table_injection() {
        assert!(validate_cargo_package_name("x\"\n[patch.crates-io]\nfoo").is_err());
        assert!(validate_cargo_package_name("x = \"1\"\n[dependencies]\ny").is_err());
        assert!(validate_cargo_package_name("1starts-with-digit").is_err());
    }

    // -- cargo version requirement -----------------------------------------------

    #[test]
    fn cargo_version_req_accepts_typical_requirements() {
        assert_eq!(validate_cargo_version_req("1.0"), Ok(()));
        assert_eq!(validate_cargo_version_req("^1.2.3"), Ok(()));
    }

    #[test]
    fn cargo_version_req_rejects_quote_breakout() {
        assert!(validate_cargo_version_req("1.0\"\n[patch.crates-io]").is_err());
    }

    // -- cargo feature name -------------------------------------------------------

    #[test]
    fn cargo_feature_name_accepts_typical_names() {
        assert_eq!(validate_cargo_feature_name("native-http"), Ok(()));
        assert_eq!(validate_cargo_feature_name("android-target"), Ok(()));
    }

    #[test]
    fn cargo_feature_name_rejects_dependency_feature_spec() {
        assert!(validate_cargo_feature_name("other-pkg/evil-feature").is_err());
        assert!(validate_cargo_feature_name("dep:evil").is_err());
        assert!(validate_cargo_feature_name("evil\"]\nfoo").is_err());
    }
}
