use super::validate_versions::VersionCheck;
use crate::core::config::{ResolvedCrateConfig, extras::Language};
use std::collections::{HashMap, HashSet};
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
    let submodules = registered_submodule_paths(workspace_root);
    let manifests = cargo_manifest_versions(workspace_root, canonical, &submodules);
    for lock_path in glob_under(workspace_root, "", "**/Cargo.lock") {
        if ignored_path(workspace_root, &lock_path, &submodules) {
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

fn cargo_manifest_versions(
    workspace_root: &Path,
    canonical: &str,
    submodules: &HashSet<PathBuf>,
) -> HashMap<String, String> {
    let mut versions = HashMap::new();
    for path in glob_under(workspace_root, "", "**/Cargo.toml") {
        if ignored_path(workspace_root, &path, submodules) {
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

// ~keep `vendor` (Cargo/Go-style vendoring) and `deps` (Mix's fetched-dependency
// cache) hold frozen copies of a crate's Cargo.toml pulled in at whatever version
// was current when they were vendored/fetched. `cargo_manifest_versions` keys its
// map by package name alone, so in a consumer repo a vendored `<crate>`/`<crate>_nif`
// manifest silently overwrote the live one's entry and poisoned every Cargo.lock
// comparison for that name repo-wide — not just the vendored copy's own lock.
fn ignored_path(workspace_root: &Path, path: &Path, submodules: &HashSet<PathBuf>) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | ".git" | ".alef-cache" | "vendor" | "deps")
        )
    }) || is_inside_unregistered_checkout(workspace_root, relative, submodules)
}

// ~keep A linked `git worktree` and a submodule are indistinguishable by their root
// marker — both carry a `.git` FILE pointing at the owning repository's gitdir (a
// nested full clone carries a `.git` directory). They are not interchangeable here:
// `.gitmodules` registers a submodule as a declared part of this repo, so its
// manifests belong in the version map, while an unregistered checkout under the tree
// is an independent worktree sitting at its own commit whose manifests are unrelated
// to this repo's version consistency — and, mid-regeneration, can differ between two
// runs of the same command. Only the *nearest* enclosing root matters, so this walks
// ancestors outward from `relative` instead of every path down from the workspace.
fn is_inside_unregistered_checkout(workspace_root: &Path, relative: &Path, submodules: &HashSet<PathBuf>) -> bool {
    let mut ancestor = workspace_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            // ~keep The final component is the manifest/lockfile itself, never a checkout root.
            break;
        }
        ancestor.push(component);
        if !submodules.contains(&ancestor) && ancestor.join(".git").exists() {
            return true;
        }
    }
    false
}

