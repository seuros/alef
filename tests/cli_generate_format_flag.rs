/// Integration tests guarding that the `--format` flag stays removed.
///
/// Formatting always runs: `alef generate` / `alef all` delegate to `poly fmt`
/// whenever poly is on PATH and skip it otherwise. The old opt-in `--format`
/// flag was removed, so it must no longer appear in `--help` and must be
/// rejected by clap as an unknown argument. `--no-format` was never introduced.
use std::process::Command;

fn alef_binary() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_alef") {
        return std::path::PathBuf::from(path);
    }
    let mut dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent")
        .to_path_buf();
    if dir.ends_with("deps") {
        dir = dir.parent().expect("parent of deps").to_path_buf();
    }
    dir.join("alef")
}

/// `alef generate --help` must NOT list `--format` or `--no-format`.
#[test]
fn generate_help_omits_format_flag() {
    let output = Command::new(alef_binary())
        .args(["generate", "--help"])
        .output()
        .expect("failed to run alef generate --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("--format"),
        "`alef generate --help` must not mention --format (it was removed); got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef generate --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef all --help` must NOT list `--format` or `--no-format`.
#[test]
fn all_help_omits_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--help"])
        .output()
        .expect("failed to run alef all --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("--format"),
        "`alef all --help` must not mention --format (it was removed); got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef all --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef generate --format` must be rejected by clap as an unknown argument.
#[test]
fn generate_rejects_removed_format_flag() {
    let output = Command::new(alef_binary())
        .args(["generate", "--format"])
        .output()
        .expect("failed to spawn alef");

    assert_eq!(
        output.status.code(),
        Some(2),
        "alef generate --format must be rejected as an unknown argument (exit code 2)"
    );
}

/// `alef all --format` must be rejected by clap as an unknown argument.
#[test]
fn all_rejects_removed_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--format"])
        .output()
        .expect("failed to spawn alef");

    assert_eq!(
        output.status.code(),
        Some(2),
        "alef all --format must be rejected as an unknown argument (exit code 2)"
    );
}
