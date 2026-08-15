//! Check the alef version pin in `alef.toml` against the running alef CLI.
//!
//! Every alef.toml may carry a `[workspace] alef_version = "X.Y.Z"` field that
//! records the alef CLI version a project expects. Generation compares the pin to
//! the running CLI and warns on any mismatch, but version synchronization remains
//! an explicit release operation.

use crate::core::config::WorkspaceConfig;
use anyhow::Result;

/// CLI version baked in at compile time.
pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare `workspace.alef_version` against the running CLI and log the direction
/// of any change. Never errors: upgrades and downgrades are warned,
/// an equal or missing pin is silent.
pub fn check_alef_toml_version(workspace: &WorkspaceConfig) -> Result<()> {
    let Some(pin) = workspace.alef_version.as_deref() else {
        return Ok(());
    };
    let cli = cli_version();
    let (Ok(pin_v), Ok(cli_v)) = (semver::Version::parse(pin), semver::Version::parse(cli)) else {
        tracing::warn!(
            "alef.toml `[workspace] alef_version = \"{pin}\"` is not valid semver; running alef {cli} without changing the pin"
        );
        return Ok(());
    };

    match cli_v.cmp(&pin_v) {
        std::cmp::Ordering::Greater => {
            tracing::warn!(
                "Running alef {cli} is newer than the pinned alef_version {pin} in alef.toml; \
                 generation will not change the pin"
            );
        }
        std::cmp::Ordering::Less => {
            tracing::warn!(
                "Running alef {cli} is older than the pinned alef_version {pin} in alef.toml; \
                 generation will not change the pin"
            );
        }
        std::cmp::Ordering::Equal => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    fn workspace_with_version(v: Option<&str>) -> WorkspaceConfig {
        let mut toml = String::new();
        if let Some(version) = v {
            toml.push_str(&format!("alef_version = \"{version}\"\n"));
        }
        toml::from_str(&toml).expect("valid workspace config")
    }

    #[test]
    fn missing_pin_is_compatible() {
        let ws = workspace_with_version(None);
        assert!(check_alef_toml_version(&ws).is_ok());
    }

    #[test]
    fn pin_equal_to_cli_passes() {
        let ws = workspace_with_version(Some(cli_version()));
        assert!(check_alef_toml_version(&ws).is_ok());
    }

    #[test]
    #[traced_test]
    fn pin_lower_than_cli_warns_that_generation_preserves_pin() {
        let ws = workspace_with_version(Some("0.0.1"));
        assert!(check_alef_toml_version(&ws).is_ok());
        assert!(logs_contain("generation will not change the pin"));
    }

    #[test]
    fn pin_higher_than_cli_warns_not_errors() {
        let ws = workspace_with_version(Some("999.0.0"));
        assert!(
            check_alef_toml_version(&ws).is_ok(),
            "a downgrade must warn, not hard-error"
        );
    }

    #[test]
    fn pin_invalid_semver_warns_not_errors() {
        let ws = workspace_with_version(Some("not-a-version"));
        assert!(
            check_alef_toml_version(&ws).is_ok(),
            "an unparseable pin must warn and continue, not error"
        );
    }

    #[test]
    fn version_check_does_not_rewrite_external_pin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        let config = "[workspace]\nlanguages = []\nalef_version = \"0.0.1\"\n";
        std::fs::write(&path, config).expect("write fixture");
        let workspace: WorkspaceConfig = toml::from_str(config).expect("parse fixture");

        check_alef_toml_version(&workspace).expect("check pin");

        assert_eq!(std::fs::read_to_string(path).expect("read fixture"), config);
    }
}