fn registered_submodule_paths(workspace_root: &Path) -> HashSet<PathBuf> {
    let mut registered = HashSet::new();
    let Ok(content) = std::fs::read_to_string(workspace_root.join(".gitmodules")) else {
        return registered;
    };
    let mut inside_submodule_section = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside_submodule_section = line.starts_with("[submodule");
            continue;
        }
        if !inside_submodule_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            registered.insert(workspace_root.join(value));
        }
    }
    registered
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

    /// Reproduces the false-positive MISMATCH observed in a consumer repo: a vendored/frozen
    /// copy of a crate's `Cargo.toml` (e.g. under a Rustler `vendor/` tree carried for
    /// offline builds) declares the same `name` as the live crate but at a stale,
    /// explicit version. `cargo_manifest_versions` keys its map by name alone, so
    /// without the `vendor` exclusion the vendored entry silently overwrites the live
    /// one and every *other* Cargo.lock's genuinely-matching `sample` entry gets
    /// compared against the wrong expected version.
    #[test]
    fn vendored_duplicate_package_name_does_not_poison_manifest_versions() {
        let temp = workspace("3.4.5");
        std::fs::create_dir_all(temp.path().join("vendor/sample")).expect("vendor directory");
        std::fs::write(
            temp.path().join("vendor/sample/Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"9.9.9\"\n",
        )
        .expect("vendored manifest");

        let nested = temp.path().join("packages/elixir/native/consumer");
        std::fs::create_dir_all(&nested).expect("lock directory");
        std::fs::write(
            nested.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
        )
        .expect("nested lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        let sample = checks
            .iter()
            .find(|check| check.label.ends_with("consumer/Cargo.lock#sample"))
            .expect("sample check present");
        assert!(
            sample.matches,
            "live entry matching canonical must not be poisoned by the vendored copy: {sample:?}"
        );
        assert!(
            checks.iter().all(|check| !check.label.contains("vendor/")),
            "vendored Cargo.lock/Cargo.toml must not be walked at all: {checks:?}"
        );
    }

    /// Same mechanism as the vendor case, but for Mix's fetched-dependency cache
    /// (`deps/`), which is how the consumer repo's `<crate>_nif` false positive actually
    /// occurred — a Hex-fetched copy of the elixir package under `deps/` bundles its own
    /// frozen `Cargo.toml` at the last-published version.
    #[test]
    fn deps_fetched_duplicate_package_name_does_not_poison_manifest_versions() {
        let temp = workspace("3.4.5");
        std::fs::create_dir_all(temp.path().join("test_apps/elixir/deps/sample_pkg/native/sample"))
            .expect("deps directory");
        std::fs::write(
            temp.path()
                .join("test_apps/elixir/deps/sample_pkg/native/sample/Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"1.8.1\"\n",
        )
        .expect("deps-fetched manifest");

        let nested = temp.path().join("packages/elixir/native/consumer");
        std::fs::create_dir_all(&nested).expect("lock directory");
        std::fs::write(
            nested.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
        )
        .expect("nested lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        let sample = checks
            .iter()
            .find(|check| check.label.ends_with("consumer/Cargo.lock#sample"))
            .expect("sample check present");
        assert!(
            sample.matches,
            "live entry matching canonical must not be poisoned by the deps-fetched copy: {sample:?}"
        );
    }

    /// Guard against the lazy fix: a genuinely stale local Cargo.lock (no vendor/deps
    /// involvement, a real path dependency that hasn't been `cargo update`d) must keep
    /// failing even in the presence of an unrelated vendored duplicate name elsewhere.
    #[test]
    fn genuine_drift_outside_vendor_still_fails() {
        let temp = workspace("3.4.5");
        std::fs::create_dir_all(temp.path().join("vendor/sample")).expect("vendor directory");
        std::fs::write(
            temp.path().join("vendor/sample/Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"9.9.9\"\n",
        )
        .expect("vendored manifest");

        let nested = temp.path().join("e2e/rust");
        std::fs::create_dir_all(&nested).expect("lock directory");
        std::fs::write(
            nested.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.0\"\n",
        )
        .expect("stale nested lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        let sample = checks
            .iter()
            .find(|check| check.label.ends_with("e2e/rust/Cargo.lock#sample"))
            .expect("sample check present");
        assert!(
            !sample.matches,
            "genuinely stale lockfile entry must still be reported as a mismatch: {sample:?}"
        );
        assert_eq!(sample.found.as_deref(), Some("3.4.0"));
    }

    /// Write a `.git` marker of the shape a linked worktree or submodule checkout
    /// carries: a FILE pointing at the owning repository's gitdir.
    fn write_git_marker(checkout_root: &Path, gitdir: &str) {
        std::fs::create_dir_all(checkout_root).expect("checkout root");
        std::fs::write(checkout_root.join(".git"), format!("gitdir: {gitdir}\n")).expect("git marker file");
    }

    /// A linked `git worktree` checked out inside the repo (e.g. under `.worktrees/`)
    /// is a different checkout at a different commit, and nothing in `.gitmodules`
    /// claims it as part of this repo. Its manifests must not be walked at all —
    /// neither to raise mismatches of their own nor to overwrite the live entry for a
    /// package name they happen to share.
    #[test]
    fn unregistered_nested_worktree_is_excluded_from_the_version_map() {
        let temp = workspace("3.4.5");
        std::fs::write(
            temp.path().join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.5\"\n",
        )
        .expect("root lock");
        let worktree_root = temp.path().join(".worktrees/scratch-lane");
        write_git_marker(&worktree_root, "../../.git/worktrees/scratch-lane");
        std::fs::write(
            worktree_root.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"9.0.0\"\n",
        )
        .expect("worktree manifest");
        std::fs::write(
            worktree_root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"3.4.0\"\n",
        )
        .expect("worktree lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        assert!(
            checks.iter().all(|check| !check.label.contains(".worktrees")),
            "no check should be produced for anything under the unregistered worktree: {checks:?}"
        );
        let sample = checks
            .iter()
            .find(|check| check.label == "Cargo.lock#sample")
            .expect("root sample check present");
        assert!(
            sample.matches,
            "the live root lockfile must not be poisoned by the worktree's stale manifest: {sample:?}"
        );
    }

    /// The other half of the same discriminator: a submodule checkout carries the very
    /// same `.git` marker file as a stray worktree, but `.gitmodules` registers its
    /// path, making it a declared part of this repo's version surface. Both scans must
    /// still descend into it — the `#subcrate` check can only exist if the submodule's
    /// `Cargo.toml` reached the manifest map *and* its `Cargo.lock` was walked.
    #[test]
    fn gitmodules_registered_submodule_is_still_validated() {
        let temp = workspace("3.4.5");
        std::fs::write(
            temp.path().join(".gitmodules"),
            "[submodule \"subcrate\"]\n\tpath = libs/subcrate\n\turl = https://example.invalid/subcrate.git\n",
        )
        .expect("gitmodules");
        let submodule_root = temp.path().join("libs/subcrate");
        write_git_marker(&submodule_root, "../../.git/modules/subcrate");
        std::fs::write(
            submodule_root.join("Cargo.toml"),
            "[package]\nname = \"subcrate\"\nversion = \"3.4.5\"\n",
        )
        .expect("submodule manifest");
        std::fs::write(
            submodule_root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"subcrate\"\nversion = \"3.4.0\"\n",
        )
        .expect("submodule lock");

        let checks = collect(&config(temp.path()), temp.path(), "3.4.5");
        let subcrate = checks
            .iter()
            .find(|check| check.label == "libs/subcrate/Cargo.lock#subcrate")
            .expect("registered submodule must still be validated");
        assert!(
            !subcrate.matches,
            "genuine drift inside a registered submodule must still be reported: {subcrate:?}"
        );
        assert_eq!(subcrate.found.as_deref(), Some("3.4.0"));
    }
}
