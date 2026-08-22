//! The generated bridge crate's `build.rs` must repair the committed FRB glue on every build.
//!
//! flutter_rust_bridge is not feature-aware: it bakes a `wire__crate__<name>_impl` wrapper and a
//! dispatch arm for every `pub fn` it can see, including ones behind `#[cfg(feature = "...")]`.
//! Compiled under a reduced feature set (Android builds with `--no-default-features`), the gated
//! definition disappears from `lib.rs` while the ungated caller in `frb_generated.rs` remains, and
//! the crate fails with `E0425: cannot find function ... in the crate root`.
//!
//! `carry_frb_cfg_gates()` is the repair: it reads the gates out of `lib.rs` — alef's own emitted
//! source of truth for which functions are gated — and re-applies them to `frb_generated.rs`. It
//! needs no external tool, touches only files inside the crate, and is idempotent.
//!
//! It used to be invoked only from the success arm of the `flutter_rust_bridge_codegen` spawn,
//! itself behind the opt-in `ALEF_FRB_REGENERATE_ON_BUILD` early return. Every configuration that
//! actually needs the repair — a normal build of the *committed* bridge, with FRB not installed —
//! took the early return or the `NotFound` arm and never reached it. Asserting only that the call
//! text appears somewhere in the file cannot catch that, so these tests assert reachability.

use super::cargo::emit_build_rs;
use syn::{Expr, Stmt};

fn generated_build_rs() -> String {
    emit_build_rs(
        "packages/dart/rust",
        "sample_router",
        "sample_router",
        "sample_router_dart",
    )
    .content
}

/// Index of the first top-level statement in `main` whose expression is a call to `name()`.
fn top_level_call_index(statements: &[Stmt], name: &str) -> Option<usize> {
    statements.iter().position(|statement| {
        let Stmt::Expr(Expr::Call(call), _) = statement else {
            return false;
        };
        let Expr::Path(path) = call.func.as_ref() else {
            return false;
        };
        path.path.is_ident(name)
    })
}

/// Index of the top-level `if !frb_regeneration_opted_in() { ... return; }` guard in `main`.
///
/// Matched on the statement's token text rather than its shape: the assertion is about which
/// statement returns early, not about how the condition happens to be spelled. ~keep
fn opt_in_guard_index(statements: &[Stmt]) -> Option<usize> {
    use quote::ToTokens;
    statements.iter().position(|statement| {
        let tokens = statement.to_token_stream().to_string();
        tokens.contains("frb_regeneration_opted_in") && tokens.contains("return")
    })
}

fn main_statements(source: &str) -> Vec<Stmt> {
    let file = syn::parse_file(source).expect("generated build.rs must be valid Rust");
    file.items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "main" => Some(function.block.stmts),
            _ => None,
        })
        .expect("generated build.rs must define fn main")
}

#[test]
fn should_call_carry_frb_cfg_gates_from_main_body_not_only_after_codegen() {
    let statements = main_statements(&generated_build_rs());
    assert!(
        top_level_call_index(&statements, "carry_frb_cfg_gates").is_some(),
        "carry_frb_cfg_gates() must be a top-level statement in main(): nested inside the \
         flutter_rust_bridge_codegen success arm it never runs for a build of the committed \
         bridge, which is exactly the build that needs the gates"
    );
}

#[test]
fn should_carry_frb_cfg_gates_before_the_regeneration_opt_in_returns() {
    let statements = main_statements(&generated_build_rs());
    let guard = opt_in_guard_index(&statements).expect("main() must keep the opt-in regeneration guard");
    let repair = top_level_call_index(&statements, "carry_frb_cfg_gates")
        .expect("main() must call carry_frb_cfg_gates() at top level");
    assert!(
        repair < guard,
        "carry_frb_cfg_gates() (statement {repair}) must run before the \
         ALEF_FRB_REGENERATE_ON_BUILD early return (statement {guard}); the default build path \
         returns there, so anything after it is unreachable in CI"
    );
}
