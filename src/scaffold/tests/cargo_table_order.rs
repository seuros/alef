use super::*;
use crate::test_support::cargo_sort_order::assert_canonical_table_order;

/// Languages whose scaffold emits a Rust binding-crate `Cargo.toml` carrying a `[lints.*]`
/// table. Asserted to have actually been exercised below, so the sweep can never report
/// success while silently checking nothing. ~keep
const MANIFEST_EMITTING_LANGUAGES: &[Language] = &[
    Language::Ffi,
    Language::Jni,
    Language::Node,
    Language::Php,
    Language::Python,
    Language::Ruby,
    Language::R,
    Language::Elixir,
];

/// Every `Cargo.toml` the scaffold emits must already satisfy `cargo sort --check`'s table
/// ordering, for every language -- consumers gate CI on `cargo sort --check --workspace`.
///
/// Driven off [`Language::ALL`] rather than a hand-listed set so a language added later is
/// swept automatically. A language whose scaffold needs configuration this default fixture
/// does not supply is skipped, which is why [`MANIFEST_EMITTING_LANGUAGES`] is asserted
/// afterwards: the languages that regressed in the field must provably have been checked.
#[test]
fn scaffolded_cargo_manifests_are_in_cargo_sort_canonical_table_order() {
    let api = test_api();
    let config = test_config();
    let mut checked: Vec<Language> = Vec::new();

    for language in Language::ALL {
        let Ok(files) = scaffold(&api, &config, &[language]) else {
            continue;
        };
        for file in &files {
            if file.path.file_name().is_some_and(|name| name == "Cargo.toml") {
                assert_canonical_table_order(&format!("{language} -> {}", file.path.display()), &file.content);
                checked.push(language);
            }
        }
    }

    for language in MANIFEST_EMITTING_LANGUAGES {
        assert!(
            checked.contains(language),
            "{language} emits a binding-crate Cargo.toml but none was checked -- the sweep went \
             vacuous. Checked: {checked:?}"
        );
    }
}

/// The specific regression: `[lints.clippy]` must land after the dependency tables, not
/// between `[package]` and `[dependencies]`. cargo-sort reports that misordering as
/// "Dependencies for <crate> are not sorted" even though the dependency KEYS are alphabetical,
/// which is what made the original failure so hard to read. ~keep
#[test]
fn scaffolded_manifests_emit_the_lints_table_after_every_dependency_table() {
    let api = test_api();
    let config = test_config();

    for language in MANIFEST_EMITTING_LANGUAGES {
        let files = scaffold(&api, &config, &[*language])
            .unwrap_or_else(|error| panic!("{language} scaffold must succeed for the default fixture: {error}"));
        let manifests: Vec<_> = files
            .iter()
            .filter(|file| file.path.file_name().is_some_and(|name| name == "Cargo.toml"))
            .collect();
        assert!(!manifests.is_empty(), "{language} must emit a Cargo.toml");

        for manifest in manifests {
            let content = &manifest.content;
            let lints_at = content.find("[lints.").unwrap_or_else(|| {
                panic!("{language} manifest must carry a lints table:\n{content}");
            });
            for table in [
                "[dependencies]",
                "[build-dependencies]",
                "[dev-dependencies]",
                "[target.",
            ] {
                if let Some(table_at) = content.find(table) {
                    assert!(
                        lints_at > table_at,
                        "{language} -> {}: `{table}` must precede the lints table:\n{content}",
                        manifest.path.display()
                    );
                }
            }
        }
    }
}
