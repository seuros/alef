//! The generated bridge crate's `build.rs` must repair cfg gates only after explicit regeneration.
//!
//! flutter_rust_bridge is not feature-aware: it bakes a `wire__crate__<name>_impl` wrapper and a
//! dispatch arm for every `pub fn` it can see, including ones behind `#[cfg(feature = "...")]`.
//! Compiled under a reduced feature set (Android builds with `--no-default-features`), the gated
//! definition disappears from `lib.rs` while the ungated caller in `frb_generated.rs` remains, and
//! the crate fails with `E0425: cannot find function ... in the crate root`.
//!
//! `carry_frb_cfg_gates()` is the repair: it reads the gates out of `lib.rs` — alef's own emitted
//! source of truth for which functions are gated — and re-applies them to `frb_generated.rs`.
//! Because that is a source-tree mutation, an ordinary Cargo build must return before reaching it;
//! the repair belongs only in the successful explicit FRB regeneration arm.

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
fn should_carry_frb_cfg_gates_after_successful_explicit_regeneration() {
    let statements = main_statements(&generated_build_rs());
    let guard = opt_in_guard_index(&statements).expect("main() must keep the opt-in regeneration guard");
    let regeneration = statements
        .iter()
        .skip(guard + 1)
        .find_map(|statement| match statement {
            Stmt::Expr(Expr::Match(expression), _) => Some(expression),
            _ => None,
        })
        .expect("main() must match on the explicit FRB regeneration result");
    use quote::ToTokens;
    let success_index = regeneration
        .arms
        .iter()
        .position(|arm| arm.pat.to_token_stream().to_string().contains("status . success"))
        .expect("FRB regeneration must have a status.success() arm");
    assert!(
        regeneration.arms[success_index]
            .body
            .to_token_stream()
            .to_string()
            .contains("carry_frb_cfg_gates"),
        "successful explicit FRB regeneration must restore feature cfg gates"
    );
    for (index, arm) in regeneration.arms.iter().enumerate() {
        if index != success_index {
            assert!(
                !arm.body.to_token_stream().to_string().contains("carry_frb_cfg_gates"),
                "failed or unavailable FRB regeneration must not mutate committed output"
            );
        }
    }
}
