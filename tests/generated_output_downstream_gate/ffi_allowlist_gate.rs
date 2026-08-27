//! The reverse half of `emitted_tree_passes_clippy`'s check.
//!
//! `emitted_tree_passes_clippy` proves the FFI crate's crate-level `#![allow(...)]` list
//! (`src/backends/ffi/gen_bindings/lib_rs.rs`) is not too *narrow*: if the emitted code trips a
//! lint nothing in the list names, `-D warnings` fails the build. Nothing anywhere proves the
//! list is not too *wide*: an entry whose triggering code pattern the emitter stopped producing
//! sits in `lib_rs.rs` forever, unverified and silently permissive for a lint that can no
//! longer fire.
//!
//! `src/backends/ffi/gen_bindings/tests/clippy_allowlist.rs`'s
//! `crate_level_allow_list_does_not_carry_dead_entries` already pins this claim as a fast,
//! in-process text search over generated output for [`KNOWN_DEAD_LINT_NAMES`]. That is real
//! coverage, but it is a claim about *text*, not about clippy's own opinion -- it would stay
//! green even if some other, unrelated change made the crate-level entry load-bearing again
//! through a path the text search does not think to look for. This lane holds the same claim to
//! the actual tool: strip every [`KNOWN_DEAD_LINT_NAMES`] entry from the emitted crate's
//! crate-level allow list, force each one back to `warn` explicitly via `-W` (so an
//! allow-by-default lint cannot look dead merely because nothing asked clippy to check it), and
//! run `cargo clippy` once. A name that fires is not dead and must not be in
//! [`KNOWN_DEAD_LINT_NAMES`] (or, symmetrically, in `lib_rs.rs`'s allow list) at the same time.
//!
//! `dropping_references` is the newest member of [`KNOWN_DEAD_LINT_NAMES`]: every explicit
//! `drop(...)` call the FFI templates emit drops an owned value (`free_bytes.jinja`'s
//! `Box::<[u8]>::from_raw(..)`, `free_string.jinja`'s `CString::from_raw(..)`,
//! `handle_registry.rs.jinja`'s `self.take::<T>(handle)?` and its owned `MutexGuard`, and
//! `orchestration.rs`'s `std::mem::drop(obj)` for a method literally named `drop`, where `obj`
//! is always bound owned via `null_check_self_owned.jinja`'s `take_handle::<T>(this)`), never a
//! reference -- so the crate-level entry had nothing left to allow.
//!
//! [`dead_entry_check_catches_a_planted_orphan`] is this lane's own anti-vacuity proof: it
//! falsely claims `clippy::missing_safety_doc` -- an entry `lib_rs.rs` still actively allows,
//! and one this lane's own `KNOWN_DEAD_LINT_NAMES` audit deliberately left out because the
//! fixture genuinely trips it (confirmed by hand: stripping the *entire* crate-level list and
//! running plain `cargo clippy` reports it) -- is dead, and asserts the lane reports the claim
//! as false. A lane that cannot go red on a live entry masquerading as dead has examined
//! nothing. `clippy::collapsible_if` filled this role until the FFI free-handle guards were
//! collapsed into let-chains (commit `8893f7550`), which made it genuinely dead too; picking a
//! lint that is still demonstrably live is the fix, not relaxing what this test asserts.
//!
//! This lane intentionally does **not** assert that every entry `lib_rs.rs` currently allows
//! fires against this one synthetic fixture. Most of those entries are defensive for code
//! shapes this minimal fixture does not exercise (async methods, byte-vec returns, capsule
//! types, service APIs, visitor callbacks, trait bridges) -- `clippy_allowlist.rs`'s own audit
//! trail notes needing a *second*, differently-configured `alef generate` run just to reach the
//! enum-context cast site for `unnecessary_cast`. A blanket "everything must fire in one
//! fixture" assertion would be false for reasons that have nothing to do with the entry being
//! dead, which is exactly the failure mode this file elsewhere calls out: a check that reports
//! something wrong for the wrong reason trains people to ignore it. [`KNOWN_DEAD_LINT_NAMES`] is
//! the auditable seed of entries someone has already looked at and can defend with static
//! reasoning (see the doc on `crate_level_allow_list_does_not_carry_dead_entries`); growing it
//! is how a newly-suspected-dead entry gets the same real-tool proof `dropping_references` did
//! here, without this lane claiming coverage over entries nobody has audited yet.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{CARGO, EmittedTree, Sabotage, clippy_manifest_dirs, emit_tree, resolve_tools};

