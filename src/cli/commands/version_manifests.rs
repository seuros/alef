use super::validate_versions::VersionCheck;
use crate::core::config::{ResolvedCrateConfig, extras::Language};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(super) fn collect(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let mut checks = Vec::new();
    collect_csproj_checks(config, workspace_root, canonical, &mut checks);
    collect_single_manifest_checks(config, workspace_root, canonical, &mut checks);
    collect_cargo_lock_checks(workspace_root, canonical, &mut checks);
    checks.sort_by(|left, right| left.label.cmp(&right.label));
    checks
}

fn collect_csproj_checks(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    checks: &mut Vec<VersionCheck>,
) {
    let directory = config.package_dir(Language::Csharp);
    let assembly_version = crate::core::version::to_dotnet_assembly_version(canonical);
    for path in glob_under(workspace_root, &directory, "**/*.csproj") {
        let Some(content) = std::fs::read_to_string(&path).ok() else {
            continue;
        };
        for field in ["Version", "AssemblyVersion", "FileVersion", "InformationalVersion"] {
            let Some(found) = read_xml_element(&content, field) else {
                continue;
            };
            // ~keep The generator stamps these two fields through `to_dotnet_assembly_version`
            // because .NET rejects SemVer prereleases in them, so comparing against raw
            // canonical would flag alef's own required output as a mismatch forever.
            // `Version` and `InformationalVersion` carry the full SemVer and compare raw.
            let expected = match field {
                "AssemblyVersion" | "FileVersion" => assembly_version.as_str(),
                _ => canonical,
            };
            push_check(checks, workspace_root, &path, Some(field), found, expected);
        }
    }
}

fn collect_single_manifest_checks(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    checks: &mut Vec<VersionCheck>,
) {
    let dart = workspace_root
        .join(config.package_dir(Language::Dart))
        .join("pubspec.yaml");
    if let Some(found) = read_prefixed_value(&dart, "version:") {
        push_check(checks, workspace_root, &dart, None, found, canonical);
    }

    let zig = workspace_root
        .join(config.package_dir(Language::Zig))
        .join("build.zig.zon");
    if let Some(found) = read_zig_version(&zig) {
        push_check(checks, workspace_root, &zig, None, found, canonical);
    }
}

fn collect_cargo_lock_checks(workspace_root: &Path, canonical: &str, checks: &mut Vec<VersionCheck>) {
    let manifests = cargo_manifest_versions(workspace_root, canonical);
    for lock_path in glob_under(workspace_root, "", "**/Cargo.lock") {
        if ignored_path(workspace_root, &lock_path) {
            continue;
        }
        let Some(content) = std::fs::read_to_string(&lock_path).ok() else {
            continue;
        };
        let Some(packages) = toml::from_str::<toml::Value>(&content)
            .ok()
            .and_then(|value| value.get("package").and_then(toml::Value::as_array).cloned())
        else {
            continue;
        };
        for package in packages {
            if package.get("source").is_some() {
                continue;
            }
            let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(found) = package.get("version").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(expected) = manifests.get(name) else {
                continue;
            };
            push_check(
                checks,
                workspace_root,
                &lock_path,
                Some(name),
                found.to_string(),
                expected,
            );
        }
    }
}

fn cargo_manifest_versions(workspace_root: &Path, canonical: &str) -> HashMap<String, String> {
    let mut versions = HashMap::new();
    for path in glob_under(workspace_root, "", "**/Cargo.toml") {
        if ignored_path(workspace_root, &path) {
            continue;
        }
        let Some(content) = std::fs::read_to_string(path).ok() else {
            continue;
        };
        let Some(package) = toml::from_str::<toml::Value>(&content)
            .ok()
            .and_then(|value| value.get("package").and_then(toml::Value::as_table).cloned())
        else {
            continue;
        };
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let version = package
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or(canonical);
        versions.insert(name.to_string(), version.to_string());
    }
    versions
}

fn glob_under(workspace_root: &Path, directory: &str, suffix: &str) -> Vec<PathBuf> {
    let root = glob::Pattern::escape(&workspace_root.to_string_lossy());
    let directory = directory.trim_matches(['/', '\\']);
    let pattern = if directory.is_empty() {
        format!("{root}/{suffix}")
    } else {
        format!("{root}/{directory}/{suffix}")
    };
    glob::glob(&pattern).into_iter().flatten().flatten().collect()
}

