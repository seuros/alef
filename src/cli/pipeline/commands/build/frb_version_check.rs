//! Orchestration for [`crate::core::backend::PostBuildStep::VerifyFrbCodegenVersion`]: fails
//! loudly, before `flutter_rust_bridge_codegen generate` runs, when the installed codegen
//! binary's version disagrees with the alef-declared `[crates.dart] frb_version` pin.
//!
//! `flutter_rust_bridge_codegen`'s generated output (import ordering, wire dispatch structure,
//! generated comments) is a function of its own version, not just the Rust input it reads. With
//! no check in place, `alef generate`/`alef build` baked whatever version happened to be on the
//! invoking machine's `PATH` into committed Dart/Rust output -- two developers, or a developer
//! and CI, with different installs produced different bytes from identical input (alef #204).

use std::process::Command;

pub(super) const FLUTTER_RUST_BRIDGE_CODEGEN: &str = "flutter_rust_bridge_codegen";

/// Verify that invoking `cmd -- --version` reports `expected_version`.
///
/// `cmd` is a parameter (rather than hard-coding [`FLUTTER_RUST_BRIDGE_CODEGEN`] internally) so
/// tests can point this at a fake binary instead of mutating process-global `PATH` state; the
/// real post-build step always calls this with `FLUTTER_RUST_BRIDGE_CODEGEN`.
///
/// A missing binary is not this step's concern: it returns `Ok(())` and leaves the report to
/// the `RunCommand` step immediately after, which already treats a missing tool as a non-fatal
/// skip (falling back to committed output). Likewise, a binary present but failing to even run
/// `--version` is left for that same `RunCommand` step to fail on its own real invocation --
/// duplicating that diagnosis here would just repeat it under a less specific error.
///
/// Fails loudly only when the binary ran and reported a version that disagrees with
/// `expected_version` -- the one case this step exists to catch.
pub(super) fn run(cmd: &str, expected_version: &str) -> anyhow::Result<()> {
    let output = match Command::new(cmd).arg("--version").output() {
        Ok(output) if output.status.success() => output,
        Ok(_) | Err(_) => return Ok(()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(installed_version) = parse_version(&stdout) else {
        // Output alef doesn't recognize the shape of -- not this step's job to diagnose that;
        // let the RunCommand step's real `generate` invocation surface whatever is actually
        // wrong with the toolchain.
        return Ok(());
    };

    if installed_version != expected_version {
        anyhow::bail!(
            "installed {cmd} reports version {installed_version}, but this project pins \
             flutter_rust_bridge {expected_version} (see `[crates.dart] frb_version` in \
             alef.toml, or alef's compiled-in default). Generated Dart/Rust bridge output is not \
             deterministic across flutter_rust_bridge_codegen versions -- install the pinned \
             version (`cargo install flutter_rust_bridge_codegen --version {expected_version} \
             --locked`) before running `alef generate`/`alef build`, or update the pin to match \
             what is installed."
        );
    }

    Ok(())
}

/// Extract the version number from `flutter_rust_bridge_codegen --version` output, which is
/// clap's default `<binary-name> <version>` format (e.g. `"flutter_rust_bridge_codegen 2.13.0\n"`).
fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .next()?
        .split_whitespace()
        .next_back()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn parse_version_extracts_trailing_token_from_clap_style_output() {
        assert_eq!(
            parse_version("flutter_rust_bridge_codegen 2.13.0\n"),
            Some("2.13.0".to_string())
        );
    }

    #[test]
    fn parse_version_handles_missing_newline() {
        assert_eq!(
            parse_version("flutter_rust_bridge_codegen 2.13.0"),
            Some("2.13.0".to_string())
        );
    }

    #[test]
    fn parse_version_returns_none_for_empty_output() {
        assert_eq!(parse_version(""), None);
    }

    /// Writes an executable shell script at `dir/name` that prints `flutter_rust_bridge_codegen
    /// <version>` and exits 0 when invoked with `--version`, and returns its absolute path.
    /// `Command::new` accepts a full path directly, so tests never need to touch the real
    /// `PATH` environment variable to stand this fake binary up.
    fn fake_codegen_binary(dir: &std::path::Path, version: &str) -> std::path::PathBuf {
        let path = dir.join("fake_flutter_rust_bridge_codegen.sh");
        let mut file = std::fs::File::create(&path).expect("must create fake binary script");
        writeln!(file, "#!/bin/sh").expect("must write shebang");
        writeln!(file, "echo 'flutter_rust_bridge_codegen {version}'").expect("must write echo line");
        drop(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("must chmod +x");
        path
    }

    #[test]
    fn run_is_ok_when_installed_version_matches_the_pin() {
        let dir = tempfile::tempdir().expect("temp dir");
        let binary = fake_codegen_binary(dir.path(), "2.13.0");

        let result = run(binary.to_str().expect("utf8 path"), "2.13.0");

        assert!(result.is_ok(), "matching versions must not fail: {result:?}");
    }

    /// The regression this whole file exists to catch: a locally installed codegen binary at a
    /// version that disagrees with the project's declared `frb_version` pin must fail loudly,
    /// not silently regenerate output stamped with whatever happens to be on this machine.
    #[test]
    fn run_fails_loudly_when_installed_version_disagrees_with_the_pin() {
        let dir = tempfile::tempdir().expect("temp dir");
        let binary = fake_codegen_binary(dir.path(), "2.9.0");

        let result = run(binary.to_str().expect("utf8 path"), "2.13.0");

        let err = result.expect_err("mismatched versions must fail");
        let message = format!("{err:#}");
        assert!(
            message.contains("2.9.0") && message.contains("2.13.0"),
            "error must name both the installed and pinned versions: {message}"
        );
    }

    /// A missing binary must not be reported as a mismatch -- see `run`'s doc comment: that
    /// diagnosis belongs to the `RunCommand` step right after, which already handles it.
    #[test]
    fn run_is_ok_when_binary_is_not_on_path() {
        let result = run("alef-frb-version-check-intentionally-not-on-path-xyz789", "2.13.0");

        assert!(
            result.is_ok(),
            "a missing binary must not be treated as a mismatch: {result:?}"
        );
    }
}
