//! Sanctioned stdout writers for genuine command *results*.
//!
//! Everything alef emits splits into two channels:
//!
//! - **Diagnostics** — progress, status, warnings, errors — flow through `tracing`
//!   (stderr, level-filtered by `-v`/`-q`/`RUST_LOG`). No production code path calls
//!   `eprintln!`/`println!` directly; the `clippy::print_stdout`/`print_stderr` lints
//!   forbid it crate-wide.
//! - **Results** — a command's deliverable that a user pipes or parses (JSON reports,
//!   schema dumps, version listings, `diff` bodies) — go to stdout through this module.
//!
//! This module is the one place in the binary allowed to write to stdout, so the
//! result surface is auditable in a single file. Report-heavy commands with their own
//! stdout formatting (snippet tables, cache listings) instead carry a narrow
//! `#[allow(clippy::print_stdout)]` on the specific reporting function.
#![allow(clippy::print_stdout)]

/// Write one line of command result output to stdout.
pub(crate) fn line(text: impl std::fmt::Display) {
    println!("{text}");
}

/// Write a blank separator line to stdout.
pub(crate) fn blank() {
    println!();
}

/// Write a pre-serialized result payload (e.g. JSON) to stdout, followed by a newline.
pub(crate) fn payload(text: impl std::fmt::Display) {
    println!("{text}");
}

/// Write a result fragment to stdout without appending a newline.
///
/// For output whose pieces already carry their own line terminators (e.g. a unified
/// diff whose lines each end in `\n`), so a trailing newline must not be added.
pub(crate) fn fragment(text: impl std::fmt::Display) {
    print!("{text}");
}
