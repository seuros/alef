#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

//! The declared MSRV lives in exactly one place -- `Cargo.toml`'s `rust-version` -- and every
//! other statement of it must agree.
//!
//! alef 0.68.0 shipped to crates.io claiming Rust 1.85 in its README while the source used
//! `if let` guards, stable only from 1.95. `rust-toolchain.toml` pins a much newer toolchain, so
//! CI compiled the crate with a compiler that accepted the feature and no job ever exercised the
//! claim -- a green build that examined nothing. An external user on 1.92 hit E0658 on three
//! files at install time (issue #262). ~keep

use std::path::Path;

/// `rust-version` as declared in `Cargo.toml`, the single source of truth.
fn declared_rust_version() -> String {
    let manifest = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("Cargo.toml is readable");
    manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("Cargo.toml declares rust-version")
        .to_string()
}

#[test]
fn readme_states_the_same_msrv_as_cargo_toml() {
    let declared = declared_rust_version();
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("README.md is readable");

    let prose = format!("Alef requires Rust {declared} or newer.");
    assert!(
        readme.contains(&prose),
        "README.md must say {prose:?} to match Cargo.toml's rust-version = {declared:?}"
    );

    let badge = format!("Rust-{declared}%2B");
    assert!(
        readme.contains(&badge),
        "README.md's version badge must encode {badge:?} to match Cargo.toml's \
         rust-version = {declared:?}"
    );

    let alt = format!("alt=\"Rust {declared}+\"");
    assert!(
        readme.contains(&alt),
        "README.md's version badge alt text must be {alt:?} to match Cargo.toml's \
         rust-version = {declared:?}"
    );
}

#[test]
fn the_ci_msrv_gate_resolves_its_toolchain_from_cargo_toml() {
    let workflow = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml"))
        .expect("ci.yml is readable");

    assert!(
        workflow.contains("\n  msrv:"),
        "ci.yml must define an `msrv` job -- without one, nothing compiles alef at its \
         declared floor and the number is free to drift again"
    );

    let job = workflow
        .split_once("\n  msrv:")
        .expect("the msrv job is a top-level job")
        .1;
    let job = job.split("\n  build:").next().unwrap_or(job);

    // Strip comments before asserting. The first version of this test matched the prose in
    // the job's own header comment and passed while the job hard-coded its toolchain -- a
    // guard that examined an explanation instead of the thing explained. ~keep
    let steps: String = job
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        steps.contains("Cargo.toml"),
        "the msrv job's steps must read Cargo.toml to resolve the toolchain, got:\n{steps}"
    );
    assert!(
        steps.contains("steps.msrv.outputs.version"),
        "the msrv job must compile with the toolchain it resolved from Cargo.toml, got:\n{steps}"
    );

    // A literal `cargo +1.88` would go stale silently the next time `rust-version` moves,
    // which is the same defect one layer up. ~keep
    let declared = declared_rust_version();
    assert!(
        !steps.contains(&format!("+{declared}")),
        "the msrv job must not pin the toolchain literally to {declared:?}, got:\n{steps}"
    );
}
