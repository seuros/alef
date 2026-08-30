/// Sanitize one reverse-DNS package label so it can never carry a raw path separator through to
/// [`derive_reverse_dns_package`]'s output.
///
/// `/` is already excluded by construction (host and org are obtained by splitting the URL on
/// `/`, so neither can contain one), and the java/kotlin/csharp backends' own
/// `.replace('.', "/")` step (see `new_config::validate_package_like_field`) neutralizes a `.` in
/// the source URL. A literal `\` is neither: it is not a URL path delimiter here, so an
/// unusual host or org (e.g. a locally-configured `[scaffold] repository` on a filesystem that
/// tolerates it) could carry one through unchanged, and `\` is a native path separator on
/// Windows — the one platform where `Path::components()` treats it as such. Folding it into `_`
/// alongside the existing hyphen normalization closes that gap without touching the `.`/`/`
/// handling this function already gets right. ~keep
fn sanitize_reverse_dns_label(label: &str) -> String {
    label.replace('-', "_").replace('\\', "_")
}

/// Derive a reverse-DNS package name from a repository URL.
///
/// Recognises `https?://<host>/<org>/<rest>` and produces `<reversed-host>.<org>`,
/// where the host is split into labels and reversed (so `github.com` → `com.github`),
/// the org's hyphens become underscores (Java identifier rules), and the trailing
/// path is ignored. Returns `None` when the URL is missing a host or path segment.
///
/// Examples:
/// - `https://github.com/sample_core-dev/sample_core` → `Some("com.github.sample_core_dev")`
/// - `https://github.com/sample_project-rs/sample_project`     → `Some("com.github.sample_project_rs")`
/// - `https://gitlab.com/foo/bar`                → `Some("com.gitlab.foo")`
/// - `https://example.invalid/x`                 → `Some("invalid.example.x")`
/// - `https://github.com/`                       → `None` (no org segment)
pub fn derive_reverse_dns_package(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let mut parts = after_scheme.split('/').filter(|s| !s.is_empty());
    let host = parts.next()?;
    let org = parts.next()?;

    let host_reversed: Vec<String> = host
        .split('.')
        .filter(|s| !s.is_empty())
        .rev()
        .map(sanitize_reverse_dns_label)
        .collect();
    if host_reversed.is_empty() {
        return None;
    }

    let mut pkg = host_reversed.join(".");
    pkg.push('.');
    pkg.push_str(&sanitize_reverse_dns_label(org));
    Some(pkg)
}

/// Derive a Go module path from a repository URL.
///
/// Strips the `https?://` scheme and any trailing slash. Returns `None` when
/// the URL has no host or no path segment beyond the host.
///
/// Examples:
/// - `https://github.com/sample_core-dev/sample_core` → `Some("github.com/sample_core-dev/sample_core")`
/// - `https://github.com/foo/bar/` → `Some("github.com/foo/bar")`
/// - `https://github.com/` → `None`
pub fn derive_go_module_from_repo(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let trimmed = after_scheme.trim_end_matches('/');
    let mut parts = trimmed.split('/');
    let host = parts.next().filter(|s| !s.is_empty())?;
    let org = parts.next().filter(|s| !s.is_empty())?;
    let repo_segment = parts.next().filter(|s| !s.is_empty());

    let mut module = format!("{host}/{org}");
    if let Some(repo) = repo_segment {
        module.push('/');
        module.push_str(repo);
    }
    Some(module)
}

/// Extract the org segment from a repository URL.
///
/// Recognises `https?://<host>/<org>/<rest>` and returns `<org>` verbatim
/// (no case or punctuation transformation). Returns `None` when the URL is
/// missing a host or org segment.
///
/// Examples:
/// - `https://github.com/sample_core-dev/sample_core` → `Some("sample_core-dev")`
/// - `https://github.com/`                       → `None`
pub fn derive_repo_org(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let mut parts = after_scheme.split('/').filter(|s| !s.is_empty());
    let _host = parts.next()?;
    let org = parts.next()?;
    Some(org.to_string())
}

#[cfg(test)]
mod tests {
    use super::derive_reverse_dns_package;

    #[test]
    fn github_org_with_hyphen_underscores_in_package() {
        assert_eq!(
            derive_reverse_dns_package("https://github.com/sample_crate-dev/sample_crate"),
            Some("com.github.sample_crate_dev".to_string())
        );
    }

    #[test]
    fn other_host_reverses_correctly() {
        assert_eq!(
            derive_reverse_dns_package("https://gitlab.com/foo/bar"),
            Some("com.gitlab.foo".to_string())
        );
    }

    #[test]
    fn missing_org_returns_none() {
        assert_eq!(derive_reverse_dns_package("https://github.com/"), None);
        assert_eq!(derive_reverse_dns_package("https://github.com"), None);
    }

    #[test]
    fn no_scheme_still_parses() {
        assert_eq!(
            derive_reverse_dns_package("github.com/foo/bar"),
            Some("com.github.foo".to_string())
        );
    }

    #[test]
    fn placeholder_url_derives_predictably() {
        assert_eq!(
            derive_reverse_dns_package("https://example.invalid/my-lib"),
            Some("invalid.example.my_lib".to_string())
        );
    }

    // -----------------------------------------------------------------------------------------
    // Property coverage: no proptest/quickcheck dependency exists in this crate (checked
    // `Cargo.toml`/`Cargo.lock`), so this drives a deliberately adversarial, table-driven input
    // list through `derive_reverse_dns_package` rather than a randomized generator. Every
    // fragment below is substituted into both the host and the org position of a repo URL, and
    // for every `Some(pkg)` the java/kotlin backends' own `pkg.replace('.', "/")` step (see
    // `new_config::validate_package_like_field`, `src/backends/java/gen_bindings/mod.rs`) must
    // not produce an absolute path or a `..` path component, and `pkg` itself must never carry a
    // raw path separator. This is the exact property the fix above closes: before it, the
    // `back\slash` / `back\\slash` fragments below left a literal `\` in the returned package
    // (`sanitize_reverse_dns_label` did not exist; only `-` was normalized), which `Path`
    // recognizes as a separator on Windows even though `.replace('.', "/")` never touches it.
    #[test]
    fn derived_package_is_never_path_hazardous_for_adversarial_host_or_org() {
        const ADVERSARIAL_FRAGMENTS: &[&str] = &[
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
        ];

        for fragment in ADVERSARIAL_FRAGMENTS {
            for url in [
                format!("https://example.com/{fragment}"),
                format!("https://{fragment}/acme"),
            ] {
                let Some(pkg) = derive_reverse_dns_package(&url) else {
                    continue;
                };
                assert!(
                    !pkg.contains('/') && !pkg.contains('\\'),
                    "derived package must never carry a raw path separator: url={url:?} pkg={pkg:?}"
                );
                let transformed = pkg.replace('.', "/");
                let transformed_path = std::path::Path::new(&transformed);
                assert!(
                    !transformed_path.is_absolute(),
                    "derived package must not become absolute after the backends' dot-to-slash \
                     transform: url={url:?} pkg={pkg:?} transformed={transformed:?}"
                );
                assert!(
                    !transformed_path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir)),
                    "derived package must not contain a `..` component after the backends' \
                     dot-to-slash transform: url={url:?} pkg={pkg:?} transformed={transformed:?}"
                );
            }
        }
    }
}
