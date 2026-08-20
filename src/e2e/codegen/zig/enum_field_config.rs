//! Merges the hand-maintained enum-field config sources into one set for `FieldResolver::
//! with_enum_fields`.
//!
//! `field_resolver.is_enum` consults this config-derived set first and only then the
//! IR-derived classification (`with_ir_enum_map`), so an explicit config entry always wins.
//! Mirrors the merge the gleam e2e generator performs for the same purpose.

use std::collections::HashSet;

use crate::core::config::e2e::CallOverride;
use crate::e2e::config::{CallConfig, E2eConfig};

/// The union of the call-level `fields_enum` set and the per-language `[overrides.zig]
/// enum_fields` / `assert_enum_fields` config keys.
pub(super) fn effective_enum_fields(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    call_overrides: Option<&CallOverride>,
) -> HashSet<String> {
    let mut fields: HashSet<String> = e2e_config.effective_fields_enum(call_config).clone();
    if let Some(overrides) = call_overrides {
        fields.extend(overrides.enum_fields.keys().cloned());
        fields.extend(overrides.assert_enum_fields.keys().cloned());
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_call_level_and_override_enum_fields() {
        let e2e_config = E2eConfig {
            fields_enum: ["shared".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let call_config = CallConfig::default();
        let overrides = CallOverride {
            enum_fields: [("kind".to_string(), "Kind".to_string())].into_iter().collect(),
            assert_enum_fields: [("status".to_string(), "Status".to_string())].into_iter().collect(),
            ..CallOverride::default()
        };

        let merged = effective_enum_fields(&e2e_config, &call_config, Some(&overrides));

        assert_eq!(
            merged,
            ["shared".to_string(), "kind".to_string(), "status".to_string()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn no_overrides_keeps_only_the_call_level_set() {
        let e2e_config = E2eConfig {
            fields_enum: ["shared".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let call_config = CallConfig::default();

        let merged = effective_enum_fields(&e2e_config, &call_config, None);

        assert_eq!(merged, ["shared".to_string()].into_iter().collect());
    }
}
