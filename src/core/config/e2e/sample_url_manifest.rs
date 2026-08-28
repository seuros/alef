//! Manifest-backed `sample_url_template` variables: `[crates.e2e.snippets].sample_url_manifest`.
//!
//! [`super::sample_url_template::SampleUrlTemplate`] resolves its placeholders against a
//! fixture's own `docs.sample_url_vars` -- a fact hand-declared once per fixture. That does not
//! scale for a content-addressed corpus with hundreds of entries, where the fact a template
//! needs (a digest, a bucket key, ...) is produced by a build step outside alef and already
//! lives in a manifest file, keyed by the corpus-relative path of the content itself.
//!
//! [`SampleUrlManifestConfig`] names that manifest file and the single template variable its
//! values populate, generically: alef has no notion of "digest" or any other consumer-specific
//! fact name built in. A project configures `path` (the manifest file, relative to the project
//! root) and `variable` (the placeholder name a manifest value fills in), and the manifest
//! itself supplies the values, keyed by each fixture's own [`super::FixtureDocs::body_file`] --
//! see `crate::e2e::fixture::FixtureDocs`.
//!
//! Resolution never becomes a second parallel resolver: [`merge_manifest_vars`] only produces
//! the `vars` map [`super::sample_url_template::resolve_templated_sample_url`] already accepts,
//! so every caller still goes through that one seam. A fixture's own `docs.sample_url_vars`
//! always wins over a manifest entry supplying the same key -- see [`merge_manifest_vars`]'s doc
//! comment for why.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The `alef.toml` table a project sets to enable manifest-backed template resolution, named
/// here so every diagnostic that mentions it spells it the same way.
pub const SAMPLE_URL_MANIFEST_CONFIG_KEY: &str = "[crates.e2e.snippets].sample_url_manifest";

/// `[crates.e2e.snippets].sample_url_manifest` -- a manifest file and the single
/// `sample_url_template` placeholder its values populate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleUrlManifestConfig {
    /// Manifest file path, relative to the project root (the directory holding `alef.toml`),
    /// mirroring `[crates.e2e.snippets].curated_snippets`'s own path convention.
    pub path: String,
    /// The `sample_url_template` placeholder name a manifest value fills in, e.g. `"digest"`
    /// for a template shaped `"https://cdn.example.org/objects/{digest}"`. Never hard-coded by
    /// alef -- the project names the fact its own template needs.
    pub variable: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidSampleUrlManifest {
    #[error(
        "`{SAMPLE_URL_MANIFEST_CONFIG_KEY}.path` is empty; remove the `sample_url_manifest` table to disable \
         manifest-backed template variables"
    )]
    EmptyPath,
    #[error(
        "`{SAMPLE_URL_MANIFEST_CONFIG_KEY}.variable` is empty; name the template placeholder this manifest's \
         values populate"
    )]
    EmptyVariable,
    #[error("`{SAMPLE_URL_MANIFEST_CONFIG_KEY}.path` names `{path}`, which could not be read: {reason}")]
    Unreadable { path: String, reason: String },
    #[error("`{SAMPLE_URL_MANIFEST_CONFIG_KEY}.path` names `{path}`, which is not a valid manifest: {reason}")]
    Malformed { path: String, reason: String },
}

/// A resolved manifest: a corpus-relative path (matching a fixture's own
/// `docs.body_file`) to the single value that fills in `variable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleUrlManifest {
    variable: String,
    values: BTreeMap<String, String>,
}

impl SampleUrlManifest {
    /// Resolve `config` into a validated, fully-loaded manifest, or `None` when the project
    /// configures none -- the additive case that leaves per-fixture `docs.sample_url_vars` the
    /// whole story. `project_root` is the directory `config.path` is resolved relative to.
    ///
    /// A configured manifest that is missing, unreadable, or malformed fails the run outright,
    /// naming the file and what went wrong -- never a silent fallback to "not configured", which
    /// would look identical to a project that never enabled this at all and hide a real
    /// misconfiguration.
    pub fn resolve(
        config: Option<&SampleUrlManifestConfig>,
        project_root: &Path,
    ) -> Result<Option<Self>, InvalidSampleUrlManifest> {
        let Some(config) = config else {
            return Ok(None);
        };
        let path = config.path.trim();
        if path.is_empty() {
            return Err(InvalidSampleUrlManifest::EmptyPath);
        }
        let variable = config.variable.trim();
        if variable.is_empty() {
            return Err(InvalidSampleUrlManifest::EmptyVariable);
        }
        let full_path = project_root.join(path);
        let content = std::fs::read_to_string(&full_path).map_err(|error| InvalidSampleUrlManifest::Unreadable {
            path: path.to_string(),
            reason: error.to_string(),
        })?;
        let entries: ManifestEntries =
            serde_json::from_str(&content).map_err(|error| InvalidSampleUrlManifest::Malformed {
                path: path.to_string(),
                reason: error.to_string(),
            })?;
        Ok(Some(Self {
            variable: variable.to_string(),
            values: entries.0,
        }))
    }

