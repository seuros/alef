//! The single call every `bin_cli` command that resolves a workspace routes through to keep
//! `alef.toml`'s `[workspace] alef_version` pin honest.
//!
//! `cli::version_pin` implements the check and the guarded, opt-in rewrite as two separate,
//! independently unit-tested functions and makes no policy decision of its own about when the
//! rewrite is allowed to run -- see that module's doc. This module is the one place that resolves
//! the policy (the `[workspace] auto_update_alef_version` toggle) and the one build-provenance
//! signal (`bin_cli::build_info::running_build_is_clean`) the rewrite needs, and calls both
//! `cli::version_pin` functions in sequence. Every command arm that used to call
//! `version_pin::check_alef_toml_version` alone now calls [`sync_alef_version_pin`] instead, so
//! auditing "is the auto-update wired up" is one grep for this function's name across
//! `all_commands.rs`, `core_commands.rs`, and `core_commands/generate.rs`, rather than four
//! independent call sites that could each individually drift out of sync with the other three. ~keep

use anyhow::Result;

use crate::core::config::WorkspaceConfig;

/// Check `workspace.alef_version` against the running CLI (always) and, only when every safety
/// condition [`crate::cli::version_pin::maybe_update_alef_toml_version_pin`] documents also
/// holds, rewrite it to match. `auto_update_enabled` is resolved here from
/// `workspace.auto_update_alef_version` rather than left to the caller, so every command arm
/// shares the exact same opt-in source. `is_build_clean` stays an explicit parameter rather than
/// reading `bin_cli::build_info::running_build_is_clean()` internally -- production call sites
/// pass that real value, but keeping it a parameter is what lets this function's own tests below
/// exercise both the clean and dirty paths deterministically, independent of whatever tree state
/// the test binary itself happened to be built from. ~keep
pub(crate) fn sync_alef_version_pin(
    workspace: &WorkspaceConfig,
    config_path: &std::path::Path,
    is_build_clean: bool,
) -> Result<()> {
    crate::cli::version_pin::check_alef_toml_version(workspace)?;
    crate::cli::version_pin::maybe_update_alef_toml_version_pin(
        workspace,
        config_path,
        workspace.auto_update_alef_version,
        is_build_clean,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(pin: &str, auto_update_alef_version: bool) -> WorkspaceConfig {
        let toml = format!("alef_version = \"{pin}\"\nauto_update_alef_version = {auto_update_alef_version}\n");
        toml::from_str(&toml).expect("valid workspace config")
    }

    fn write_fixture_alef_toml(dir: &std::path::Path, pin: &str, auto_update_alef_version: bool) -> std::path::PathBuf {
        let path = dir.join("alef.toml");
        let config = format!(
            "[workspace]\nalef_version = \"{pin}\"\nauto_update_alef_version = {auto_update_alef_version}\nlanguages = []\n"
        );
        std::fs::write(&path, config).expect("write fixture");
        path
    }

    /// The end-to-end positive case every command arm relies on: opted in via the workspace
    /// toggle (not a raw `bool` the test injects directly), clean build, pin older than the
    /// running CLI -- the pin is rewritten. Exercises the exact call sequence
    /// `all_commands.rs`/`core_commands.rs`/`generate.rs` now use, not just
    /// `maybe_update_alef_toml_version_pin` in isolation.
    #[test]
    fn opted_in_clean_and_newer_rewrites_the_pin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1", true);
        let workspace = workspace_with("0.0.1", true);

        sync_alef_version_pin(&workspace, &path, true).expect("must not error");

        let after = std::fs::read_to_string(&path).expect("read fixture");
        assert!(
            after.contains(&format!(
                "alef_version = \"{}\"",
                crate::cli::version_pin::cli_version()
            )),
            "pin must be rewritten to the running CLI version:\n{after}"
        );
    }

    /// `workspace.auto_update_alef_version` defaults to `false` (see its own default-empty
    /// deserialize test in `core::config::workspace`), and this function must resolve the opt-in
    /// from that field rather than assume it -- a clean, newer, eligible pin still must not be
    /// rewritten when the workspace never turned the toggle on.
    #[test]
    fn opted_out_by_default_does_not_rewrite_even_when_otherwise_eligible() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1", false);
        let workspace = workspace_with("0.0.1", false);
        let before = std::fs::read_to_string(&path).expect("read fixture");

        sync_alef_version_pin(&workspace, &path, true).expect("must not error");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read fixture"),
            before,
            "auto_update_alef_version defaults to off; the pin must not move without an explicit opt-in"
        );
    }

    /// REGRESSION: a dirty build must never rewrite the pin even when the workspace opted in --
    /// mirrors `version_pin::auto_update_pin_does_not_write_from_a_dirty_build`, but through this
    /// module's own entry point, so a future refactor that drops the `is_build_clean` argument
    /// from the call this function makes fails here too, not only in `cli::version_pin`'s own
    /// test module.
    #[test]
    fn opted_in_but_dirty_build_does_not_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1", true);
        let workspace = workspace_with("0.0.1", true);
        let before = std::fs::read_to_string(&path).expect("read fixture");

        sync_alef_version_pin(&workspace, &path, false).expect("must not error");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read fixture"),
            before,
            "a dirty build must never rewrite the pin, even opted in"
        );
    }

    /// STRUCTURAL: proves the CLI itself calls this function -- not merely that this function
    /// behaves correctly in isolation. This is exactly the failure mode #465 shipped with:
    /// `maybe_update_alef_toml_version_pin` had 12 passing unit tests and zero call sites. Reads
    /// each command-arm source file as text and asserts it calls `sync_alef_version_pin(`, so a
    /// future edit that reverts a call site back to `check_alef_toml_version` alone (or never
    /// adds one to a new command) fails a test instead of shipping silently inert again. ~keep
    #[test]
    fn every_known_command_arm_calls_sync_alef_version_pin() {
        let call_sites: &[(&str, &str)] = &[
            ("all_commands.rs", include_str!("all_commands.rs")),
            ("core_commands.rs", include_str!("core_commands.rs")),
            ("core_commands/generate.rs", include_str!("core_commands/generate.rs")),
        ];
        for (name, source) in call_sites {
            let occurrences = source.matches("sync_alef_version_pin(").count();
            assert!(
                occurrences > 0,
                "{name} must call version_pin_sync::sync_alef_version_pin at least once"
            );
        }
        // `all_commands.rs`'s `Commands::All` arm resolves the workspace twice (once up front,
        // once again after a registry-mode version sync may have rewritten `alef.toml`) and must
        // sync the pin after each resolution -- see the call sites this guards.
        let all_commands_source = call_sites[0].1;
        assert_eq!(
            all_commands_source.matches("sync_alef_version_pin(").count(),
            2,
            "Commands::All must sync the pin both after the initial config load and after a \
             registry-version-sync reload"
        );
    }
}
