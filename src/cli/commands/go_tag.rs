//! Go submodule tagging helper.
//!
//! Creates the two Go module tags required per release:
//! - `packages/go/v{major}/{tag}` — correct per Go module spec
//! - `packages/go/{tag}` — legacy format for backwards compatibility
//!
//! Both tags are pushed to the remote with `--force-with-lease` (or printed in
//! dry-run mode).
//!
//! Ports: `sample_core/scripts/publish/go/tag-and-push-go-module.sh`

use crate::core::config::ResolvedCrateConfig;
use anyhow::{Context, Result};
use serde_json::json;

/// Parameters for the go-tag command.
pub struct GoTagParams<'a> {
    pub version: &'a str,
    pub remote: &'a str,
    pub dry_run: bool,
    pub output_json: bool,
    pub config: &'a ResolvedCrateConfig,
    /// Working directory (repository root).
    pub workspace_root: &'a std::path::Path,
}

/// Create and push Go module tags for a release.
pub fn run(params: &GoTagParams<'_>) -> Result<Vec<String>> {
    let version = params.version.trim_start_matches('v');
    let tag = format!("v{version}");

    let major: u64 = version
        .split('.')
        .next()
        .context("cannot parse major version")?
        .parse()
        .context("major version is not a number")?;

    let go_output = params.config.package_dir(crate::core::config::extras::Language::Go);
    let go_base = go_output.trim_end_matches('/').to_string();

    let go_module_path = if major >= 2 {
        format!("{go_base}/v{major}")
    } else {
        go_base.clone()
    };

    let module_tag = format!("{go_module_path}/{tag}");
    let legacy_tag = format!("{go_base}/{tag}");

    let tags = if major >= 2 {
        vec![module_tag.clone(), legacy_tag.clone()]
    } else {
        vec![module_tag.clone()]
    };

    let mut created = Vec::new();

    for ref_tag in &tags {
        if params.dry_run {
            tracing::info!("[dry-run] Would create git tag: {ref_tag}");
            tracing::info!("[dry-run] Would push to {}: {ref_tag}", params.remote);
            created.push(ref_tag.clone());
        } else {
            create_and_push_tag(ref_tag, &tag, params.remote, params.workspace_root)?;
            created.push(ref_tag.clone());
        }
    }

    if params.output_json {
        let out = json!({
            "version": tag,
            "major": major,
            "tags_created": created,
            "remote": params.remote,
            "dry_run": params.dry_run,
        });
        crate::bin_cli::output::payload(serde_json::to_string_pretty(&out)?);
    } else if !params.dry_run {
        for t in &created {
            crate::bin_cli::output::line(format!("Created and pushed tag: {t}"));
        }
    }

    Ok(created)
}

