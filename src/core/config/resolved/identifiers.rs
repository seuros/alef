//! Go/Java/Kotlin/C# identifier methods and reverse-DNS derivations.

use super::ResolvedCrateConfig;
use crate::core::config::derive::{derive_go_module_from_repo, derive_reverse_dns_package};

impl ResolvedCrateConfig {
    /// Get the GitHub repository URL, returning an error when no source has it set.
    ///
    /// Resolution order:
    /// 1. `[e2e.registry] github_repo`
    /// 2. `[package_metadata] repository`
    /// 3. `[scaffold] repository`
    pub fn try_github_repo(&self) -> Result<String, String> {
        if let Some(e2e) = &self.e2e
            && let Some(url) = &e2e.registry.github_repo
        {
            return Ok(url.clone());
        }
        if let Some(url) = self.package_metadata.as_ref().and_then(|p| p.repository.as_ref()) {
            return Ok(url.clone());
        }
        if let Some(url) = self.scaffold.as_ref().and_then(|s| s.repository.as_ref()) {
            return Ok(url.clone());
        }
        Err(format!(
            "no repository URL configured — set `[scaffold] repository = \"...\"` (or `[e2e.registry] github_repo`) for crate `{}`",
            self.name
        ))
    }

    /// Get the GitHub repository URL with a vendor-neutral placeholder fallback.
    pub fn github_repo(&self) -> String {
        self.try_github_repo()
            .unwrap_or_else(|_| format!("https://example.invalid/{}", self.name))
    }

    /// Get the Go module path, returning an error when neither `[go].module`
    /// nor a derivable repository URL is configured.
    pub(crate) fn try_go_module(&self) -> Result<String, String> {
        if let Some(module) = self.go.as_ref().and_then(|g| g.module.as_ref()) {
            return Ok(module.clone());
        }
        if let Ok(repo) = self.try_github_repo()
            && let Some(module) = derive_go_module_from_repo(&repo)
        {
            return Ok(module);
        }
        Err(format!(
            "no Go module configured — set `[go] module = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Go module path with a vendor-neutral placeholder fallback.
    pub fn go_module(&self) -> String {
        self.try_go_module()
            .unwrap_or_else(|_| format!("example.invalid/{}", self.name))
    }

    /// Get the Go package name (e.g. `"samplecrate"`).
    ///
    /// Returns `[go] package_name` if set, otherwise derives it from the last segment of
    /// [`Self::go_module`]. This is the single source every Go-targeting generator (the Go
    /// backend's binding file, the error-type generator, the e2e/docs snippet generator) must
    /// call for the package name, so they can never disagree about it.
    pub fn go_package_name(&self) -> String {
        self.go
            .as_ref()
            .and_then(|g| g.package_name.clone())
            .unwrap_or_else(|| crate::codegen::naming::go_package_name_from_module(&self.go_module()))
    }

    /// Get the Java package name, returning an error when neither `[java].package`
    /// nor a derivable repository URL is configured.
    pub(crate) fn try_java_package(&self) -> Result<String, String> {
        if let Some(pkg) = self.java.as_ref().and_then(|j| j.package.as_ref()) {
            return Ok(pkg.clone());
        }
        if let Ok(repo) = self.try_github_repo()
            && let Some(pkg) = derive_reverse_dns_package(&repo)
        {
            return Ok(pkg);
        }
        Err(format!(
            "no Java package configured — set `[java] package = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Java package name with a vendor-neutral placeholder fallback.
    pub fn java_package(&self) -> String {
        self.try_java_package()
            .unwrap_or_else(|_| "unconfigured.alef".to_string())
    }

    /// Get the Java Maven groupId.
    ///
    /// Prefers the explicit `[java] group_id` override when set; otherwise falls back to
    /// the Java package name (most projects publish under `groupId == java package`).
    pub fn java_group_id(&self) -> String {
        if let Some(gid) = self.java.as_ref().and_then(|j| j.group_id.as_ref()) {
            return gid.clone();
        }
        self.java_package()
    }

    /// Get the Java Maven artifactId.
    ///
    /// Prefers the explicit `[java] artifact_id` override; otherwise falls back to the
    /// crate name (`[[crates]] name`).
    pub fn java_artifact_id(&self) -> String {
        self.java
            .as_ref()
            .and_then(|j| j.artifact_id.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }

    /// Get the Kotlin package name, returning an error when neither
    /// `[kotlin].package` nor a derivable repository URL is configured.
    pub(crate) fn try_kotlin_package(&self) -> Result<String, String> {
        if let Some(pkg) = self.kotlin.as_ref().and_then(|k| k.package.as_ref()) {
            return Ok(pkg.clone());
        }
        if let Ok(repo) = self.try_github_repo()
            && let Some(pkg) = derive_reverse_dns_package(&repo)
        {
            return Ok(pkg);
        }
        Err(format!(
            "no Kotlin package configured — set `[kotlin] package = \"...\"` or `[scaffold] repository = \"https://<host>/<org>/...\"` for crate `{}`",
            self.name
        ))
    }

    /// Get the Kotlin package name with a vendor-neutral placeholder fallback.
    pub fn kotlin_package(&self) -> String {
        self.try_kotlin_package()
            .unwrap_or_else(|_| "unconfigured.alef".to_string())
    }

    /// Get the C# namespace.
    pub fn csharp_namespace(&self) -> String {
        self.csharp
            .as_ref()
            .and_then(|c| c.namespace.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                use heck::ToPascalCase;
                self.name.to_pascal_case()
            })
    }

    /// Get the NuGet package ID.
    ///
    /// Prefers the explicit `[csharp] package_id` override; otherwise falls back to
    /// [`Self::csharp_namespace`] (most projects publish under `packageId ==
    /// namespace`). The single source of truth for this coordinate, so
    /// `validate_dotnet_coordinates` and the cross-crate NuGet collision check in
    /// [`crate::core::config::new_config::NewAlefConfig::resolve`] can never disagree about it.
    pub fn nuget_package_id(&self) -> String {
        self.csharp
            .as_ref()
            .and_then(|config| config.package_id.clone())
            .unwrap_or_else(|| self.csharp_namespace())
    }
}

#[cfg(test)]
mod tests {
    use crate::core::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn with_repo(name: &str, repo: &str) -> super::super::ResolvedCrateConfig {
        resolved_one(&format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "{name}"
sources = ["src/lib.rs"]

[crates.scaffold]
repository = "{repo}"
"#
        ))
    }

    #[test]
    fn go_module_derives_from_repo() {
        let r = with_repo("my-lib", "https://github.com/foo/my-lib");
        assert_eq!(r.go_module(), "github.com/foo/my-lib");
    }

    #[test]
    fn go_module_explicit_wins_over_repo() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
repository = "https://github.com/foo/my-lib"

[crates.go]
module = "custom.example.com/my-lib"
"#,
        );
        assert_eq!(r.go_module(), "custom.example.com/my-lib");
    }

    #[test]
    fn github_repo_uses_package_metadata_repository() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.package_metadata]
repository = "https://gitlab.example.invalid/acme/my-lib"
"#,
        );
        assert_eq!(
            r.try_github_repo().as_deref(),
            Ok("https://gitlab.example.invalid/acme/my-lib")
        );
    }