fn ignored_path(workspace_root: &Path, path: &Path) -> bool {
    path.strip_prefix(workspace_root).is_ok_and(|relative| {
        relative
            .components()
            .any(|component| matches!(component.as_os_str().to_str(), Some("target" | ".git" | ".alef-cache")))
    })
}

fn read_xml_element(content: &str, element: &str) -> Option<String> {
    let open = format!("<{element}>");
    let close = format!("</{element}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)?;
    Some(content[start..start + end].trim().to_string())
}

fn read_prefixed_value(path: &Path, prefix: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .map(|value| value.split('#').next().unwrap_or(value).trim().to_string())
    })
}

fn read_zig_version(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|line| {
        line.trim()
            .strip_prefix(".version")?
            .split_once('=')
            .map(|(_, value)| value.trim().trim_end_matches(',').trim_matches('"').to_string())
    })
}

fn push_check(
    checks: &mut Vec<VersionCheck>,
    workspace_root: &Path,
    path: &Path,
    field: Option<&str>,
    found: String,
    expected: &str,
) {
    let mut label = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if let Some(field) = field {
        label.push('#');
        label.push_str(field);
    }
    checks.push(VersionCheck {
        label,
        matches: found == expected,
        found: Some(found),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config(root: &Path) -> ResolvedCrateConfig {
        let manifest = root.join("Cargo.toml").to_string_lossy().replace('\\', "/");
        let source = format!(
            "[workspace]\nlanguages = [\"csharp\", \"dart\", \"zig\"]\n\
             [[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\nversion_from = \"{manifest}\"\n"
        );
        let parsed: crate::core::config::NewAlefConfig = toml::from_str(&source).expect("config parses");
        parsed.resolve().expect("config resolves").remove(0)
    }

    fn workspace(version: &str) -> TempDir {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(
            temp.path().join("Cargo.toml"),
            format!(
                "[workspace]\nmembers = [\"crates/sample\", \"crates/helper\"]\n\
                 [workspace.package]\nversion = \"{version}\"\n"
            ),
        )
        .expect("workspace manifest");
        for name in ["sample", "helper"] {
            let directory = temp.path().join("crates").join(name);
            std::fs::create_dir_all(&directory).expect("crate directory");
            std::fs::write(
                directory.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion.workspace = true\n"),
            )
            .expect("crate manifest");
        }
        temp
    }

    #[test]
    fn discovers_nested_csproj_and_checks_all_version_fields() {
        let temp = workspace("1.2.3");
        let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
        std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
        std::fs::write(
            &project,
            concat!(
                "<Project><PropertyGroup>",
                "<Version>1.2.3</Version>",
                "<AssemblyVersion>1.2.2</AssemblyVersion>",
                "<FileVersion>1.2.1</FileVersion>",
                "<InformationalVersion>1.2.0</InformationalVersion>",
                "</PropertyGroup></Project>",
            ),
        )
        .expect("csproj");

        let checks = collect(&config(temp.path()), temp.path(), "1.2.3");
        let project_checks: Vec<_> = checks
            .iter()
            .filter(|check| check.label.contains("Sample.csproj#"))
            .collect();
        assert_eq!(project_checks.len(), 4);
        assert!(
            project_checks
                .iter()
                .any(|check| check.label.ends_with("#Version") && check.matches)
        );
        for field in ["AssemblyVersion", "FileVersion", "InformationalVersion"] {
            assert!(
                project_checks
                    .iter()
                    .any(|check| check.label.ends_with(&format!("#{field}")) && !check.matches)
            );
        }
    }

    /// Write a csproj whose four version fields carry exactly what the generator
    /// emits for `canonical`, then return its checks.
    fn generator_shaped_csproj_checks(canonical: &str) -> Vec<VersionCheck> {
        let temp = workspace(canonical);
        let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
        std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
        let assembly = crate::core::version::to_dotnet_assembly_version(canonical);
        std::fs::write(
            &project,
            format!(
                "<Project><PropertyGroup>\
                 <Version>{canonical}</Version>\
                 <AssemblyVersion>{assembly}</AssemblyVersion>\
                 <FileVersion>{assembly}</FileVersion>\
                 <InformationalVersion>{canonical}</InformationalVersion>\
                 </PropertyGroup></Project>",
            ),
        )
        .expect("csproj");

        collect(&config(temp.path()), temp.path(), canonical)
            .into_iter()
            .filter(|check| check.label.contains("Sample.csproj#"))
            .collect()
    }

    #[test]
    fn generated_four_component_assembly_version_is_not_reported_as_a_mismatch() {
        let checks = generator_shaped_csproj_checks("1.17.0");
        assert_eq!(checks.len(), 4);
        for check in &checks {
            assert!(
                check.matches,
                "generator output must validate clean, but {} reported {:?}",
                check.label, check.found
            );
        }
    }

    #[test]
    fn prerelease_assembly_version_matches_while_semver_fields_keep_the_prerelease() {
        let checks = generator_shaped_csproj_checks("1.9.0-rc.48");
        let find = |suffix: &str| {
            checks
                .iter()
                .find(|check| check.label.ends_with(suffix))
                .unwrap_or_else(|| panic!("missing {suffix}"))
        };
        for suffix in ["#AssemblyVersion", "#FileVersion"] {
            let check = find(suffix);
            assert!(check.matches, "{suffix} should match: {:?}", check.found);
            assert_eq!(check.found.as_deref(), Some("1.9.0.0"));
        }
        for suffix in ["#Version", "#InformationalVersion"] {
            let check = find(suffix);
            assert!(check.matches, "{suffix} should match: {:?}", check.found);
            assert_eq!(check.found.as_deref(), Some("1.9.0-rc.48"));
        }
    }

    #[test]
    fn semver_fields_still_reject_the_four_component_assembly_form() {
        let temp = workspace("1.17.0");
        let project = temp.path().join("packages/csharp/src/Sample/Sample.csproj");
        std::fs::create_dir_all(project.parent().expect("project parent")).expect("project directory");
        std::fs::write(
            &project,
            concat!(
                "<Project><PropertyGroup>",
                "<Version>1.17.0.0</Version>",
                "<InformationalVersion>1.17.0.0</InformationalVersion>",
                "</PropertyGroup></Project>",
            ),
        )
        .expect("csproj");

        let checks = collect(&config(temp.path()), temp.path(), "1.17.0");
        for suffix in ["#Version", "#InformationalVersion"] {
            let check = checks
                .iter()
                .find(|check| check.label.ends_with(suffix))
                .unwrap_or_else(|| panic!("missing {suffix}"));
            assert!(
                !check.matches,
                "{suffix} carries full SemVer and must not accept the assembly form"
            );
        }
    }

    #[test]
    fn checks_dart_and_zig_manifests() {
        let temp = workspace("2.0.0");
        std::fs::create_dir_all(temp.path().join("packages/dart")).expect("dart directory");
        std::fs::write(
            temp.path().join("packages/dart/pubspec.yaml"),
            "name: sample\nversion: 2.0.0 # release\n",
        )
        .expect("pubspec");
        std::fs::create_dir_all(temp.path().join("packages/zig")).expect("zig directory");
        std::fs::write(
            temp.path().join("packages/zig/build.zig.zon"),
            ".{\n    .name = \"sample\",\n    .version = \"1.9.0\",\n}\n",
        )
        .expect("zig manifest");

        let checks = collect(&config(temp.path()), temp.path(), "2.0.0");
        let dart = checks
            .iter()
            .find(|check| check.label.ends_with("pubspec.yaml"))
            .expect("dart check");
        let zig = checks
            .iter()
            .find(|check| check.label.ends_with("build.zig.zon"))
            .expect("zig check");
        assert!(dart.matches);
        assert!(!zig.matches);
    }

    #[test]
    fn checks_each_local_package_in_every_cargo_lock() {
        let temp = workspace("3.4.5");
        std::fs::write(
            temp.path().join("crates/helper/Cargo.toml"),
            "[package]\nname = \"helper\"\nversion = \"4.0.0\"\n",
        )
        .expect("versioned helper manifest");
        let nested = temp.path().join("packages/elixir/native/sample");
        std::fs::create_dir_all(&nested).expect("lock directory");
        let lock = concat!(
            "version = 4\n\n",
            "[[package]]\nname = \"sample\"\nversion = \"3.4.4\"\n\n",
            "[[package]]\nname = \"helper\"\nversion = \"4.0.0\"\n\n",
            "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://example.invalid/index\"\n",
        );
        std::fs::write(nested.join("Cargo.lock"), lock).expect("nested lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        let lock_checks: Vec<_> = checks
            .iter()
            .filter(|check| check.label.contains("Cargo.lock#"))
            .collect();
        assert_eq!(lock_checks.len(), 2);
        assert!(
            lock_checks
                .iter()
                .any(|check| check.label.ends_with("#sample") && !check.matches)
        );
        assert!(
            lock_checks
                .iter()
                .any(|check| check.label.ends_with("#helper") && check.matches)
        );
        assert!(lock_checks.iter().all(|check| !check.label.contains("//")));
    }
}
