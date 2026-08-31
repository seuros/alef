//! Hostile-payload proofs for the values `default_test_apps_run_config` embeds into a
//! test-app run command.
//!
//! Two values reaching these commands are free-form, user-authored config with no syntax
//! restrictions: `[crates.e2e.registry].output` (the `test_apps_dir` argument) and
//! `[crates.e2e.registry.packages.<lang>].version` (the `published_version` argument).
//! Both used to be spliced unquoted into a shell string, so a `;`, backtick, `$(...)`,
//! `|`, `&&`, or newline inside either one executed arbitrary commands during `alef
//! test-apps run`.
//!
//! Every test here executes a real `/bin/sh` and proves the payload became an inert
//! literal -- absence of a marker file, not a string comparison, is the assertion.
//! [`unquoted_registry_output_would_have_executed_the_payload`] is the negative control:
//! it runs the *pre-fix* rendering through the same harness and requires every marker to
//! appear, so a harness that quietly stopped being able to observe an injection fails
//! loudly instead of reporting a vacuous pass. ~keep

use super::super::tools::ToolsConfig;
use super::*;
use std::path::Path;

/// A directory name carrying every shell metacharacter class at once. Chosen so that the
/// *unquoted* rendering is still syntactically valid shell -- balanced quotes, no `/` --
/// because the negative control below depends on the pre-fix string actually running. Each
/// clause creates one distinctly named marker so a partial escape is visible rather than
/// masked by a single boolean. The final `#` line comments out whatever the surrounding
/// `format!` appended (`/rust && cargo test`, `/python && uv sync && ...`), which is what
/// keeps this test from ever invoking a real toolchain. ~keep
const HOSTILE_DIR: &str = concat!(
    "ta;touch SEMI;$(touch DOLLAR);`touch BACKTICK`;",
    "touch PIPE|cat;true&&touch AND;echo 'sq' \"dq\"\n",
    "touch NEWLINE\n#"
);

/// Marker filenames the [`HOSTILE_DIR`] payload creates if any part of it is parsed as shell
/// syntax rather than carried as one literal word.
const MARKERS: [&str; 6] = ["SEMI", "DOLLAR", "BACKTICK", "PIPE", "AND", "NEWLINE"];

/// A version string with the same metacharacter coverage, for the PHP `published_version`
/// path. No trailing `#` is needed: the PHP default is argv-only, so there is no shell
/// string for a comment to terminate.
const HOSTILE_VERSION: &str = "1.0.0;touch SEMI;$(touch DOLLAR);`touch BACKTICK`|touch PIPE&&touch AND\ntouch NEWLINE";

/// Symlink one real binary into `root/bin` under `name`, idempotently.
fn link_tool(root: &Path, name: &str, candidates: &[&str]) -> std::path::PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create sandbox bin dir");
    let link = bin.join(name);
    if !link.exists() {
        let target = candidates
            .iter()
            .map(Path::new)
            .find(|candidate| candidate.exists())
            .unwrap_or_else(|| panic!("no {name} binary found among {candidates:?}"));
        std::os::unix::fs::symlink(target, &link).expect("symlink into the sandbox bin dir");
    }
    bin
}

/// Build a `PATH` that resolves `touch` and nothing else.
///
/// `touch` must resolve, or an injection would fail to create its marker and this whole
/// file would pass while proving nothing. Everything else must NOT resolve: a unit test has
/// no business running `cargo test`, `pnpm install`, `zig build`, or `composer` on the
/// machine it happens to be executing on. Shell builtins (`cd`, `echo`, `true`, `:`) are
/// unaffected by `PATH` and still work, which is all the payload needs. ~keep
fn sandbox_path(root: &Path) -> String {
    link_tool(root, "touch", &["/usr/bin/touch", "/bin/touch"])
        .display()
        .to_string()
}

/// Run `command` through a real `/bin/sh` rooted at `root`, with the sandboxed `PATH` and an
/// otherwise empty environment. The exit status is deliberately ignored: every command under
/// test is *expected* to fail (its `cd` target does not exist, its toolchain is not on
/// `PATH`). What is asserted is which files exist afterwards.
fn run_in_sandbox(root: &Path, command: &str) {
    let path = sandbox_path(root);
    std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .current_dir(root)
        .env_clear()
        .env("PATH", path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("/bin/sh should start");
}

/// Marker files that exist under `root` after a run, in [`MARKERS`] order.
fn markers_created(root: &Path) -> Vec<&'static str> {
    MARKERS
        .into_iter()
        .filter(|marker| root.join(marker).exists())
        .collect()
}

fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Every `Language` whose default still renders a shell string, paired with that string.
/// Derived from `Language::ALL` rather than a hand-kept list, so a language added later is
/// covered the day it lands instead of the day someone remembers this file.
fn shell_run_commands(test_apps_dir: &str) -> Vec<(Language, String)> {
    let tools = ToolsConfig::default();
    let ctx = LangContext::default(&tools);
    Language::ALL
        .into_iter()
        .filter_map(|lang| {
            let cfg = default_test_apps_run_config(lang, test_apps_dir, &ctx, None, None);
            cfg.run.map(|run| (lang, run.commands().join("\n")))
        })
        .collect()
}

/// GREEN: no shell-string default lets a hostile `[crates.e2e.registry].output` execute.
#[test]
fn hostile_registry_output_is_inert_in_every_shell_default() {
    let commands = shell_run_commands(HOSTILE_DIR);
    assert!(
        commands.len() >= 10,
        "expected the shell-string defaults to still be numerous; got {} -- if this dropped \
         because languages moved to argv that is good, but re-check the floor rather than \
         letting this test silently examine almost nothing",
        commands.len()
    );
    for (lang, command) in commands {
        let root = temp_root();
        run_in_sandbox(root.path(), &command);
        assert_eq!(
            markers_created(root.path()),
            Vec::<&str>::new(),
            "{lang}: the hostile registry output executed; command was:\n{command}"
        );
    }
}

/// RED harness proof: the *pre-fix* rendering -- the same payload spliced in unquoted -- must
/// create every marker. Without this, `hostile_registry_output_is_inert_in_every_shell_default`
/// could pass because the sandbox cannot run `touch` at all.
#[test]
fn unquoted_registry_output_would_have_executed_the_payload() {
    let root = temp_root();
    let pre_fix = format!("cd {HOSTILE_DIR}/rust && cargo test");
    run_in_sandbox(root.path(), &pre_fix);
    assert_eq!(
        markers_created(root.path()),
        MARKERS.to_vec(),
        "the unquoted rendering must execute the payload, otherwise this file's other \
         assertions prove nothing; command was:\n{pre_fix}"
    );
}

/// The quoted directory is not merely inert, it is byte-exact: a `[crates.e2e.registry].output`
/// containing metacharacters must still name the same directory the Rust side resolves with
/// `base_dir.join(&e2e.registry.output)`. Inertness alone would also be satisfied by mangling
/// the value, which would be a different bug.
#[test]
fn quoted_registry_output_round_trips_byte_exactly() {
    let root = temp_root();
    let path = sandbox_path(root.path());
    let (lang, command) = shell_run_commands(HOSTILE_DIR)
        .into_iter()
        .find(|(lang, _)| *lang == Language::Rust)
        .expect("rust has a shell-string default");
    let quoted = command
        .strip_prefix("cd ")
        .and_then(|rest| rest.strip_suffix(" && cargo test"))
        .unwrap_or_else(|| panic!("{lang} default shape changed: {command}"))
        .to_owned();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("printf '%s' {quoted}")])
        .current_dir(root.path())
        .env_clear()
        .env("PATH", path)
        .output()
        .expect("/bin/sh should start");
    assert!(output.status.success(), "printf should succeed for: {quoted}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).as_ref(),
        format!("{HOSTILE_DIR}/rust").as_str(),
        "the quoted directory must expand back to the exact configured value"
    );
}

/// PHP's default is argv-only, so the hostile version is one opaque element handed to
/// `install.sh`. Argv entries are COUNTED and compared element-by-element -- never rejoined
/// into a string, which is exactly what would hide a word-splitting defect.
#[test]
fn hostile_php_version_stays_one_argv_element() {
    let tools = ToolsConfig::default();
    let ctx = LangContext::default(&tools);
    let cfg = default_test_apps_run_config(Language::Php, HOSTILE_DIR, &ctx, Some(HOSTILE_VERSION), None);
    assert!(
        cfg.run.is_none(),
        "php must not fall back to a shell string that could reinterpret this payload: {:?}",
        cfg.run
    );
    let argv = cfg.argv_run.expect("php should have an argv run command");
    assert_eq!(argv.work_dir, format!("{HOSTILE_DIR}/php"));
    assert_eq!(argv.steps.len(), 3, "install.sh, composer install, composer test");
    assert_eq!(argv.steps[0].command, "bash");
    assert_eq!(
        argv.steps[0].args.len(),
        2,
        "the version must be exactly one additional argv element, not word-split into several: {:?}",
        argv.steps[0].args
    );
    assert_eq!(argv.steps[0].args[0], "install.sh");
    assert_eq!(
        argv.steps[0].args[1], HOSTILE_VERSION,
        "the whole payload, newline included, must arrive as one literal argument"
    );
}

