//! Test-only support shared across the whole crate.
//!
//! `cargo test` runs every `#[test]` as a thread inside one process, so any state a test mutates
//! through a process-global API -- like `std::env::set_current_dir` -- is shared mutable state
//! across every other test in the binary, not just the tests in the same module. Before this
//! module existed, four separate `CWD_LOCK` statics lived in `cli::cache`,
//! `cli::breaking_changes`, `cli::pipeline::version_tests`, and
//! `cli::pipeline::generate::generation` (plus an unguarded fifth lock local to
//! `bin_cli::all_commands_tests`), each correctly serializing the tests in its own module but
//! doing nothing to serialize against the other four -- so two cwd-mutating tests from different
//! modules could still run concurrently and race. [`CWD_LOCK`] is the one lock every cwd-mutating
//! test in this crate now shares. ~keep

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The single lock serializing every test in this crate that mutates the process-global current
/// directory. See the module docs for why one shared lock is required rather than one per module.
pub(crate) static CWD_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that enters `dir` as the process current directory for its lifetime and restores
/// the original directory on drop -- including when the guarded scope panics, since `Drop` still
/// runs while a panic unwinds. Holds [`CWD_LOCK`] for its entire lifetime, so at most one
/// `CwdGuard` is ever live across the whole crate at a time.
///
/// A poisoned lock (an earlier guard's scope panicked while holding it) is still acquired: one
/// panicking test must not cascade into every other cwd-mutating test failing on a poisoned
/// mutex, and the poison carries no invalidated data here -- the guard that poisoned the lock had
/// already restored its own original directory via `Drop` before the panic finished unwinding
/// through it.
pub(crate) struct CwdGuard {
    _lock: MutexGuard<'static, ()>,
    original: PathBuf,
}

