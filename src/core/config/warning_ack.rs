//! The closed, narrow warning-acknowledgement schema surface for `alef.toml`.
//!
//! Task #540's audit reached one design: a consumer may silence a specific warning only by
//! naming its exact identity AND the exact source target it fired for -- never by warning
//! class, never by a path glob. A category that must stay always-actionable (see the runtime
//! engine in `crate::core::warning_ack` for the categories that are deliberately absent here)
//! gets no variant on [`AcknowledgeableWarningCategory`], so `toml::from_str` rejects any
//! attempt to name it before alef ever runs. That is a structural guarantee, not a documented
//! convention a consumer could work around: there is no config surface for the forbidden
//! categories to occupy.
//!
//! This module only defines the schema shape. Matching, staleness detection, and the
//! matched-count report live in [`crate::core::warning_ack`], which is not part of the
//! `alef.toml` schema and so is not derived `JsonSchema`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The closed set of warning categories a consumer may acknowledge anywhere in `alef.toml`.
///
/// Adding a variant here is a deliberate, reviewable decision to make a whole new category
/// acknowledgeable project-wide; it is not the place to loosen an individual producer's
/// matching rules. Each producer that consults [`crate::core::warning_ack::AcknowledgementLedger`]
/// additionally restricts which of these variants it accepts at its own config location (see
/// `AcknowledgementLedger::new`'s `scope` argument), so a category can exist here yet still be
/// rejected everywhere it is not the correct match key for. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AcknowledgeableWarningCategory {
    /// A virtual field path synthesized across a tagged-union/method crossing that a target
    /// language's IR cannot represent directly. Owned by `src/e2e/codegen/presentation.rs`; this
    /// variant exists so that module's warning can be acknowledged once it starts consulting the
    /// ledger, but nothing in this crate wires it in yet.
    VirtualFieldPath,
    /// A documentation snippet publishing IANA's reserved `example.com` domain (RFC 2606 §3)
    /// because the fixture's target language has no configured
    /// `[crates.e2e.snippets].sample_base_url`. See `crate::e2e::snippets::render_body`.
    DocSnippetReservedDomain,
}

impl AcknowledgeableWarningCategory {
    /// The exact string this category serializes as in `alef.toml`, named once so every
    /// diagnostic and every generated example entry spells it identically.
    pub fn config_value(self) -> &'static str {
        match self {
            Self::VirtualFieldPath => "virtual_field_path",
            Self::DocSnippetReservedDomain => "doc_snippet_reserved_domain",
        }
    }
}

impl std::fmt::Display for AcknowledgeableWarningCategory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.config_value())
    }
}

/// One consumer-authored acknowledgement: silence exactly one warning identity for exactly one
/// source target.
///
/// Never a glob and never class-wide by construction -- `identity` and `target` are matched by
/// exact string equality (see `crate::core::warning_ack::AcknowledgementLedger::check`), and
/// there is no field here that could broaden a match to "every target" or "every identity in
/// this category". An entry that stops matching because the underlying warning was fixed (or
/// never fires for that identity/target again) fails the run rather than aging into silent,
/// permanent suppression -- see `AcknowledgementLedger::finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WarningAcknowledgement {
    pub category: AcknowledgeableWarningCategory,
    /// The exact warning identity this entry silences, e.g. a fixture id for
    /// `doc_snippet_reserved_domain` or a field path for `virtual_field_path`. Never a glob or
    /// prefix -- matched by exact string equality only.
    pub identity: String,
    /// The exact source target this entry is scoped to, e.g. a target language such as
    /// `"python"`. An acknowledgement never applies to a target it does not name verbatim.
    pub target: String,
    /// Free-text audit note for the consumer's own record; alef never reads it to decide
    /// anything, only echoes it back when reporting a match.
    #[serde(default)]
    pub reason: Option<String>,
}

impl WarningAcknowledgement {
    /// Render the exact `alef.toml` array-table entry that would acknowledge this
    /// `category`/`identity`/`target` combination.
    ///
    /// Used two ways: an unacknowledged warning's provenance names this as the config a
    /// consumer would add to act on it, and an acknowledged warning's report line names this
    /// as the entry that matched -- so a consumer never has to guess the shape of what they
    /// configured.
    pub fn config_entry_for(category: AcknowledgeableWarningCategory, identity: &str, target: &str) -> String {
        format!("{{ category = \"{category}\", identity = \"{identity}\", target = \"{target}\" }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acknowledgeable_category_round_trips_through_toml() {
        let toml_str = r#"
            category = "doc_snippet_reserved_domain"
            identity = "extract_uri"
            target = "python"
        "#;
        let entry: WarningAcknowledgement = toml::from_str(toml_str).expect("a well-formed entry parses");
        assert_eq!(entry.category, AcknowledgeableWarningCategory::DocSnippetReservedDomain);
        assert_eq!(entry.identity, "extract_uri");
        assert_eq!(entry.target, "python");
        assert_eq!(entry.reason, None);
    }

    /// The structural half of task #540's hard requirement: a category that must stay
    /// always-actionable (display/debug fallbacks, scaffold ownership refusals, missing native
    /// staging artifacts, interrupted partial output, ...) has no variant on
    /// `AcknowledgeableWarningCategory` at all, so naming it is a parse error a consumer cannot
    /// work around by copying an existing entry and editing the string.
    #[test]
    fn a_non_acknowledgeable_category_is_rejected_by_deserialization_not_merely_discouraged() {
        let toml_str = r#"
            category = "scaffold_ownership_refusal"
            identity = "unowned-app"
            target = "go"
        "#;
        let error = toml::from_str::<WarningAcknowledgement>(toml_str)
            .expect_err("a category with no enum variant must fail to parse");
        let message = error.to_string();
        assert!(
            message.contains("scaffold_ownership_refusal") || message.contains("unknown variant"),
            "error must name the offending, non-acknowledgeable category: {message}"
        );
    }

    #[test]
    fn an_unknown_key_is_rejected_not_silently_dropped() {
        let toml_str = r#"
            category = "doc_snippet_reserved_domain"
            identity = "extract_uri"
            target = "python"
            severity = "low"
        "#;
        let error = toml::from_str::<WarningAcknowledgement>(toml_str).expect_err("an unknown key must be rejected");
        assert!(
            error.to_string().contains("severity"),
            "error must name the offending key: {error}"
        );
    }

    #[test]
    fn config_entry_for_renders_the_exact_shape_a_consumer_pastes_back() {
        let rendered = WarningAcknowledgement::config_entry_for(
            AcknowledgeableWarningCategory::DocSnippetReservedDomain,
            "extract_uri",
            "python",
        );
        assert_eq!(
            rendered,
            r#"{ category = "doc_snippet_reserved_domain", identity = "extract_uri", target = "python" }"#
        );
    }
}
