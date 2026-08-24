//! The public base URL a *documentation snippet* binds for a `mock_url` /
//! `mock_url_list` argument.
//!
//! A fixture's URL arguments serve two audiences with opposite requirements. The
//! executable e2e suite needs them bound to the per-fixture mock server: a generated test
//! must never reach the network. A published snippet needs them bound to an address a
//! reader can actually copy, paste, and run against -- a mock-server address is useless
//! there, and `crate::e2e::snippets::mock_harness_guard` refuses one outright.
//!
//! Only the documentation side is configurable, and only through this type: the mock side
//! is correct as it stands and is never rewritten. A project points
//! `[crates.e2e.snippets].sample_base_url` at a host that really serves its sample inputs,
//! and every relative fixture path (`"/pdf/report.pdf"`) resolves against the mock server
//! for tests and against that host for docs, from the same fixture, with no per-fixture
//! edit.
//!
//! When a project configures nothing, [`DEFAULT_DOCS_SAMPLE_BASE_URL`] stands in and
//! [`DocsSampleBaseUrl::is_placeholder`] reports that it did, so the snippet run can say so
//! rather than publishing an unrunnable address in silence.

/// The address alef binds when a project configures no `sample_base_url`.
///
/// `example.com` is IANA's reserved documentation domain (RFC 2606 §3): it resolves for
/// nobody and belongs to nobody, so a snippet built on it fails honestly instead of
/// pointing a reader's copy-paste at some third party's live host. It is a placeholder, not
/// a working default -- see [`DocsSampleBaseUrl::is_placeholder`]. ~keep
pub const DEFAULT_DOCS_SAMPLE_BASE_URL: &str = "https://example.com";

/// The `alef.toml` key a project sets to replace [`DEFAULT_DOCS_SAMPLE_BASE_URL`], named
/// here so every diagnostic that mentions it spells it the same way.
pub const SAMPLE_BASE_URL_CONFIG_KEY: &str = "[crates.e2e.snippets].sample_base_url";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidSampleBaseUrl {
    #[error("`{SAMPLE_BASE_URL_CONFIG_KEY}` is empty; remove the key to fall back to `{DEFAULT_DOCS_SAMPLE_BASE_URL}`")]
    Empty,
    #[error(
        "`{SAMPLE_BASE_URL_CONFIG_KEY}` must contain no whitespace, got `{value}`; \
         it is prefixed onto a fixture's relative path to form a URL a reader pastes into a shell"
    )]
    Whitespace { value: String },
    #[error(
        "`{SAMPLE_BASE_URL_CONFIG_KEY}` must be absolute and name a scheme (e.g. \
         `https://samples.example.org`), got `{value}`"
    )]
    Relative { value: String },
}

/// A resolved documentation sample base URL, plus whether it came from configuration or
/// from [`DEFAULT_DOCS_SAMPLE_BASE_URL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocsSampleBaseUrl<'a> {
    base: &'a str,
    configured: bool,
}

impl<'a> DocsSampleBaseUrl<'a> {
    /// Resolve `configured` (the raw `sample_base_url` value, if any) into a base URL.
    ///
    /// A configured value is validated rather than silently repaired: a base that cannot
    /// form a usable URL would otherwise reach published documentation as a broken address,
    /// which is the exact failure this type exists to remove. Only the trailing `/` is
    /// normalized away, so `"https://host/"` and `"https://host"` join identically.
    pub fn resolve(configured: Option<&'a str>) -> Result<Self, InvalidSampleBaseUrl> {
        let Some(value) = configured else {
            return Ok(Self {
                base: DEFAULT_DOCS_SAMPLE_BASE_URL,
                configured: false,
            });
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(InvalidSampleBaseUrl::Empty);
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err(InvalidSampleBaseUrl::Whitespace {
                value: trimmed.to_string(),
            });
        }
        if !has_url_scheme(trimmed) {
            return Err(InvalidSampleBaseUrl::Relative {
                value: trimmed.to_string(),
            });
        }
        Ok(Self {
            base: trimmed.trim_end_matches('/'),
            configured: true,
        })
    }

