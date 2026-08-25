//! `[crates.verify]` -- a narrow, path-named opt-out for `alef verify`'s "missing generated
//! file" checks, for output a consumer deliberately never commits.
//!
//! MEASURED (alef-tasks #318): a crate with an `[e2e]` block always runs `alef verify`'s
//! registry-mode test-app stage as part of its managed surface (`collect_managed_surface`'s
//! `e2e_registry_stage`, unconditional whenever `config.e2e` is `Some`), regardless of whether
//! the consumer publishes or commits that output. A consumer whose `.gitignore` excludes the
//! whole `test_apps/` tree -- because it is ephemeral, regenerated per CI run, and never meant
//! to be committed -- gets every one of those paths routed into `missing_gitignored`
//! (`bin_cli::verify_gitignore::split_missing_by_gitignore`), which is a HARD, PERMANENT
//! failure (`has_missing_gitignored_files` in `bin_cli::core_commands::verify::run`): `alef
//! generate` cannot fix it (the file is written, then discarded by the ignore rule before it
//! can be committed), and the consumer never intends to commit it in the first place. A minimal
//! reproduction (one fixture, one language) measured 3 of 3 registry-mode files landing in
//! `missing_gitignored`; a real consumer tree measured 316.
//!
//! [`VerifyConfig::ignore_ephemeral`] closes that gap the narrow way the alef-tasks brief
//! demands: it NAMES PATHS (repo-relative glob patterns), not "trust whatever `.gitignore`
//! already says" -- the latter would blind `missing_gitignored` to the exact class of bug it
//! exists to catch (a `.gitignore` rule that accidentally discards output the consumer DOES
//! mean to commit). And every path this run actually excludes is still counted and reported by
//! `bin_cli::core_commands::verify::run`, printed unconditionally alongside `alef verify`'s
//! other coverage facts -- a run that excluded 316 paths must say so, not quietly shrink its
//! own scope. See that function's `ignore_ephemeral`-handling for the report text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-crate `alef verify` opt-outs. Everything here narrows what one specific check may name
/// as a failure; nothing here can turn a genuine drift or ownership finding silent for a path
/// it does not match.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyConfig {
    /// Repo-relative glob patterns (matched with [`glob::Pattern`] against the path relative to
    /// the config file's directory) naming generated output that is intentionally ephemeral --
    /// regenerated per run, deliberately gitignored, and never meant to be committed.
    ///
    /// A path this run's managed surface expects but that is absent from disk AND matches one of
    /// these patterns is dropped entirely from `alef verify`'s "missing generated files" and
    /// "missing generated files that are also gitignored" reports, instead of failing the run
    /// forever for output the consumer never intended to track. It does NOT affect a path that
    /// already exists on disk (stale/frozen/orphan checks are unaffected -- those apply to real
    /// bytes on disk, which "ephemeral" says nothing about the correctness of).
    ///
    /// Example, matching a consumer's whole registry-mode test-app output:
    /// ```toml
    /// [crates.verify]
    /// ignore_ephemeral = ["test_apps/**"]
    /// ```
    #[serde(default)]
    pub ignore_ephemeral: Vec<String>,
}

impl VerifyConfig {
    /// Compiled [`glob::Pattern`]s for [`Self::ignore_ephemeral`], silently dropping a pattern
    /// that fails to compile as a glob -- `alef verify` is read-only and must never hard-fail a
    /// whole run over one malformed opt-out pattern; the remaining patterns still apply, and an
    /// entry that never matches anything is indistinguishable in effect from one that never
    /// compiled, so there is no silent-acceptance hazard either way.
    fn patterns(&self) -> Vec<glob::Pattern> {
        self.ignore_ephemeral
            .iter()
            .filter_map(|pattern| glob::Pattern::new(pattern).ok())
            .collect()
    }

    /// Split `paths` (absolute, as `alef verify`'s missing-file lists carry them) into ones this
    /// config's [`Self::ignore_ephemeral`] patterns do NOT match (kept, still reportable) and the
    /// count that matched (excluded). Matching is against the path relative to `base_dir`; a path
    /// this function cannot make relative to `base_dir` is never excluded -- conservatively kept,
    /// exactly like a pattern that failed to compile.
    pub(crate) fn partition_ephemeral(&self, paths: Vec<String>, base_dir: &std::path::Path) -> (Vec<String>, usize) {
        let patterns = self.patterns();
        if patterns.is_empty() {
            return (paths, 0);
        }
        let mut kept = Vec::with_capacity(paths.len());
        let mut excluded = 0usize;
        for path in paths {
            let relative = std::path::Path::new(&path)
                .strip_prefix(base_dir)
                .unwrap_or(std::path::Path::new(&path));
            if patterns.iter().any(|pattern| pattern.matches_path(relative)) {
                excluded += 1;
            } else {
                kept.push(path);
            }
        }
        (kept, excluded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_no_patterns_and_excludes_nothing() {
        let config = VerifyConfig::default();
        let (kept, excluded) = config.partition_ephemeral(
            vec!["/repo/test_apps/python/conftest.py".to_string()],
            std::path::Path::new("/repo"),
        );
        assert_eq!(kept, vec!["/repo/test_apps/python/conftest.py".to_string()]);
        assert_eq!(excluded, 0);
    }

    #[test]
    fn a_matching_glob_excludes_every_path_under_it_and_reports_the_count() {
        let config = VerifyConfig {
            ignore_ephemeral: vec!["test_apps/**".to_string()],
        };
        let paths = vec![
            "/repo/test_apps/python/conftest.py".to_string(),
            "/repo/test_apps/python/pyproject.toml".to_string(),
            "/repo/packages/python/lib.rs".to_string(),
        ];
        let (kept, excluded) = config.partition_ephemeral(paths, std::path::Path::new("/repo"));
        assert_eq!(kept, vec!["/repo/packages/python/lib.rs".to_string()]);
        assert_eq!(excluded, 2);
    }

    #[test]
    fn a_malformed_pattern_is_dropped_without_failing_the_others() {
        let config = VerifyConfig {
            ignore_ephemeral: vec!["[".to_string(), "test_apps/**".to_string()],
        };
        let (kept, excluded) = config.partition_ephemeral(
            vec!["/repo/test_apps/python/conftest.py".to_string()],
            std::path::Path::new("/repo"),
        );
        assert!(kept.is_empty());
        assert_eq!(excluded, 1);
    }

    #[test]
    fn does_not_match_a_sibling_directory_with_a_shared_prefix() {
        let config = VerifyConfig {
            ignore_ephemeral: vec!["test_apps/**".to_string()],
        };
        let (kept, excluded) = config.partition_ephemeral(
            vec!["/repo/test_apps_backup/README.md".to_string()],
            std::path::Path::new("/repo"),
        );
        assert_eq!(kept, vec!["/repo/test_apps_backup/README.md".to_string()]);
        assert_eq!(excluded, 0);
    }
}
