use anyhow::Context as _;
use std::ffi::OsStr;
use std::path::PathBuf;

const FLUTTER_RUST_BRIDGE_CODEGEN: &str = "flutter_rust_bridge_codegen";

pub(super) fn configure(command: &mut std::process::Command, cmd: &str, cache_scope: &str) -> anyhow::Result<()> {
    if cmd != FLUTTER_RUST_BRIDGE_CODEGEN {
        return Ok(());
    }

    let xdg_cache_home = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    let local_app_data = std::env::var_os("LOCALAPPDATA");

    if let Some(cache_path) = managed_fvm_cache_path(
        std::env::var_os("FVM_CACHE_PATH").as_deref(),
        std::env::var_os("FVM_HOME").as_deref(),
        xdg_cache_home.as_deref(),
        home.as_deref(),
        local_app_data.as_deref(),
    ) {
        std::fs::create_dir_all(&cache_path)
            .with_context(|| format!("failed to create FVM cache at {}", cache_path.display()))?;
        command.env("FVM_CACHE_PATH", cache_path);
    }

    if let Some(target_path) = managed_frb_cargo_target_path(
        std::env::var_os("CARGO_TARGET_DIR").as_deref(),
        xdg_cache_home.as_deref(),
        home.as_deref(),
        local_app_data.as_deref(),
        cache_scope,
    ) {
        std::fs::create_dir_all(&target_path)
            .with_context(|| format!("failed to create FRB Cargo target cache at {}", target_path.display()))?;
        command.env("CARGO_TARGET_DIR", target_path);
    }
    Ok(())
}

fn fallback_alef_cache_root(
    xdg_cache_home: Option<&OsStr>,
    home: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    xdg_cache_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".cache"))
        })
        .or_else(|| local_app_data.filter(|value| !value.is_empty()).map(PathBuf::from))
        .map(|root| root.join("alef"))
}

fn managed_fvm_cache_path(
    fvm_cache_path: Option<&OsStr>,
    fvm_home: Option<&OsStr>,
    xdg_cache_home: Option<&OsStr>,
    home: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    if fvm_cache_path.is_some() || fvm_home.is_some() {
        return None;
    }
    fallback_alef_cache_root(xdg_cache_home, home, local_app_data).map(|root| root.join("fvm"))
}

fn managed_frb_cargo_target_path(
    cargo_target_dir: Option<&OsStr>,
    xdg_cache_home: Option<&OsStr>,
    home: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
    cache_scope: &str,
) -> Option<PathBuf> {
    if cargo_target_dir.is_some() {
        return None;
    }
    fallback_alef_cache_root(xdg_cache_home, home, local_app_data).map(|root| {
        root.join("cargo-targets")
            .join("flutter-rust-bridge")
            .join(cache_scope_component(cache_scope))
    })
}

fn cache_scope_component(scope: &str) -> String {
    scope
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fvm_cache_is_stable_across_clean_worktrees() {
        let cache = managed_fvm_cache_path(None, None, Some(OsStr::new("/cache")), None, None);
        assert_eq!(cache, Some(PathBuf::from("/cache/alef/fvm")));
        assert_eq!(
            cache,
            managed_fvm_cache_path(None, None, Some(OsStr::new("/cache")), None, None)
        );
    }

    #[test]
    fn fvm_cache_uses_platform_fallbacks() {
        assert_eq!(
            managed_fvm_cache_path(None, None, None, Some(OsStr::new("/users/example")), None),
            Some(PathBuf::from("/users/example/.cache/alef/fvm"))
        );
        assert_eq!(
            managed_fvm_cache_path(
                None,
                None,
                None,
                None,
                Some(OsStr::new("C:/Users/example/AppData/Local"))
            ),
            Some(PathBuf::from("C:/Users/example/AppData/Local/alef/fvm"))
        );
        assert_eq!(managed_fvm_cache_path(None, None, None, None, None), None);
    }

    #[test]
    fn explicit_fvm_cache_settings_remain_authoritative() {
        assert_eq!(
            managed_fvm_cache_path(
                Some(OsStr::new("/custom/fvm")),
                None,
                Some(OsStr::new("/cache")),
                None,
                None
            ),
            None
        );
        assert_eq!(
            managed_fvm_cache_path(
                None,
                Some(OsStr::new("/legacy/fvm")),
                Some(OsStr::new("/cache")),
                None,
                None
            ),
            None
        );
    }

    #[test]
    fn frb_cargo_target_cache_is_stable_and_scoped() {
        let first = managed_frb_cargo_target_path(None, Some(OsStr::new("/cache")), None, None, "sample-api");
        let second = managed_frb_cargo_target_path(None, Some(OsStr::new("/cache")), None, None, "sample-api");
        assert_eq!(first, second);
        assert_eq!(
            first,
            Some(PathBuf::from(
                "/cache/alef/cargo-targets/flutter-rust-bridge/sample-api"
            ))
        );
    }

    #[test]
    fn explicit_cargo_target_dir_remains_authoritative() {
        assert_eq!(
            managed_frb_cargo_target_path(
                Some(OsStr::new("/custom/target")),
                Some(OsStr::new("/cache")),
                None,
                None,
                "sample-api"
            ),
            None
        );
    }

    #[test]
    fn cargo_target_scope_is_safe_for_a_path_component() {
        assert_eq!(cache_scope_component("sample/api:core"), "sample_api_core");
    }
}