impl CwdGuard {
    /// Locks [`CWD_LOCK`] and enters `dir` as the process current directory, returning a guard
    /// that restores the original directory when dropped.
    pub(crate) fn enter(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original = std::env::current_dir().expect("read current directory");
        std::env::set_current_dir(dir).expect("enter directory");
        Self { _lock: lock, original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

/// `cargo sort --check` conformance for the table ORDER of a generated `Cargo.toml`.
///
/// Consumers gate CI on `cargo sort --check --workspace`, so every manifest alef emits has to
/// already be in cargo-sort's canonical table order. This module encodes the ordering RULE
/// rather than any one expected manifest, so a table a future emitter adds is covered without
/// touching this file. ~keep
pub(crate) mod cargo_sort_order {
    /// cargo-sort's `DEF_TABLE_ORDER`, verbatim from its `src/fmt.rs` at v2.1.4 (the version
    /// pinned in CI). Tables absent from this list -- `lints`, `profile`, `patch`, `badges` --
    /// are sorted AFTER every listed one, which is why `[lints.*]` must be emitted last and not
    /// tucked between `[package]` and `[dependencies]`. ~keep
    pub(crate) const DEF_TABLE_ORDER: &[&str] = &[
        "package",
        "workspace",
        "lib",
        "bin",
        "features",
        "dependencies",
        "build-dependencies",
        "dev-dependencies",
    ];

    /// Split a table header's inner text on `.`, treating quoted spans as opaque so a dotted
    /// cfg predicate (`target.'cfg(target_os = "x.y")'.dependencies`) stays one segment. ~keep
    fn header_segments(inner: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        for character in inner.chars() {
            match quote {
                Some(open) if character == open => quote = None,
                Some(_) => current.push(character),
                None if character == '\'' || character == '"' => quote = Some(character),
                None if character == '.' => segments.push(std::mem::take(&mut current)),
                None => current.push(character),
            }
        }
        segments.push(current);
        segments
    }

    fn rank_of(name: &str) -> usize {
        DEF_TABLE_ORDER
            .iter()
            .position(|table| *table == name)
            .unwrap_or(DEF_TABLE_ORDER.len())
    }

    /// Sort key cargo-sort effectively assigns a top-level table header.
    ///
    /// The first segment picks the group, because a subtable is repositioned immediately after
    /// its parent (`[package.metadata.*]` rides along with `[package]`). `[target.*]` is
    /// cargo-sort's one special case: its nested dependency table is grouped with that
    /// dependency KIND rather than sorted under `target`, and lands just after the plain table
    /// of the same kind -- hence the second tuple element. Every unlisted table shares the
    /// trailing rank, so their order relative to each other is unconstrained, matching
    /// cargo-sort's preservation of document order among them. ~keep
    fn table_sort_key(inner: &str) -> (usize, u8) {
        let segments = header_segments(inner);
        let first = segments.first().map(String::as_str).unwrap_or_default();
        if first == "target" {
            let kind = segments.last().map(String::as_str).unwrap_or_default();
            return (rank_of(kind), 1);
        }
        (rank_of(first), 0)
    }

    /// Assert every table header in `manifest` appears in cargo-sort's canonical order.
    ///
    /// `label` identifies the manifest in the failure message.
    pub(crate) fn assert_canonical_table_order(label: &str, manifest: &str) {
        let mut previous: Option<((usize, u8), &str)> = None;
        let mut header_count = 0usize;
        for line in manifest.lines() {
            let trimmed = line.trim();
            let Some(inner) = trimmed.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) else {
                continue;
            };
            let inner = inner
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(inner);
            header_count += 1;
            let key = table_sort_key(inner);
            if let Some((previous_key, previous_header)) = previous {
                assert!(
                    key >= previous_key,
                    "{label}: table `{trimmed}` must not follow `{previous_header}` -- cargo-sort \
                     orders tables {DEF_TABLE_ORDER:?} first and every other table after them, so \
                     `cargo sort --check` would reorder this manifest and fail it:\n{manifest}"
                );
            }
            previous = Some((key, trimmed));
        }
        assert!(
            header_count > 0,
            "{label}: no table headers found, so this check examined nothing:\n{manifest}"
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Guards the checker itself: it must REJECT the exact layout that broke consumers --
        /// `[lints.clippy]` emitted between `[package]` and `[dependencies]`. Without this, a
        /// checker that never fails is indistinguishable from a fixed emitter. ~keep
        #[test]
        fn should_reject_lints_table_placed_before_dependencies() {
            let manifest = "[package]\nname = \"demo\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\n\n\
                            [dependencies]\nserde = \"1\"\n";
            let result = std::panic::catch_unwind(|| assert_canonical_table_order("demo", manifest));
            assert!(result.is_err(), "checker must reject lints emitted before dependencies");
        }

        #[test]
        fn should_accept_lints_table_placed_last() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\n\n\
                            [lints.clippy]\ndbg_macro = \"deny\"\n";
            assert_canonical_table_order("demo", manifest);
        }

        /// `[package.metadata.*]` rides with `[package]`, and a `[target.*.dependencies]` block
        /// sits with the plain `[dependencies]` table rather than after `[dev-dependencies]`.
        #[test]
        fn should_accept_subtables_and_target_dependency_blocks() {
            let manifest = "[package]\nname = \"demo\"\n\n[package.metadata.cargo-machete]\n\
                            ignored = []\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[features]\n\
                            default = []\n\n[dependencies]\nserde = \"1\"\n\n\
                            [target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n\n\
                            [build-dependencies]\ncc = \"1\"\n\n[dev-dependencies]\n\
                            tempfile = \"3\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\n";
            assert_canonical_table_order("demo", manifest);
        }

        #[test]
        fn should_reject_features_table_placed_after_dependencies() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\n\n\
                            [features]\ndefault = []\n";
            let result = std::panic::catch_unwind(|| assert_canonical_table_order("demo", manifest));
            assert!(
                result.is_err(),
                "checker must reject features emitted after dependencies"
            );
        }
    }
}