    #[test]
    fn java_package_derives_from_repo() {
        let r = with_repo("my-lib", "https://github.com/foo-org/my-lib");
        assert_eq!(r.java_package(), "com.github.foo_org");
    }

    #[test]
    fn java_package_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"
"#,
        );
        assert_eq!(r.java_package(), "dev.sample_crate");
    }

    #[test]
    fn kotlin_package_falls_back_to_placeholder() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.kotlin_package(), "unconfigured.alef");
    }

    #[test]
    fn csharp_namespace_derives_pascal_case() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.csharp_namespace(), "MyLib");
    }

    #[test]
    fn java_group_id_equals_package() {
        let r = with_repo("my-lib", "https://github.com/foo-org/my-lib");
        assert_eq!(r.java_group_id(), r.java_package());
    }

    // -----------------------------------------------------------------------------------------
    // Property coverage: no proptest/quickcheck dependency exists in this crate, so this drives a
    // deliberately adversarial, table-driven input list -- rather than a randomized generator --
    // through `csharp_namespace()`'s unset-override default (`self.name.to_pascal_case()`,
    // exercised here via `heck::ToPascalCase` directly rather than round-tripping through
    // `NewAlefConfig::resolve()`, so the property holds independent of whatever the crate-name
    // path-safety gate in `new_config` currently allows or rejects as a `name` value -- this is
    // deliberately not relying on that gate as the only line of defense, matching
    // `validate_path_segment_field`'s own "does not rely on that incidental protection"
    // reasoning). For every input, the backend's own `namespace.replace('.', "/")` step (see
    // `src/backends/csharp/gen_bindings/mod.rs`, `service_api.rs`) must not turn the pascal-cased
    // output into an absolute path or a `..` path component, and the raw output must never carry
    // a path separator or NUL byte. Confirmed empirically against the real `heck` 0.5.0
    // dependency (not just reasoned from its docs) before writing this assertion: every fragment
    // below strips to only alphanumeric characters (heck's word-boundary algorithm treats `.`,
    // `-`, `_`, `/`, `\`, NUL, and whitespace purely as separators, never part of the output), so
    // this test is expected to pass on current code -- it is defense-in-depth coverage for a
    // property already true, not a fix for a discovered break.
    #[test]
    fn csharp_namespace_default_is_never_path_hazardous_for_any_crate_name() {
        use heck::ToPascalCase;

        const ADVERSARIAL_NAMES: &[&str] = &[
            "..",
            ".",
            "...",
            "....",
            "a..b",
            ".leading",
            "trailing.",
            "..leading-trailing..",
            "back\\slash",
            "back\\\\slash",
            "with\0null",
            "unicode-\u{65e5}\u{672c}\u{8a9e}-\u{4f60}\u{597d}",
            "  spaces  ",
            "-",
            "--",
            "___",
            "UPPER-CASE",
            "MixedCase.Name",
            "a.b.c.d.e",
            "%2e%2e",
            "..%2f..",
            "a/b",
            "////",
            "...-...-",
            "-.-.-",
            "a-.-b",
            "",
        ];

        for name in ADVERSARIAL_NAMES {
            let namespace = name.to_pascal_case();
            assert!(
                !namespace.contains('/') && !namespace.contains('\\') && !namespace.contains('\0'),
                "csharp_namespace default must never carry a raw path separator or NUL: \
                 name={name:?} namespace={namespace:?}"
            );
            let transformed = namespace.replace('.', "/");
            let transformed_path = std::path::Path::new(&transformed);
            assert!(
                !transformed_path.is_absolute(),
                "csharp_namespace default must not become absolute after the backend's \
                 dot-to-slash transform: name={name:?} namespace={namespace:?} \
                 transformed={transformed:?}"
            );
            assert!(
                !transformed_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
                "csharp_namespace default must not contain a `..` component after the backend's \
                 dot-to-slash transform: name={name:?} namespace={namespace:?} \
                 transformed={transformed:?}"
            );
        }
    }
}
