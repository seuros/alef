//! Exact-value proofs for the environment `test_apps_run` injects into every registry-mode
//! test-app command.
//!
//! `[crates.e2e.env]` values and the mock-server's `MOCK_SERVER_URL`/`MOCK_SERVERS`/per-fixture
//! URLs are EXACT values, not search paths. They used to be passed as
//! `run_command_streamed_with_envs`' `path_env`, which renders the PATH-style prepend guard
//! `export K='V'"${K:+:$K}"` into the `sh -c` text: harmless for `LD_LIBRARY_PATH`, corrupting
//! here, because whatever the operator's own shell already exported under that name gets
//! appended to the value alef set. A test app then resolved
//! `http://127.0.0.1:<port>/:<inherited>` and any strict comparison against the configured
//! value failed.
//!
//! Every test below spawns the real `test_apps_run` pipeline and reads back what the child
//! process actually saw, byte for byte. [`inherited_value_must_not_be_appended`] is the
//! negative control that makes these non-vacuous: it deliberately exports a WRONG value for
//! the same key in this process first, which is the only condition under which the two
//! mechanisms differ -- without it, a `path_env` regression would pass every assertion here. ~keep

use super::*;
use std::sync::{Mutex, MutexGuard};

/// The key these tests export process-globally. Deliberately unique to this module: the
/// codebase has twice had to fix a "two independent locks guarding one process-global"
/// hazard (see `test_support::SKIP_COMMANDS_LOCK`), and the way this module avoids adding a
/// third is by owning a variable no other test in the crate touches. ~keep
const EXACT_KEY: &str = "ALEF_TEST_APPS_ENV_EXACTNESS";

/// Serializes the tests in this module against each other; see [`EXACT_KEY`] for why a
/// module-local lock is correct here rather than a shared one.
static EXACT_KEY_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that holds [`EXACT_KEY_LOCK`] and, optionally, exports [`EXACT_KEY`] in this
/// process for its lifetime, restoring the previous state on drop (including on panic).
struct InheritedEnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
}

impl InheritedEnvGuard {
    fn set(value: Option<&str>) -> Self {
        let lock = EXACT_KEY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var(EXACT_KEY).ok();
        // SAFETY: `_lock` is held for this guard's whole lifetime, so no other test in this
        // module can read or write `EXACT_KEY` concurrently, and no other module uses it.
        unsafe {
            match value {
                Some(value) => std::env::set_var(EXACT_KEY, value),
                None => std::env::remove_var(EXACT_KEY),
            }
        }
        Self { _lock: lock, previous }
    }
}

impl Drop for InheritedEnvGuard {
    fn drop(&mut self) {
        // SAFETY: see `set` -- `_lock` is still held during `Drop`.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(EXACT_KEY, value),
                None => std::env::remove_var(EXACT_KEY),
            }
        }
    }
}

fn base_config() -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
    )
    .expect("fixture config should parse");
    cfg.resolve().expect("fixture config should resolve").remove(0)
}

/// Run `test_apps_run` for a single `python` target whose run command writes the value the
/// child process sees for [`EXACT_KEY`] into `sink`, and return those exact bytes.
///
/// The value is planted through `[crates.e2e.env]`, which is the same map `test_app_env_vars`
/// reads, so this exercises the real injection path rather than a re-derivation of it.
fn observed_env_value(configured: &str, inherited: Option<&str>) -> String {
    let _guard = InheritedEnvGuard::set(inherited);
    let temp = tempfile::tempdir().expect("tempdir");
    let sink = temp.path().join("observed");

    let mut config = base_config();
    let e2e = config.e2e.as_mut().expect("fixture config has an e2e section");
    e2e.env.insert(EXACT_KEY.to_owned(), configured.to_owned());
    e2e.registry.run.insert(
        "python".to_owned(),
        crate::core::config::output::TestAppRunConfig {
            precondition: Some("true".to_owned()),
            before: None,
            // `{EXACT_KEY}` is a Rust inline format capture, not shell syntax: the rendered
            // command is `printf '%s' "$ALEF_TEST_APPS_ENV_EXACTNESS" > '<sink>'`. ~keep
            run: Some(crate::core::config::output::StringOrVec::Single(format!(
                "printf '%s' \"${EXACT_KEY}\" > '{}'",
                sink.display()
            ))),
            argv_run: None,
        },
    );

    let names = ["python".to_owned()];
    test_apps_run(&config, Path::new("env_exactness_nonexistent_alef.toml"), &names)
        .expect("the probe run command should succeed");

    std::fs::read_to_string(&sink).expect("the probe should have written the observed value")
}

/// NEGATIVE CONTROL. With a wrong value already exported for the same key in this process,
/// the child must still see exactly what `[crates.e2e.env]` configured -- not the
/// `configured:inherited` concatenation the PATH-prepend guard produces. This is the single
/// condition that distinguishes `exact_env` from `path_env`; every other test in this file
/// would pass under the old, corrupting path.
#[test]
fn inherited_value_must_not_be_appended() {
    let configured = "http://127.0.0.1:53211/";
    let inherited = "http://attacker.example/inherited";
    let observed = observed_env_value(configured, Some(inherited));

    assert_eq!(
        observed, configured,
        "the configured value must arrive byte-exact; an inherited value must not be appended"
    );
    assert!(
        !observed.contains(inherited),
        "the inherited value leaked into the child's environment: {observed:?}"
    );
    assert_ne!(
        observed,
        format!("{configured}:{inherited}"),
        "this is the exact corruption the PATH-prepend guard produced"
    );
}

/// The baseline the negative control is measured against: with nothing inherited, the value
/// still arrives byte-exact. Asserted separately so a change that broke the ordinary path
/// cannot hide behind the inherited-env case.
#[test]
fn configured_value_arrives_exactly_when_nothing_is_inherited() {
    let configured = "http://127.0.0.1:53211/";
    assert_eq!(observed_env_value(configured, None), configured);
}

/// A wrong value is rejected, proving the assertion above is an equality check against the
/// configured value and not an "is anything set at all" check.
#[test]
fn a_deliberately_wrong_expected_value_does_not_match() {
    let configured = "http://127.0.0.1:53211/";
    let observed = observed_env_value(configured, Some("http://attacker.example/inherited"));
    assert_ne!(
        observed, "http://127.0.0.1:53212/",
        "a different port must not compare equal"
    );
    assert_ne!(observed, "", "an empty value must not compare equal");
    assert_ne!(
        observed, "http://127.0.0.1:53211",
        "a value missing the trailing slash must not compare equal"
    );
}

/// Exact means exact for hostile content too: an apostrophe (which is what the old
/// `export K='...'` rendering had to escape by hand), `$`, backticks, double quotes, and a
/// newline must all reach the child as literal bytes -- neither expanded nor truncated at the
/// newline. `Command::envs` has no quoting to get wrong, which is the point.
#[test]
fn hostile_characters_in_an_env_value_arrive_verbatim() {
    let configured = "v'1 $(id) `id` \"dq\" a;b|c&&d\nsecond-line";
    let observed = observed_env_value(configured, None);
    assert_eq!(
        observed, configured,
        "the value must arrive byte-exact, newline included"
    );
    assert_eq!(
        observed.lines().count(),
        2,
        "the value must not be truncated at its newline: {observed:?}"
    );
}
