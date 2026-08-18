use crate::cli::git::IgnoreFilter;
use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;

use super::version_text::replace_version_pattern;

pub(super) fn sync_python_versions(
    config: &ResolvedCrateConfig,
    version: &str,
    python_version: &str,
    writable: &IgnoreFilter,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    sync_python_pyproject_versions(config, python_version, updated)?;
    sync_python_init_versions(version, writable, updated)
}

fn sync_python_pyproject_versions(
    config: &ResolvedCrateConfig,
    python_version: &str,
    updated: &mut Vec<String>,
) -> anyhow::Result<()> {
    let pkg_dir = config.package_dir(Language::Python);
    let mut python_paths: Vec<String> = vec![
        "packages/python/pyproject.toml".to_string(),
        std::path::Path::new(&pkg_dir)
            .join("pyproject.toml")
            .to_string_lossy()
            .into_owned(),
    ];
    if let Some(output_dir) = config.output_for("python") {
        python_paths.push(output_dir.join("pyproject.toml").to_string_lossy().into_owned());
    }

    let mut seen_canonical: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    for python_path in python_paths {
        let canonical = match std::fs::canonicalize(&python_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !seen_canonical.insert(canonical) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&python_path)
            && let Some(new_content) = replace_version_pattern(&content, r#"version = "[^"]*""#, python_version)
        {
            std::fs::write(&python_path, &new_content).with_context(|| format!("failed to write {python_path}"))?;
            updated.push(python_path);
        }
    }

    Ok(())
}

/// A setuptools/hatch build mirrors the whole package into `packages/python/build/lib/`, which
/// this `**` pattern matches as readily as the source tree. Both copies carry the same
/// `__version__` line, so the write went to both and only one of them was ever reviewed. ~keep
fn sync_python_init_versions(version: &str, writable: &IgnoreFilter, updated: &mut Vec<String>) -> anyhow::Result<()> {
    for py_init in writable.glob("packages/python/**/__init__.py") {
        if let Ok(content) = std::fs::read_to_string(&py_init)
            && let Some(new_content) = replace_version_pattern(&content, r#"__version__\s*=\s*"[^"]*""#, version)
        {
            std::fs::write(&py_init, &new_content).with_context(|| format!("failed to write {}", py_init.display()))?;
            updated.push(py_init.to_string_lossy().to_string());
        }
    }

    Ok(())
}