    pub fn base(&self) -> &'a str {
        self.base
    }

    /// True when no project configuration supplied this address, so it is
    /// [`DEFAULT_DOCS_SAMPLE_BASE_URL`] and any snippet built on it is illustrative rather
    /// than runnable.
    pub fn is_placeholder(&self) -> bool {
        !self.configured
    }

    /// Resolve a fixture's mock-server-relative path against this base.
    ///
    /// The separator is inserted when the path lacks one, so a fixture that writes
    /// `"seed1"` rather than `"/seed1"` yields `https://host/seed1` instead of the
    /// concatenated `https://hostseed1` a bare `format!` produced.
    pub fn join(&self, path: &str) -> String {
        if path.is_empty() {
            return self.base.to_string();
        }
        if path.starts_with('/') {
            format!("{}{path}", self.base)
        } else {
            format!("{}/{path}", self.base)
        }
    }
}

/// Whether `value` already names an explicit scheme (`"https://..."`, `"s3://..."`, ...)
/// rather than a bare path meant to be resolved against a base.
///
/// Deliberately not restricted to `http`/`https`: a project whose public sample inputs live
/// behind `s3://` or `file://` is describing its own API surface, and alef has no standing
/// to decide which schemes its consumers' documentation may show. ~keep
pub fn has_url_scheme(value: &str) -> bool {
    value.contains("://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_base_falls_back_to_the_reserved_documentation_domain() {
        let resolved = DocsSampleBaseUrl::resolve(None).expect("no configuration always resolves");

        assert_eq!(resolved.base(), "https://example.com");
        assert!(
            resolved.is_placeholder(),
            "the fallback must announce itself as a placeholder so the run can report it"
        );
    }

    #[test]
    fn a_configured_base_is_used_verbatim_and_is_not_a_placeholder() {
        let resolved = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base resolves");

        assert_eq!(resolved.base(), "https://samples.example.org");
        assert!(!resolved.is_placeholder());
    }

    #[test]
    fn a_trailing_slash_is_normalized_away_so_joins_do_not_double_it() {
        let resolved = DocsSampleBaseUrl::resolve(Some("https://samples.example.org/")).expect("valid base resolves");

        assert_eq!(resolved.base(), "https://samples.example.org");
        assert_eq!(resolved.join("/report.pdf"), "https://samples.example.org/report.pdf");
    }

    #[test]
    fn joining_a_path_without_a_leading_slash_still_inserts_the_separator() {
        let resolved = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base resolves");

        assert_eq!(resolved.join("report.pdf"), "https://samples.example.org/report.pdf");
    }

    #[test]
    fn joining_an_empty_path_yields_the_bare_base() {
        let resolved = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base resolves");

        assert_eq!(resolved.join(""), "https://samples.example.org");
    }

    #[test]
    fn an_empty_configured_base_is_rejected_rather_than_falling_back() {
        assert_eq!(
            DocsSampleBaseUrl::resolve(Some("   ")).expect_err("an empty base cannot form a URL"),
            InvalidSampleBaseUrl::Empty
        );
    }

    #[test]
    fn a_base_with_whitespace_is_rejected() {
        let error =
            DocsSampleBaseUrl::resolve(Some("https://samples.example.org/my docs")).expect_err("whitespace is invalid");

        assert!(
            error.to_string().contains("whitespace"),
            "error must name the defect: {error}"
        );
    }

    #[test]
    fn a_scheme_less_base_is_rejected_because_a_reader_cannot_paste_it() {
        let error = DocsSampleBaseUrl::resolve(Some("samples.example.org")).expect_err("a relative base is invalid");

        assert!(
            error.to_string().contains("absolute"),
            "error must name the defect: {error}"
        );
    }

    #[test]
    fn a_non_http_scheme_is_accepted() {
        let resolved = DocsSampleBaseUrl::resolve(Some("s3://sample-bucket")).expect("any scheme is a valid base");

        assert_eq!(resolved.join("/report.pdf"), "s3://sample-bucket/report.pdf");
    }
}
