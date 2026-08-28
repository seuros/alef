//! `--jobs`/`-j` already caps the process-wide rayon global pool at startup
//! (`rayon::ThreadPoolBuilder::num_threads(cli.jobs).build_global()` in `lib.rs`), and every other
//! parallel pipeline stage reads that same global pool through a bare `par_iter()`. Snippet
//! validation did not: `RunnerConfig::default()`'s `parallelism` field built its own dedicated
//! pool sized from `std::thread::available_parallelism()`, a raw CPU count blind to `--jobs`, so
//! `alef all --clean`'s snippet-validation stage always dispatched at full host width.
//!
//! This test pins the fix without touching global rayon state (which a shared test binary cannot
//! safely mutate more than once): `available_parallelism()`'s replacement,
//! `rayon::current_num_threads()`, is sensitive to the *ambient* rayon registry a caller runs
//! inside of, not just the true global one, so running the same call inside a scoped, differently
//! sized pool observes that pool's size instead. Reverting to
//! `std::thread::available_parallelism()` makes this fail on any machine with more logical CPUs
//! than the scoped pool below -- true of essentially every CI runner and dev machine.

use super::*;

#[test]
fn default_parallelism_reflects_the_ambient_rayon_pool_size_not_a_raw_cpu_count() {
    const SCOPED_POOL_SIZE: usize = 2;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(SCOPED_POOL_SIZE)
        .build()
        .expect("scoped thread pool");

    let parallelism = pool.install(|| RunnerConfig::default().parallelism);

    assert_eq!(
        parallelism, SCOPED_POOL_SIZE,
        "RunnerConfig::default().parallelism must read the ambient rayon pool's size (which --jobs \
         controls at startup), not an independent raw CPU count"
    );
}
