//! Retention tests for the toolchain cache root.
//!
//! Every one of these drives `purge_toolchain_cache_root` with an explicit cutoff rather than
//! `purge_stale_toolchain_caches`'s clock-derived one, so "this directory is inside the grace
//! window" is stated by the test instead of waited for. `reclaim_cutoff` is pinned separately, so
//! the wiring between the two is not left untested. ~keep

use super::*;

/// A toolchain cache generation, complete with the subdirectory a real one carries and the use
/// stamp a run leaves behind. Sleeps afterwards so successive calls are ordered by modification
/// time on any filesystem with sub-10ms timestamp resolution -- the retention policy is
/// "most recently used", so a test that cannot tell two generations apart in time would assert
/// nothing about ordering. ~keep
fn generation(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.join("cargo-target")).expect("toolchain cache generation");
    std::fs::write(path.join(USE_STAMP), []).expect("use stamp");
    std::thread::sleep(Duration::from_millis(10));
    path
}

/// Nothing swept `.alef/snippets/cache/` before this module existed, so every key that fell out of
/// use kept its whole cargo target directory forever. The live key must survive regardless of its
/// age, exactly one stale generation is retained at `generations = 1`, and it must be the most
/// recently used one rather than whichever `read_dir` happened to yield first. ~keep
#[test]
fn only_the_live_key_and_the_newest_retained_generation_survive_a_sweep() {
    let directory = tempfile::tempdir().expect("temp directory");
    let root = toolchain_cache_root(directory.path());
    let oldest = generation(&root, "oldest");
    let middle = generation(&root, "middle");
    let newest = generation(&root, "newest");
    let live_key = "live".to_string();
    let live = generation(&root, &live_key);
    let stray = root.join("stray-file");
    std::fs::write(&stray, b"leftover").expect("stray file");

    purge_toolchain_cache_root(directory.path(), &BTreeSet::from([live_key]), SystemTime::now(), 1)
        .expect("sweeping the toolchain cache root");

    assert!(
        live.is_dir(),
        "the key this run is about to use must never be reclaimed"
    );
    assert!(
        newest.is_dir(),
        "the most recently used stale generation must be retained at generations = 1"
    );
    assert!(
        !middle.exists(),
        "a stale generation beyond the retention cap must be reclaimed"
    );
    assert!(
        !oldest.exists(),
        "a stale generation beyond the retention cap must be reclaimed"
    );
    assert!(!stray.exists(), "a stray file in the cache root must be removed");
}

/// The concurrency guarantee. Two alef processes can legitimately share one working directory, and
/// a directory a live `cargo` is writing into is not merely expensive to reclaim -- removing it
/// fails that build outright. A directory touched more recently than the cutoff is therefore off
/// limits even when it is not this run's key and no generations are retained at all. ~keep
#[test]
fn a_cache_a_concurrent_run_is_still_writing_into_is_never_reclaimed() {
    let directory = tempfile::tempdir().expect("temp directory");
    let root = toolchain_cache_root(directory.path());
    let concurrent = generation(&root, "another-processes-key");

    purge_toolchain_cache_root(directory.path(), &BTreeSet::new(), SystemTime::UNIX_EPOCH, 0)
        .expect("sweeping the toolchain cache root");

    assert!(
        concurrent.is_dir(),
        "a cache modified inside the grace window must survive a sweep that retains no generations"
    );
}

/// Pins the wiring the two tests above stub out: the cutoff production actually uses trails the
/// present by a whole run timeout plus the same grace the scratch sweep applies.
#[test]
fn the_reclaim_cutoff_trails_the_present_by_the_run_timeout_plus_the_abandoned_grace() {
    let timeout_secs = 120;
    let cutoff = reclaim_cutoff(timeout_secs).expect("a cutoff for an ordinary timeout");
    let age = SystemTime::now()
        .duration_since(cutoff)
        .expect("the cutoff is in the past");

    let grace = Duration::from_secs(timeout_secs + ABANDONED_GRACE_SECS);
    assert!(
        age >= grace,
        "the cutoff must be at least {grace:?} in the past, was {age:?}"
    );
    assert!(
        age < grace + Duration::from_secs(5),
        "the cutoff must trail the present by the grace window and nothing more, was {age:?}"
    );
}

/// A run whose snippets all hit the verdict cache launches no compiler, so nothing inside its
/// toolchain cache is written and its recency would otherwise still read as whenever it was last
/// compiled into. The use stamp is what keeps such a cache from sorting to the back of the
/// retention order and being reclaimed out from under the next run. ~keep
#[test]
fn stamping_a_cache_refreshes_its_recency_without_any_toolchain_write() {
    let directory = tempfile::tempdir().expect("temp directory");
    let caches = ToolchainCaches {
        root: directory.path().join("key"),
        go_build: directory.path().join("key/go-build"),
        zig_global: directory.path().join("key/zig-global"),
        cargo_target: directory.path().join("key/cargo-target"),
    };
    std::fs::create_dir_all(&caches.cargo_target).expect("toolchain cache directories");
    let before = last_used(&caches.root);
    std::thread::sleep(Duration::from_millis(10));

    caches.mark_used();

    assert!(
        last_used(&caches.root) > before,
        "stamping a cache must make it more recently used than it was"
    );
}