/// Spawning the PHP install step for real proves the version is inert, not just that it looks
/// like one `Vec` element: `install.sh` here is a stub that records `$#` and `$1`, and no
/// marker file may appear afterwards.
#[test]
fn hostile_php_version_does_not_execute_when_the_step_is_spawned() {
    let root = temp_root();
    let path = sandbox_path(root.path());
    link_tool(root.path(), "bash", &["/bin/bash", "/usr/bin/bash", "/bin/sh"]);
    let work_dir = root.path().join("app");
    std::fs::create_dir_all(&work_dir).expect("create work dir");
    std::fs::write(
        work_dir.join("install.sh"),
        "#!/bin/sh\nprintf '%s' \"$#\" > argc\nprintf '%s' \"${1-}\" > argv1\n",
    )
    .expect("write install.sh stub");

    let tools = ToolsConfig::default();
    let ctx = LangContext::default(&tools);
    let cfg = default_test_apps_run_config(Language::Php, "unused", &ctx, Some(HOSTILE_VERSION), None);
    let argv = cfg.argv_run.expect("php should have an argv run command");
    let step = &argv.steps[0];

    let status = std::process::Command::new(&step.command)
        .args(&step.args)
        .current_dir(&work_dir)
        .env_clear()
        .env("PATH", path)
        .status()
        .expect("bash should start");
    assert!(status.success(), "the install.sh stub should exit 0");

    assert_eq!(
        markers_created(root.path()),
        Vec::<&str>::new(),
        "spawning the PHP install step executed the hostile version payload"
    );
    assert_eq!(
        std::fs::read_to_string(work_dir.join("argc")).expect("argc"),
        "1",
        "install.sh must receive exactly one argument -- counted, never rejoined"
    );
    assert_eq!(
        std::fs::read_to_string(work_dir.join("argv1")).expect("argv1"),
        HOSTILE_VERSION,
        "the argument must arrive byte-exact, newline and all"
    );
}

/// The argv-only defaults take the hostile directory as an opaque `work_dir`. It must be
/// carried verbatim (it becomes `Command::current_dir`, which never parses it) and the step
/// lists must be exactly the expected length.
#[test]
fn argv_defaults_carry_the_hostile_directory_verbatim() {
    let tools = ToolsConfig::default();
    let ctx = LangContext::default(&tools);

    let go = default_test_apps_run_config(Language::Go, HOSTILE_DIR, &ctx, None, None)
        .argv_run
        .expect("go argv run");
    assert_eq!(go.work_dir, format!("{HOSTILE_DIR}/go"));
    assert_eq!(go.steps.len(), 2);

    let php = default_test_apps_run_config(Language::Php, HOSTILE_DIR, &ctx, None, None)
        .argv_run
        .expect("php argv run");
    assert_eq!(php.work_dir, format!("{HOSTILE_DIR}/php"));
    assert_eq!(php.steps.len(), 3);
    assert_eq!(
        php.steps[0].args.len(),
        1,
        "no published version means install.sh takes no extra argument: {:?}",
        php.steps[0].args
    );

    for name in ["brew", "homebrew"] {
        let cfg = default_test_apps_run_config_for_name(name, HOSTILE_DIR, &ctx);
        assert!(cfg.run.is_none(), "{name} must be argv-only: {:?}", cfg.run);
        let argv = cfg.argv_run.unwrap_or_else(|| panic!("{name} argv run"));
        assert_eq!(argv.work_dir, format!("{HOSTILE_DIR}/{name}"));
        assert_eq!(argv.steps.len(), 1);
        assert_eq!(argv.steps[0].command, "bash");
        assert_eq!(argv.steps[0].args.len(), 1);
        assert_eq!(argv.steps[0].args[0], "run_tests.sh");
    }
}
