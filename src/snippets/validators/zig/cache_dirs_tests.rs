//! `apply_cache_dirs` used to set `ZIG_GLOBAL_CACHE_DIR` unconditionally, and every real caller
//! then called `ValidationSession::apply` immediately afterward, whose own `ZIG_GLOBAL_CACHE_DIR`
//! write silently shadowed it (`std::process::Command::env` is last-write-wins). That made the
//! session-shared global cache depend on an ordering invariant nothing enforced. These tests pin
//! the fix -- `apply_cache_dirs` now leaves `ZIG_GLOBAL_CACHE_DIR` genuinely unset whenever a
//! session is present, so the sharing no longer depends on caller order.

use super::apply_cache_dirs;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::Language;
use std::path::PathBuf;

#[test]
fn without_a_session_both_caches_are_scoped_to_the_snippet_directory() {
    let root = tempfile::tempdir().expect("temporary root");
    let mut command = std::process::Command::new("zig");
    apply_cache_dirs(&mut command, root.path(), None);

    let configured: Vec<_> = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_string_lossy().into_owned(), PathBuf::from(value))))
        .collect();

    assert_eq!(
        configured,
        vec![
            ("ZIG_GLOBAL_CACHE_DIR".to_string(), root.path().join("zig-global-cache")),
            ("ZIG_LOCAL_CACHE_DIR".to_string(), root.path().join("zig-local-cache")),
        ]
    );
}

/// Pins the fix's whole point: with a session, `apply_cache_dirs` must leave
/// `ZIG_GLOBAL_CACHE_DIR` unset -- not set-then-shadowed by call order, genuinely unset -- so only
/// `ValidationSession::apply_environment`'s fingerprint-scoped, session-shared directory ever
/// reaches the process. Before this fix, deleting or reordering the `session.apply(&mut command)`
/// call after `apply_cache_dirs` (both real call sites do this today, but nothing enforced it)
/// would have silently regressed every zig snippet in a session back to a fresh, unshared,
/// `--clean`-cold global cache per snippet, with no test to catch it. This test would still catch
/// that regression even if a future caller forgot to call `session.apply` afterward, because it
/// asserts on `apply_cache_dirs`'s own output alone. ~keep
#[test]
fn with_a_session_apply_cache_dirs_leaves_the_global_cache_unset() {
    let root = tempfile::tempdir().expect("temporary root");
    let session = ValidationSession {
        language: Language::Zig,
        working_directory: root.path().to_path_buf(),
        manifest: None,
        fingerprint: "session-shared-global-cache-fixture".into(),
        env: std::collections::BTreeMap::new(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: std::collections::BTreeMap::new(),
    };
    let mut command = std::process::Command::new("zig");
    apply_cache_dirs(&mut command, root.path(), Some(&session));

    let configured: std::collections::BTreeMap<String, PathBuf> = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_string_lossy().into_owned(), PathBuf::from(value))))
        .collect();

    assert!(
        !configured.contains_key("ZIG_GLOBAL_CACHE_DIR"),
        "apply_cache_dirs must not set ZIG_GLOBAL_CACHE_DIR when a session is present: {configured:?}"
    );
    assert_eq!(configured.get("ZIG_LOCAL_CACHE_DIR"), Some(&root.path().join("zig-local-cache")));

    // The other half of the invariant: once `session.apply` runs (as every real caller does
    // immediately afterward), the global cache resolves to the session's own fingerprint-scoped
    // directory -- not the scratch directory `apply_cache_dirs` was given.
    session.apply(&mut command);
    let after_session_apply: std::collections::BTreeMap<String, PathBuf> = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_string_lossy().into_owned(), PathBuf::from(value))))
        .collect();
    let shared_global_cache = after_session_apply
        .get("ZIG_GLOBAL_CACHE_DIR")
        .expect("session.apply must set ZIG_GLOBAL_CACHE_DIR");
    assert!(
        shared_global_cache.starts_with(root.path().join(".alef/snippets/cache").join(&session.fingerprint)),
        "the global cache must be the session's fingerprint-scoped, session-shared directory, not a \
         scratch directory: {}",
        shared_global_cache.display()
    );
}
