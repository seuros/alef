use anyhow::Context as _;
use std::ffi::OsStr;
use std::path::PathBuf;

const FLUTTER_RUST_BRIDGE_CODEGEN: &str = "flutter_rust_bridge_codegen";

/// `flutter_rust_bridge_codegen` treats the presence of `CARGO_MANIFEST_DIR` in its own
/// environment as proof that it is running nested inside an already-active `cargo` invocation
/// (Cargo sets this variable only for processes it spawns itself, such as a build script) and,
/// to avoid deadlocking on that outer invocation's jobserver, silently skips its `cargo-expand`
/// macro/cfg-expansion pass — logging "Skip cargo-expand ... because cargo is already running
/// and would block cargo-expand" — and falls back to a raw, cfg-unaware `syn` parse that emits
/// bindings for every `pub fn` regardless of whether its `#[cfg(feature = ...)]` gate is
/// actually active. This was confirmed empirically (alef #140) by bisecting a build script's
/// full captured environment down to this one variable: with it present, a bare, non-nested
/// invocation reproduces the exact same degraded output as a real `cargo build`-nested one;
/// without it, the same invocation runs a real `cargo-expand` and correctly excludes
/// feature-gated functions whose feature is not active for the crate.
///
/// alef's own invocation must always take the full, cfg-aware path, regardless of how alef
/// itself happens to be launched (an installed binary has no such variable, but `cargo run`
/// during alef's own development would set it for alef's process and, by default child-process
/// env inheritance, for this command too). Stripped unconditionally rather than relying on
/// alef's own launch context staying clean forever.
const CARGO_MANIFEST_DIR_ENV: &str = "CARGO_MANIFEST_DIR";

pub(super) fn configure(command: &mut std::process::Command, cmd: &str, cache_scope: &str) -> anyhow::Result<()> {
    if cmd != FLUTTER_RUST_BRIDGE_CODEGEN {
        return Ok(());
    }

    command.env_remove(CARGO_MANIFEST_DIR_ENV);

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

    /// alef #140: `flutter_rust_bridge_codegen` degrades to a cfg-unaware parse whenever
    /// `CARGO_MANIFEST_DIR` is present in its environment (see `CARGO_MANIFEST_DIR_ENV`'s doc
    /// for how this was established). `configure` must strip it from the spawned command's
    /// environment unconditionally, regardless of whether it happens to be set in alef's own
    /// process -- `Command::env_remove` records an explicit removal that `get_envs` reports as
    /// `None`, which is what proves the child would not inherit an ambient value.
    #[test]
    fn configure_strips_cargo_manifest_dir_from_the_frb_command() {
        let mut command = std::process::Command::new(FLUTTER_RUST_BRIDGE_CODEGEN);
        configure(&mut command, FLUTTER_RUST_BRIDGE_CODEGEN, "sample-api").expect("configure must succeed");

        let removed = command
            .get_envs()
            .any(|(key, value)| key == OsStr::new(CARGO_MANIFEST_DIR_ENV) && value.is_none());
        assert!(
            removed,
            "CARGO_MANIFEST_DIR must be explicitly removed from the frb command's env"
        );
    }

    /// `configure` only touches `flutter_rust_bridge_codegen` invocations (see the `cmd !=
    /// FLUTTER_RUST_BRIDGE_CODEGEN` early return) -- it must not strip `CARGO_MANIFEST_DIR`
    /// from an unrelated post-build `RunCommand`, which may legitimately depend on it.
    #[test]
    fn configure_leaves_other_commands_env_untouched() {
        let mut command = std::process::Command::new("echo");
        configure(&mut command, "echo", "sample-api").expect("configure must succeed");

        let touched = command
            .get_envs()
            .any(|(key, _)| key == OsStr::new(CARGO_MANIFEST_DIR_ENV));
        assert!(
            !touched,
            "configure must not touch env for commands other than flutter_rust_bridge_codegen"
        );
    }

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