/// Lint names claimed dead at the FFI crate's crate level, with the reasoning for each living
/// next to the text-search assertion that first pinned it:
/// `src/backends/ffi/gen_bindings/tests/clippy_allowlist.rs`'s
/// `crate_level_allow_list_does_not_carry_dead_entries`. Every name here must also be absent
/// from `src/backends/ffi/gen_bindings/lib_rs.rs`'s active `#![allow(...)]` lists -- that half
/// is what the text-search test enforces; this file enforces that the *reason* still holds
/// against a real `cargo clippy` run.
///
/// `missing_docs` is deliberately not here: its removal was never a "no code triggers this"
/// claim (this lane's `-W` re-enable would in fact make it fire -- confirmed by hand), it was
/// "a plain consumer `-D warnings` run can never see it fire, because `missing_docs` starts
/// allow-by-default under rustc and `-D warnings` only escalates warnings, it does not enable
/// anything." That is a claim about lint *escalation*, not about a generated pattern's
/// existence, and this lane's `-W`-forcing methodology tests exactly the thing that claim says
/// does not matter. The text-search test already covers it correctly on its own terms.
const KNOWN_DEAD_LINT_NAMES: &[&str] = &[
    "clippy::too_many_arguments",
    "clippy::useless_conversion",
    "clippy::unnecessary_cast",
    "dropping_references",
];

/// Delete every crate-level `#![allow(...)]` attribute, single-line or wrapped across
/// multiple lines, leaving the rest of the source untouched.
///
/// `lib_rs.rs` renders its longer allow lists as `#![allow(\n    clippy::foo,\n    ...\n)]`
/// once the list has enough entries to wrap (see `src/backends/ffi/gen_bindings/lib_rs.rs`).
/// A filter that only matched a single line starting with `#![allow(` *and* ending with `)]`
/// on that same line left a wrapped block's opening line, its bare lint names, and its
/// closing `)]` all in the file -- syntactically valid and still suppressing every lint
/// inside it, so the strip silently did nothing to that block. That is exactly why this
/// lane's `dead_entry_check_catches_a_planted_orphan` reported an empty fired-lint set: the
/// crate-level `#![allow(..., clippy::collapsible_if)]` block is multi-line, survived the old
/// filter untouched, and its source-level `#![allow(...)]` overrides a command-line `-W`. ~keep
fn strip_crate_level_allows(source: &str) -> String {
    let mut stripped = String::new();
    let mut in_wrapped_allow_block = false;
    for line in source.lines() {
        if in_wrapped_allow_block {
            if line.trim_end().ends_with(")]") {
                in_wrapped_allow_block = false;
            }
            continue;
        }
        if line.starts_with("#![allow(") {
            if !line.trim_end().ends_with(")]") {
                in_wrapped_allow_block = true;
            }
            continue;
        }
        stripped.push_str(line);
        stripped.push('\n');
    }
    stripped
}

#[cfg(test)]
mod strip_crate_level_allows_tests {
    use super::strip_crate_level_allows;

    #[test]
    fn strips_a_single_line_allow_block() {
        let source = "#![allow(dead_code, unused_imports)]\nfn main() {}\n";
        assert_eq!(strip_crate_level_allows(source), "fn main() {}\n");
    }

    #[test]
    fn strips_a_wrapped_multi_line_allow_block() {
        let source = "#![allow(\n    clippy::foo,\n    clippy::bar\n)]\nfn main() {}\n";
        assert_eq!(strip_crate_level_allows(source), "fn main() {}\n");
    }

    #[test]
    fn strips_two_allow_blocks_of_mixed_shape() {
        let source = "#![allow(dead_code)]\n#![allow(\n    clippy::foo,\n    clippy::bar\n)]\n\nfn main() {}\n";
        assert_eq!(strip_crate_level_allows(source), "\nfn main() {}\n");
    }

    #[test]
    fn leaves_a_non_crate_level_allow_untouched() {
        let source = "#[allow(dead_code)]\nfn main() {}\n";
        assert_eq!(strip_crate_level_allows(source), source);
    }
}

/// Every `warning`-level lint code `cargo clippy` reports for the crate at `dir`, with `lints`
/// explicitly re-enabled via `-W` so an allow-by-default lint cannot look dead merely because
/// nothing asked clippy to check it.
fn fired_lint_codes(dir: &Path, lints: &[&str]) -> BTreeSet<String> {
    let mut args: Vec<&str> = vec!["clippy", "--all-targets", "--message-format=json", "--"];
    for lint in lints {
        args.push("-W");
        args.push(lint);
    }
    let output = Command::new(CARGO.program)
        .args(&args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|error| panic!("running `cargo {}` in {}: {error}", args.join(" "), dir.display()));

    let mut codes = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message.get("reason").and_then(|value| value.as_str()) != Some("compiler-message") {
            continue;
        }
        if let Some(code) = message.pointer("/message/code/code").and_then(|value| value.as_str()) {
            codes.insert(code.to_string());
        }
    }
    codes
}

