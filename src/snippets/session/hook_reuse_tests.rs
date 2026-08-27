//! Several configured session targets legitimately describe one physical package: `kotlin` and
//! `kotlin_android` both resolve to `Language::Kotlin` over a single Gradle project, and
//! `typescript`/`node`/`wasm` all resolve to `Language::TypeScript` over a single npm package (see
//! `Language::from_session_target`). Each target carries its own copy of that package's `before`
//! hook, and activation used to run every copy -- one `./gradlew assembleDebug` per target,
//! sequentially, before a single snippet could be validated. When the hook outran `timeout_secs`,
//! the run paid that whole timeout once per target and reported one preparation failure per
//! target, which is what a reader sees as "the hook ran twice".

use super::*;
use std::collections::BTreeMap;

/// Appends one line to `marker`. Line-counted rather than byte-counted because `cmd`'s `echo`
/// terminates with CRLF and `sh`'s does not. ~keep
fn counting_hook(marker: &Path) -> String {
    format!("echo ran >> {}", marker.display())
}

#[cfg(unix)]
fn sleeping_hook(seconds: u64) -> String {
    format!("sleep {seconds}")
}

/// See `preparation_error_tests::sleep_hook`: `timeout /t` cannot run without a console, so a
/// loopback `ping` is the console-free `cmd` sleep. ~keep
#[cfg(windows)]
fn sleeping_hook(seconds: u64) -> String {
    format!("ping -n {} 127.0.0.1", seconds + 1)
}

fn spec(language: Language, working_directory: &Path, before: String, env: BTreeMap<String, String>) -> SessionSpec {
    SessionSpec {
        language,
        working_directory: working_directory.to_path_buf(),
        manifest: None,
        before: vec![before],
        env,
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: BTreeMap::new(),
    }
}

fn times_run(marker: &Path) -> usize {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

/// The defect itself, asserted on the hook's own side effect rather than on any internal
/// bookkeeping: the shell command must be executed once, not once per target that named it. ~keep
#[test]
fn one_hook_serves_every_target_over_the_same_package() {
    let package = tempfile::tempdir().expect("package directory");
    let markers = tempfile::tempdir().expect("marker directory");
    let marker = markers.path().join("runs.txt");
    let hook = counting_hook(&marker);
    let mut specs = HashMap::new();
    specs.insert(
        "kotlin".to_string(),
        spec(Language::Kotlin, package.path(), hook.clone(), BTreeMap::new()),
    );
    specs.insert(
        "kotlin_android".to_string(),
        spec(Language::Kotlin, package.path(), hook, BTreeMap::new()),
    );

    let prepared = prepare_sessions_isolated(&specs, 30);

    assert!(prepared.errors.is_empty(), "both targets must prepare: {:?}", {
        prepared.errors.keys().collect::<Vec<_>>()
    });
    assert_eq!(prepared.sessions.len(), 2, "both targets must still get a session");
    assert_eq!(times_run(&marker), 1, "the shared before hook must run exactly once");
}

/// A hook the two targets do not actually share must still run for each of them. Without this the
/// fix above could be satisfied by never running a second hook at all. ~keep
#[test]
fn a_hook_over_a_different_package_is_not_reused() {
    let first_package = tempfile::tempdir().expect("first package");
    let second_package = tempfile::tempdir().expect("second package");
    let markers = tempfile::tempdir().expect("marker directory");
    let first = markers.path().join("first.txt");
    let second = markers.path().join("second.txt");
    let mut specs = HashMap::new();
    specs.insert(
        "node".to_string(),
        spec(
            Language::TypeScript,
            first_package.path(),
            counting_hook(&first),
            BTreeMap::new(),
        ),
    );
    specs.insert(
        "wasm".to_string(),
        spec(
            Language::TypeScript,
            second_package.path(),
            counting_hook(&second),
            BTreeMap::new(),
        ),
    );

    let prepared = prepare_sessions_isolated(&specs, 30);

    assert!(prepared.errors.is_empty(), "both targets must prepare");
    assert_eq!(times_run(&first), 1, "the first package's hook must run");
    assert_eq!(times_run(&second), 1, "the second package's hook must run");
}

/// The environment is part of what the shell sees, so two targets that hand the same command
/// different variables are asking for two different builds and must get them. ~keep
#[test]
fn a_hook_handed_a_different_environment_is_not_reused() {
    let package = tempfile::tempdir().expect("package directory");
    let markers = tempfile::tempdir().expect("marker directory");
    let marker = markers.path().join("runs.txt");
    let hook = counting_hook(&marker);
    let mut specs = HashMap::new();
    specs.insert(
        "node".to_string(),
        spec(Language::TypeScript, package.path(), hook.clone(), BTreeMap::new()),
    );
    specs.insert(
        "wasm".to_string(),
        spec(
            Language::TypeScript,
            package.path(),
            hook,
            BTreeMap::from([("ALEF_SNIPPET_PROFILE".to_string(), "wasm".to_string())]),
        ),
    );

    let prepared = prepare_sessions_isolated(&specs, 30);

    assert!(prepared.errors.is_empty(), "both targets must prepare");
    assert_eq!(times_run(&marker), 2, "a differing environment must not reuse a hook");
}

/// The observed failure mode, and the reason reuse has to replay failures rather than drop them:
/// two targets over one Gradle project each paid the full `timeout_secs` for the same hook, so a
/// 30-minute budget cost an hour before any snippet ran. Every target must still be reported as
/// unprepared with `ordering` set, and the wall clock must show one timeout, not two. ~keep
#[test]
fn a_timed_out_hook_is_charged_once_and_still_fails_every_target() {
    let package = tempfile::tempdir().expect("package directory");
    let hook = sleeping_hook(30);
    let mut specs = HashMap::new();
    specs.insert(
        "kotlin".to_string(),
        spec(Language::Kotlin, package.path(), hook.clone(), BTreeMap::new()),
    );
    specs.insert(
        "kotlin_android".to_string(),
        spec(Language::Kotlin, package.path(), hook, BTreeMap::new()),
    );
    let timeout_secs = 2;
    let started = std::time::Instant::now();

    let prepared = prepare_sessions_isolated(&specs, timeout_secs);
    let elapsed = started.elapsed();

    for target in ["kotlin", "kotlin_android"] {
        let error = prepared
            .errors
            .get(target)
            .unwrap_or_else(|| panic!("{target} must be reported as unprepared"));
        assert!(
            error.ordering,
            "{target} must keep the ordering classification of the hook it shared: {}",
            error.message
        );
    }
    assert!(
        prepared.sessions.is_empty(),
        "a target whose hook timed out must not be handed a session"
    );
    crate::test_support::assert_elapsed_under(
        "the shared hook's timeout was paid more than once",
        elapsed,
        std::time::Duration::from_secs(timeout_secs * 2),
    );
}
