//! CLI binary release matrix — validates the platform list that drives the alef CLI's own
//! GitHub release build.
//!
//! `.github/workflows/publish.yaml`'s "Resolve CLI target matrix" step reads
//! `.github/cli-targets.json` and is the single producer for two consumers: the `build-cli`
//! job's `strategy.matrix.include` (which platforms actually get built) and
//! `check-github-release`'s expected asset list (which archive filenames must exist on the
//! release). Both trust that producer's output to be non-empty. GitHub Actions does not
//! fail a job whose computed matrix has zero entries -- it silently runs zero legs and
//! reports the job as a vacuous success, so an empty `.github/cli-targets.json` (a bad
//! merge, a botched regen, a filter that drops every row) would ship a "successful"
//! release with no CLI binaries and no failure signal. That is the same shape as the
//! html-to-markdown incident where SKIPPED e2e steps read as PASSED and a bug got filed on
//! a false premise: a step that completes without doing anything is a defect, not a
//! no-op. ~keep
//!
//! This module is the typed, tested mirror of the validation that step's inline script
//! enforces: parsing `.github/cli-targets.json` (or an equivalent target list) must fail
//! loudly on zero targets rather than silently producing an empty matrix.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One row of the CLI release build matrix, matching `.github/cli-targets.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CliReleaseTarget {
    pub label: String,
    pub runner: String,
    pub target: String,
    pub archive_ext: String,
}

/// Parse and validate a CLI release target list (the contents of `.github/cli-targets.json`).
///
/// Returns an error -- never an empty `Vec` -- when the decoded list has zero entries. An
/// empty list must never reach `strategy.matrix.include`: GitHub Actions treats a zero-leg
/// matrix job as a silent success rather than a failure, so the caller must refuse to
/// produce one instead of passing it through.
pub fn parse_cli_release_targets(json: &str) -> Result<Vec<CliReleaseTarget>> {
    let targets: Vec<CliReleaseTarget> = serde_json::from_str(json).context("parsing CLI release target list")?;
    require_non_empty(&targets)?;
    Ok(targets)
}

/// Compute the `matrix=` and `assets=` `GITHUB_OUTPUT` payloads from a validated target list.
///
/// Errors rather than emitting an empty matrix/asset pair when `targets` is empty, so a
/// caller that skips [`parse_cli_release_targets`] cannot smuggle a vacuous matrix through
/// this function either.
pub fn cli_release_matrix_outputs(targets: &[CliReleaseTarget]) -> Result<(String, String)> {
    require_non_empty(targets)?;
    let matrix = serde_json::to_string(targets).context("serialising CLI release matrix")?;
    let assets = targets
        .iter()
        .map(|t| format!("alef-{}.{}", t.target, t.archive_ext))
        .collect::<Vec<_>>()
        .join(",");
    Ok((matrix, assets))
}

fn require_non_empty(targets: &[CliReleaseTarget]) -> Result<()> {
    if targets.is_empty() {
        anyhow::bail!(
            "CLI release target list is empty -- expected at least one platform \
             (e.g. x86_64-unknown-linux-gnu); refusing to build a vacuous release matrix \
             that GitHub Actions would report as a silent success with no CLI binaries built"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ParseCase {
        name: &'static str,
        json: &'static str,
        expect_err: bool,
        expect_len: usize,
    }

    #[test]
    fn parse_cli_release_targets_table() {
        let cases = [
            ParseCase {
                name: "empty array errors",
                json: "[]",
                expect_err: true,
                expect_len: 0,
            },
            ParseCase {
                name: "whitespace-padded empty array errors",
                json: "  []  ",
                expect_err: true,
                expect_len: 0,
            },
            ParseCase {
                name: "single target parses",
                json: r#"[{"label":"linux-x86_64","runner":"ubuntu-latest",
                    "target":"x86_64-unknown-linux-gnu","archive_ext":"tar.gz"}]"#,
                expect_err: false,
                expect_len: 1,
            },
            ParseCase {
                name: "multiple targets parse",
                json: r#"[
                    {"label":"linux-x86_64","runner":"ubuntu-latest",
                     "target":"x86_64-unknown-linux-gnu","archive_ext":"tar.gz"},
                    {"label":"windows-x86_64","runner":"windows-latest",
                     "target":"x86_64-pc-windows-msvc","archive_ext":"zip"}
                ]"#,
                expect_err: false,
                expect_len: 2,
            },
        ];

        for case in cases {
            let result = parse_cli_release_targets(case.json);
            assert_eq!(
                result.is_err(),
                case.expect_err,
                "case {:?}: expected error={}, got {:?}",
                case.name,
                case.expect_err,
                result
            );
            if let Ok(targets) = result {
                assert_eq!(
                    targets.len(),
                    case.expect_len,
                    "case {:?}: wrong target count",
                    case.name
                );
            }
        }
    }

    #[test]
    fn parse_cli_release_targets_rejects_malformed_json() {
        let result = parse_cli_release_targets("not json");
        assert!(
            result.is_err(),
            "malformed JSON must error, not silently yield an empty matrix"
        );
    }

    #[test]
    fn cli_release_matrix_outputs_table() {
        struct Case {
            name: &'static str,
            targets: Vec<CliReleaseTarget>,
            expect_err: bool,
            expect_assets: &'static str,
        }

        let cases = vec![
            Case {
                name: "empty slice errors",
                targets: vec![],
                expect_err: true,
                expect_assets: "",
            },
            Case {
                name: "single target computes matrix and assets",
                targets: vec![CliReleaseTarget {
                    label: "linux-x86_64".to_string(),
                    runner: "ubuntu-latest".to_string(),
                    target: "x86_64-unknown-linux-gnu".to_string(),
                    archive_ext: "tar.gz".to_string(),
                }],
                expect_err: false,
                expect_assets: "alef-x86_64-unknown-linux-gnu.tar.gz",
            },
            Case {
                name: "multiple targets join assets with commas",
                targets: vec![
                    CliReleaseTarget {
                        label: "linux-x86_64".to_string(),
                        runner: "ubuntu-latest".to_string(),
                        target: "x86_64-unknown-linux-gnu".to_string(),
                        archive_ext: "tar.gz".to_string(),
                    },
                    CliReleaseTarget {
                        label: "windows-x86_64".to_string(),
                        runner: "windows-latest".to_string(),
                        target: "x86_64-pc-windows-msvc".to_string(),
                        archive_ext: "zip".to_string(),
                    },
                ],
                expect_err: false,
                expect_assets: "alef-x86_64-unknown-linux-gnu.tar.gz,alef-x86_64-pc-windows-msvc.zip",
            },
        ];

        for case in cases {
            let result = cli_release_matrix_outputs(&case.targets);
            assert_eq!(
                result.is_err(),
                case.expect_err,
                "case {:?}: expected error={}, got {:?}",
                case.name,
                case.expect_err,
                result
            );
            if let Ok((matrix, assets)) = result {
                assert_eq!(assets, case.expect_assets, "case {:?}: wrong assets string", case.name);
                assert!(!matrix.is_empty(), "case {:?}: matrix must not be empty", case.name);
            }
        }
    }
}