fn create_and_push_tag(new_tag: &str, source_ref: &str, remote: &str, workspace_root: &std::path::Path) -> Result<()> {
    // ~keep A non-zero `git rev-parse <tag>` legitimately means "tag absent", but a
    // spawn failure means nothing was learned, so it propagates instead of being read
    // as absence.
    let local_check = std::process::Command::new("git")
        .args(["rev-parse", new_tag])
        .current_dir(workspace_root)
        .output()
        .with_context(|| format!("git rev-parse {new_tag}"))?;

    if local_check.status.success() {
        tracing::warn!("  Tag {new_tag} already exists locally; skipping.");
        return Ok(());
    }

    let remote_check = std::process::Command::new("git")
        .args(["ls-remote", "--tags", remote])
        .current_dir(workspace_root)
        .output()
        .with_context(|| format!("git ls-remote --tags {remote}"))?;

    // ~keep `ls-remote` is the only evidence about remote tag state, and a failure
    // (auth, network, unknown remote) yields empty stdout — indistinguishable from
    // "tag absent" if only stdout is inspected. Continuing meant a transient failure
    // created the tag and pushed it with `--force-with-lease`, which for a tag ref has
    // no remote-tracking ref to lease against and so degrades to a plain force. Abort
    // rather than retry: a retry cannot fix auth or unknown-remote failures, and
    // re-running the release step costs far less than a wrong forced tag on a shared
    // remote.
    if !remote_check.status.success() {
        anyhow::bail!(
            "git ls-remote --tags {remote} failed ({}): cannot determine whether tag {new_tag} already exists, \
             refusing to create and force-push it.\n{}",
            remote_check.status,
            String::from_utf8_lossy(&remote_check.stderr).trim()
        );
    }

    if String::from_utf8_lossy(&remote_check.stdout)
        .lines()
        .any(|l| l.contains(&format!("refs/tags/{new_tag}")))
    {
        tracing::warn!("  Tag {new_tag} already exists on remote; skipping.");
        return Ok(());
    }

    let tag_status = std::process::Command::new("git")
        .args([
            "tag",
            "-a",
            new_tag,
            source_ref,
            "-m",
            &format!("Go module tag {new_tag}"),
        ])
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("git tag {new_tag}"))?;

    if !tag_status.success() {
        anyhow::bail!("git tag {new_tag} failed");
    }

    let push_status = std::process::Command::new("git")
        .args(["push", "--force-with-lease", remote, &format!("refs/tags/{new_tag}")])
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("git push tag {new_tag}"))?;

    if !push_status.success() {
        anyhow::bail!("git push for tag {new_tag} failed");
    }

    tracing::info!("  Tag {new_tag} created and pushed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_git_repo(dir: &std::path::Path) {
        Command::new("git").args(["init"]).current_dir(dir).output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("README.md"), "test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["tag", "-a", "v4.1.0", "HEAD", "-m", "Release v4.1.0"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    fn minimal_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["go"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn dry_run_prints_tags() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let config = minimal_config();
        let params = GoTagParams {
            version: "4.1.0",
            remote: "origin",
            dry_run: true,
            output_json: false,
            config: &config,
            workspace_root: tmp.path(),
        };
        let tags = run(&params).unwrap();
        assert!(!tags.is_empty());
        assert!(tags.iter().any(|t| t.contains("packages/go/v4/v4.1.0")));
        assert!(tags.iter().any(|t| t.contains("packages/go/v4.1.0")));
    }

    #[test]
    fn major_version_extracted() {
        let v = "4.1.0";
        let major: u64 = v.split('.').next().unwrap().parse().unwrap();
        assert_eq!(major, 4);
    }

    #[test]
    fn version_with_v_prefix_stripped() {
        let version = "v4.1.0".trim_start_matches('v');
        assert_eq!(version, "4.1.0");
    }

    #[test]
    fn dry_run_json_output() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let config = minimal_config();
        let params = GoTagParams {
            version: "4.0.0",
            remote: "origin",
            dry_run: true,
            output_json: true,
            config: &config,
            workspace_root: tmp.path(),
        };
        let result = run(&params);
        assert!(result.is_ok());
    }

    #[test]
    fn failed_ls_remote_aborts_before_creating_any_tag() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let config = minimal_config();
        let params = GoTagParams {
            version: "4.1.0",
            remote: "no-such-remote",
            dry_run: false,
            output_json: false,
            config: &config,
            workspace_root: tmp.path(),
        };

        let error = run(&params).expect_err("a failed ls-remote must abort, not be read as 'tag absent'");
        assert!(
            error.chain().any(|cause| cause.to_string().contains("ls-remote")),
            "error must name the failed remote read: {error:#}"
        );

        // ~keep The load-bearing assertion. Asserting only on the message would still
        // pass if the abort happened *after* `git tag` created the ref; the absence of
        // the tag is what proves the tag-creation and force-push path was never reached.
        let listed = Command::new("git")
            .args(["tag", "--list"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(
            !listed.contains("packages/go/"),
            "no Go module tag may exist after an unreadable remote: {listed}"
        );
    }
}