/// The emitted FFI crate's own manifest directory, out of every clippy-lane language's
/// directory `clippy_manifest_dirs` returns.
fn ffi_crate_dir(tree: &EmittedTree) -> PathBuf {
    clippy_manifest_dirs(tree)
        .into_iter()
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("-ffi"))
        })
        .expect("emitted tree has no `*-ffi` crate directory; `alef generate` may have changed the FFI output layout")
}

/// Strip the emitted FFI crate's crate-level allow list, force every name in `claimed_dead`
/// back to `warn`, and return the ones that fired at least once -- claims real clippy
/// disagrees with.
fn lint_names_that_still_fire(tree: &EmittedTree, claimed_dead: &[&str]) -> Vec<String> {
    assert!(
        !claimed_dead.is_empty(),
        "checked an empty lint-name set, which would pass having examined nothing"
    );
    let ffi_dir = ffi_crate_dir(tree);
    let lib_rs_path = ffi_dir.join("src/lib.rs");
    let original =
        std::fs::read_to_string(&lib_rs_path).unwrap_or_else(|error| panic!("read {}: {error}", lib_rs_path.display()));

    std::fs::write(&lib_rs_path, strip_crate_level_allows(&original))
        .unwrap_or_else(|error| panic!("write {}: {error}", lib_rs_path.display()));

    let fired = fired_lint_codes(&ffi_dir, claimed_dead);
    claimed_dead
        .iter()
        .filter(|lint| fired.contains(**lint))
        .map(|lint| (*lint).to_string())
        .collect()
}

#[test]
#[ignore = "compiles the emitted FFI crate twice; run via the CI gate job"]
fn ffi_crate_level_allow_list_known_dead_entries_stay_dead() {
    resolve_tools(&[&CARGO]);
    let tree = emit_tree(Sabotage::None);
    let still_firing = lint_names_that_still_fire(&tree, KNOWN_DEAD_LINT_NAMES);
    assert!(
        still_firing.is_empty(),
        "KNOWN_DEAD_LINT_NAMES in tests/generated_output_downstream_gate/ffi_allowlist_gate.rs \
         claims these lints never fire against the emitted FFI crate, but a real cargo clippy \
         run (with the crate-level allow removed and the lint force-enabled via -W) disagrees: \
         {still_firing:?}\n\
         Either the claim in KNOWN_DEAD_LINT_NAMES is stale, or the entry belongs back in \
         src/backends/ffi/gen_bindings/lib_rs.rs's active #![allow(...)] lists.",
    );
}

#[test]
#[ignore = "compiles the emitted FFI crate twice; run via the CI gate job"]
fn dead_entry_check_catches_a_planted_orphan() {
    resolve_tools(&[&CARGO]);
    let tree = emit_tree(Sabotage::None);
    // `clippy::missing_safety_doc` is not in KNOWN_DEAD_LINT_NAMES because it is not dead: it
    // is one of `lib_rs.rs`'s active crate-level allows (see the second `#![allow(...)]` block
    // there), and this fixture's generated constructor and free-function wrappers -- e.g.
    // `toolkit_session_new`, `toolkit_summarize`, `toolkit_count_tokens` -- are `pub unsafe
    // extern "C" fn`s whose doc comments carry no `# Safety` section at all (unlike the
    // handle-`_free` wrappers, which do), so clippy's `missing_safety_doc` fires on each one.
    // Confirmed by hand: stripping the whole crate-level list and running plain `cargo clippy`
    // reports it. `clippy::collapsible_if` filled this role until the FFI free-handle guards
    // were collapsed into let-chains (commit `8893f7550`), which made the nested if/if-let
    // shape it used to catch stop being emitted and left it genuinely dead -- exactly the class
    // of drift this lane exists to catch, just aimed at its own planted orphan instead of a
    // real crate-level entry. Falsely claiming `missing_safety_doc` dead here and asserting the
    // lane still reports it as firing is the guard against a check that passes merely because
    // it always agrees with the claim it is handed. ~keep
    let still_firing = lint_names_that_still_fire(&tree, &["clippy::missing_safety_doc"]);
    assert_eq!(
        still_firing,
        vec!["clippy::missing_safety_doc".to_string()],
        "falsely claiming a lint that still fires in this fixture is dead must be reported by \
         this lane, or the lane is vacuous: got {still_firing:?}"
    );
}
