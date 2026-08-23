/// Regression for alef #119: `alef all` used to gate its whole-tree formatting pass on
/// `changed_languages`, a set populated only when the bindings, service-API, or stub write
/// phase reported a byte-level change. Every other write phase -- scaffold, public API, e2e,
/// README, docs -- could write new content on disk without ever being consulted by that gate.
/// A steady-state run where only README output changed (e.g. because bindings were entirely
/// skipped by the per-language lang-hash cache while README content still differed) left
/// `changed_languages` empty, so the whole-tree `poly fmt` convergence pass never ran, and any
/// stray unformatted file elsewhere in the tree stayed unformatted forever -- with no second
/// `alef all` run ever fixing it, since the same narrow gate applied every time.
///
/// This is reproduced here with a single `python` target (no FFI build, no post-build step,
/// so the run is fast and needs no toolchain beyond alef's own generation pipeline):
/// 1. `alef all` once, from scratch -- a full write that also runs formatting.
/// 2. Delete the generated `README.md` and drop a deliberately unformatted, alef-unmanaged
///    `messy.py` file inside the python package directory.
/// 3. `alef all` again -- bindings/stubs/service-API are byte-identical to what's on disk (the
///    per-language cache hits), so only the README write phase reports a change. Under the old
///    gate this second run's log has no "Formatting generated files..." line at all, and
///    `messy.py` is left exactly as written (`x=1`).
/// 4. A third `alef all` run must make no further changes -- the tree is formatted and hash-
///    stable, not oscillating.
///
/// Step 3's actual reformatting of `messy.py` depends on `poly` being on `PATH` -- it is the
/// engine `pipeline::format_generated_reporting` shells out to. CI's `test` job deliberately
/// does not install `poly` (see `.github/workflows/ci.yml`'s comment on that job), so this test
/// cannot assert the `x=1` -> `x = 1` transformation unconditionally without being vacuous
/// there: with `poly` absent, the whole-tree pass still runs but every step it would perform
/// is recorded as skipped, `messy.py` stays exactly as seeded, and a test that only checked the
/// file's contents could not tell "the gate is still narrowly keyed" (the bug this regression
/// guards) apart from "there is no formatter to prove the gate ran" -- a vacuous assertion.
/// So this test brings its own two-path proof instead of silently tolerating that: when
/// `poly` is on `PATH`, it asserts the real transformation directly;
/// when it is not, it asserts the whole-tree pass's own log evidence instead -- the
/// "Formatting generated files..." line proving the gate fired, paired with the deferred-step
/// warning naming `poly` as the missing tool -- which a narrowly-keyed gate could never
/// produce on a README-only change regardless of whether `poly` is installed. ~keep
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Whether `poly` is on `PATH` in this process's environment, mirroring the same tolerance
/// `e2e::format::format_language` already applies to poly-shelling tests (see that module's
/// non-`--strict` deferral) -- this test is not exempt from CI's `test` job omitting `poly`
/// just because it happens to also touch `alef all`'s formatting gate rather than the e2e
/// pipeline's. ~keep
fn poly_is_available() -> bool {
    which::which("poly").is_ok()
}

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create fixture source directory");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"format-gate-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write fixture Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Record { pub value: String }\n\npub fn record_value(record: Record) -> String { record.value }\n",
    )
    .expect("write fixture source");
    fs::write(
        root.join("alef.toml"),
        format!(
            "[workspace]\nalef_version = \"{}\"\nlanguages = [\"python\"]\n\n\
             [[crates]]\nname = \"format-gate-fixture\"\nsources = [\"src/lib.rs\"]\n\
             version_from = \"Cargo.toml\"\n\n[crates.generate]\npublic_api = false\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("write alef config");
}

fn run_all(root: &Path) -> Output {
    Command::new(alef_binary())
        .current_dir(root)
        .arg("all")
        .output()
        .expect("run alef all")
}

#[test]
fn a_readme_only_change_still_formats_stray_files_and_stays_hash_stable() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    let root = fixture.path();
    write_fixture(root);

    let first = run_all(root);
    assert!(
        first.status.success(),
        "first `alef all` run must succeed: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let readme = root.join("packages/python/README.md");
    assert!(readme.is_file(), "first run must generate a README");
    fs::remove_file(&readme).expect("delete README to force a README-only change on the next run");

    let messy = root.join("packages/python/messy.py");
    fs::write(&messy, "x=1\n").expect("seed an unformatted, alef-unmanaged file");

    let second = run_all(root);
    assert!(
        second.status.success(),
        "second `alef all` run must succeed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout_and_stderr = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        readme.is_file(),
        "the deleted README must be regenerated on the second run"
    );

    // Whichever branch runs, the whole-tree pass must have fired at all: a narrowly-keyed
    // gate (the alef #119 bug) never logs this line on a README-only change, with or
    // without `poly` on `PATH`. This is the one assertion both branches below share. ~keep
    assert!(
        second_stdout_and_stderr.contains("Formatting generated files"),
        "a run whose only reported change was the README must still run the whole-tree \
         formatting pass -- if the \"Formatting generated files...\" line is missing, the \
         format gate is still narrowly keyed to bindings/service-API/stubs only. Full run \
         output:\n{second_stdout_and_stderr}"
    );

    if poly_is_available() {
        assert_eq!(
            fs::read_to_string(&messy).expect("read messy.py after the second run"),
            "x = 1\n",
            "a run whose only reported change was the README must still reformat every other \
             file under the tree, including this alef-unmanaged stray file -- if this still \
             reads `x=1`, the whole-tree pass ran but did not actually reach this file. Full \
             run output:\n{second_stdout_and_stderr}"
        );
    } else {
        // `poly` is not installed (this is CI's `test` job, which omits it deliberately --
        // see the module doc). The whole-tree pass still ran (asserted above) but every step
        // it would perform gets recorded as deferred and `messy.py` is left exactly as
        // seeded, so the only remaining proof available here is the deferred-step warning
        // itself naming `poly` as the missing tool. Asserting on that instead of skipping the
        // test keeps this regression covered even where `poly` is absent, rather than going
        // silent exactly where the vacuous-assertion failure mode this test guards against
        // would otherwise hide. ~keep
        assert!(
            second_stdout_and_stderr.contains("poly") && second_stdout_and_stderr.contains("not installed"),
            "with `poly` absent, the whole-tree pass must still record a deferred step naming \
             `poly` as the missing tool -- if that warning is missing, the pass did not \
             actually attempt to format the tree. Full run output:\n{second_stdout_and_stderr}"
        );
        assert_eq!(
            fs::read_to_string(&messy).expect("read messy.py after the second run"),
            "x=1\n",
            "with `poly` absent no formatter can have touched this file; if it changed, this \
             assertion (not the reformatting one above) needs updating instead. Full run \
             output:\n{second_stdout_and_stderr}"
        );
    }

    let messy_after_second = fs::read_to_string(&messy).expect("read messy.py again");
    let third = run_all(root);
    assert!(
        third.status.success(),
        "third `alef all` run must succeed: {}",
        String::from_utf8_lossy(&third.stderr)
    );
    assert_eq!(
        fs::read_to_string(&messy).expect("read messy.py after the third run"),
        messy_after_second,
        "a converged tree must be hash-stable: a third run must not change what the second \
         run's formatting pass already settled"
    );
}
