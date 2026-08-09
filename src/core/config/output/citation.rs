use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A person or legal-entity author in a generated `CITATION.cff`. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CitationAuthor {
    /// Person author: family name(s). ~keep
    #[serde(default, alias = "family-names")]
    pub family_names: Option<String>,
    /// Person author: given name(s). ~keep
    #[serde(default, alias = "given-names")]
    pub given_names: Option<String>,
    /// Entity author: organisation or legal-entity name. ~keep
    #[serde(default)]
    pub name: Option<String>,
    /// Optional contact email. ~keep
    #[serde(default)]
    pub email: Option<String>,
    /// Optional ORCID iD URL. ~keep
    #[serde(default)]
    pub orcid: Option<String>,
}

/// Configuration for the Alef-generated `CITATION.cff` file. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CitationConfig {
    /// Software title. ~keep
    pub title: String,
    /// One-paragraph software summary. ~keep
    #[serde(rename = "abstract")]
    pub abstract_: String,
    /// Person and legal-entity authors. ~keep
    pub authors: Vec<CitationAuthor>,
    /// Canonical citation message shown to consumers. ~keep
    #[serde(default = "default_citation_message")]
    pub message: String,
    /// Source-code repository URL. ~keep
    #[serde(rename = "repository-code", alias = "repository_code")]
    pub repository_code: String,
    /// Project landing-page URL. ~keep
    #[serde(default)]
    pub url: Option<String>,
    /// SPDX license identifier. ~keep
    #[serde(default)]
    pub license: Option<String>,
    /// Optional release date in `YYYY-MM-DD` form. ~keep
    #[serde(default, rename = "date-released", alias = "date_released")]
    pub date_released: Option<String>,
    /// Persistent DOI for the cited release. ~keep
    #[serde(default)]
    pub doi: Option<String>,
}

fn default_citation_message() -> String {
    "If you use this software, please cite it using the metadata below.".to_string()
}