    /// The single-entry `vars` map this manifest supplies for `body_file` (a fixture's own
    /// `docs.body_file`), or `None` when the manifest carries no entry for it -- the caller's
    /// signal to fall back exactly as an uncovered fixture always has.
    pub fn vars_for(&self, body_file: &str) -> Option<BTreeMap<String, String>> {
        self.values
            .get(body_file)
            .map(|value| BTreeMap::from([(self.variable.clone(), value.clone())]))
    }
}

/// Merge a manifest's facts for `body_file` underneath a fixture's own explicit
/// `docs.sample_url_vars`, producing exactly the `vars` map
/// [`super::sample_url_template::resolve_templated_sample_url`] already accepts -- manifest
/// resolution is additive data preparation, never a second parallel resolver.
///
/// A fixture's own declared var always wins over a manifest entry supplying the same key: the
/// fixture is the more specific, more obviously-intentional source, and letting it win keeps a
/// fixture author able to correct or override one manifest entry locally without touching the
/// manifest itself. `body_file` is `None` for a fixture that declares none (or when no manifest
/// is configured), in which case this reduces to a clone of `fixture_vars`, unchanged from
/// before manifests existed. ~keep
pub fn merge_manifest_vars(
    manifest: Option<&SampleUrlManifest>,
    body_file: Option<&str>,
    fixture_vars: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = body_file
        .and_then(|body_file| manifest.and_then(|manifest| manifest.vars_for(body_file)))
        .unwrap_or_default();
    merged.extend(fixture_vars.iter().map(|(key, value)| (key.clone(), value.clone())));
    merged
}

/// A validated manifest body: a JSON object mapping a corpus-relative path to a single string
/// value. Deserialized through a hand-rolled [`serde::de::Visitor`] rather than
/// `serde_json::Value` first, because parsing into `Value`'s map silently keeps only the last
/// occurrence of a duplicate JSON key -- streaming through `MapAccess` instead lets this type
/// see every occurrence and reject the duplicate outright, per the "validate at a system
/// boundary" requirement a build-generated manifest needs. ~keep
struct ManifestEntries(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for ManifestEntries {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EntriesVisitor;

        impl<'de> serde::de::Visitor<'de> for EntriesVisitor {
            type Value = ManifestEntries;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object mapping a corpus-relative path to a string value")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut entries = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if entries.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(format!("duplicate manifest key `{key}`")));
                    }
                }
                Ok(ManifestEntries(entries))
            }
        }

        deserializer.deserialize_map(EntriesVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(directory: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, content).expect("write manifest fixture");
        path
    }

    fn config(path: &str, variable: &str) -> SampleUrlManifestConfig {
        SampleUrlManifestConfig {
            path: path.to_string(),
            variable: variable.to_string(),
        }
    }

    #[test]
    fn an_unconfigured_manifest_resolves_to_none() {
        assert_eq!(
            SampleUrlManifest::resolve(None, Path::new(".")).expect("no configuration always resolves"),
            None
        );
    }

    #[test]
    fn a_configured_manifest_resolves_the_variable_it_names_for_a_covered_path() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_manifest(
            directory.path(),
            "manifest.json",
            r#"{"pdf/memo.pdf": "9f86d081884c7d659a2feaa0c55ad015"}"#,
        );
        let config = config("manifest.json", "digest");

        let manifest = SampleUrlManifest::resolve(Some(&config), directory.path())
            .expect("valid manifest resolves")
            .expect("a configured value produces a manifest");

        assert_eq!(
            manifest.vars_for("pdf/memo.pdf"),
            Some(BTreeMap::from([(
                "digest".to_string(),
                "9f86d081884c7d659a2feaa0c55ad015".to_string()
            )]))
        );
    }

    /// The fallback case: a path the manifest never mentions must resolve to `None`, the exact
    /// signal callers use to fall back to `sample_base_url` unchanged.
    #[test]
    fn a_path_the_manifest_does_not_cover_resolves_to_none() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_manifest(
            directory.path(),
            "manifest.json",
            r#"{"pdf/memo.pdf": "9f86d081884c7d659a2feaa0c55ad015"}"#,
        );
        let config = config("manifest.json", "digest");

        let manifest = SampleUrlManifest::resolve(Some(&config), directory.path())
            .expect("valid manifest resolves")
            .expect("a configured value produces a manifest");

        assert_eq!(manifest.vars_for("images/logo.png"), None);
    }

    #[test]
    fn a_missing_manifest_file_errors_naming_the_path() {
        let directory = tempfile::tempdir().expect("temp dir");
        let config = config("does-not-exist.json", "digest");

        let error =
            SampleUrlManifest::resolve(Some(&config), directory.path()).expect_err("a missing manifest must fail");

        match error {
            InvalidSampleUrlManifest::Unreadable { path, reason } => {
                assert_eq!(path, "does-not-exist.json", "the error must name the configured path");
                assert!(!reason.is_empty(), "the underlying I/O reason must be preserved");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_that_is_not_a_json_object_is_rejected() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_manifest(directory.path(), "manifest.json", r#"["pdf/memo.pdf"]"#);
        let config = config("manifest.json", "digest");

        let error =
            SampleUrlManifest::resolve(Some(&config), directory.path()).expect_err("a non-object manifest must fail");

        assert!(matches!(error, InvalidSampleUrlManifest::Malformed { path, .. } if path == "manifest.json"));
    }

    #[test]
    fn a_manifest_entry_with_a_non_string_value_is_rejected() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_manifest(
            directory.path(),
            "manifest.json",
            r#"{"pdf/memo.pdf": {"digest": "abc"}}"#,
        );
        let config = config("manifest.json", "digest");

        let error = SampleUrlManifest::resolve(Some(&config), directory.path())
            .expect_err("a non-string manifest value must fail");

        assert!(matches!(error, InvalidSampleUrlManifest::Malformed { path, .. } if path == "manifest.json"));
    }

    /// The validation requirement this exists to enforce: a duplicate JSON key in a
    /// build-generated manifest is a real defect (two builds racing on the same output path,
    /// say) and must fail loudly rather than silently keeping whichever occurrence
    /// `serde_json::Value` happened to keep last.
    #[test]
    fn a_manifest_with_a_duplicate_key_is_rejected() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_manifest(
            directory.path(),
            "manifest.json",
            r#"{"pdf/memo.pdf": "first", "pdf/memo.pdf": "second"}"#,
        );
        let config = config("manifest.json", "digest");

        let error = SampleUrlManifest::resolve(Some(&config), directory.path()).expect_err("a duplicate key must fail");

        assert!(matches!(error, InvalidSampleUrlManifest::Malformed { path, .. } if path == "manifest.json"));
    }

    #[test]
    fn an_empty_path_is_rejected() {
        let config = config("", "digest");
        assert_eq!(
            SampleUrlManifest::resolve(Some(&config), Path::new(".")).expect_err("an empty path cannot resolve"),
            InvalidSampleUrlManifest::EmptyPath
        );
    }

    #[test]
    fn an_empty_variable_name_is_rejected() {
        let config = config("manifest.json", "");
        assert_eq!(
            SampleUrlManifest::resolve(Some(&config), Path::new("."))
                .expect_err("an empty variable name cannot resolve"),
            InvalidSampleUrlManifest::EmptyVariable
        );
    }

    fn manifest_with(entries: &[(&str, &str)], variable: &str) -> SampleUrlManifest {
        SampleUrlManifest {
            variable: variable.to_string(),
            values: entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn merge_falls_back_to_an_empty_map_with_no_manifest_and_no_fixture_vars() {
        assert_eq!(merge_manifest_vars(None, None, &BTreeMap::new()), BTreeMap::new());
    }

    #[test]
    fn merge_uses_the_manifest_value_when_the_fixture_declares_nothing_for_that_key() {
        let manifest = manifest_with(&[("pdf/memo.pdf", "abc123")], "digest");

        let merged = merge_manifest_vars(Some(&manifest), Some("pdf/memo.pdf"), &BTreeMap::new());

        assert_eq!(merged, BTreeMap::from([("digest".to_string(), "abc123".to_string())]));
    }

    /// Precedence, direction one: the fixture's own `docs.sample_url_vars` entry for a key the
    /// manifest also supplies must win.
    #[test]
    fn merge_lets_an_explicit_fixture_var_override_the_manifest_for_the_same_key() {
        let manifest = manifest_with(&[("pdf/memo.pdf", "from-manifest")], "digest");
        let fixture_vars = BTreeMap::from([("digest".to_string(), "from-fixture".to_string())]);

        let merged = merge_manifest_vars(Some(&manifest), Some("pdf/memo.pdf"), &fixture_vars);

        assert_eq!(
            merged,
            BTreeMap::from([("digest".to_string(), "from-fixture".to_string())]),
            "an explicit fixture declaration must win over the manifest for the same key"
        );
    }

    /// Precedence, direction two: a fixture var naming a DIFFERENT key than the manifest supplies
    /// must combine with it rather than replacing it wholesale.
    #[test]
    fn merge_combines_disjoint_manifest_and_fixture_keys() {
        let manifest = manifest_with(&[("pdf/memo.pdf", "abc123")], "digest");
        let fixture_vars = BTreeMap::from([("region".to_string(), "us-east".to_string())]);

        let merged = merge_manifest_vars(Some(&manifest), Some("pdf/memo.pdf"), &fixture_vars);

        assert_eq!(
            merged,
            BTreeMap::from([
                ("digest".to_string(), "abc123".to_string()),
                ("region".to_string(), "us-east".to_string()),
            ])
        );
    }
}
