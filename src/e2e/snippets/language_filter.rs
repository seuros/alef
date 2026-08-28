//! Restricts which per-language snippet files a `--lang`-filtered run rewrites.
//!
//! The snippet stage produces two structurally different kinds of output, and only one of them
//! may be narrowed by `--lang`:
//!
//! - **Per-language files**, at `<output>/<language-slug>/<topic>/<stem>.md`. Every rendered
//!   snippet belongs to exactly one target language, so a run scoped to one language has no
//!   business rewriting another language's files.
//! - **Shared artifacts**, which describe the whole tree at once: the coverage ledger
//!   `.alef-snippet-coverage.json` at the output root (the only record of which path alef
//!   personally wrote for which cell — see `coverage::orphaned_paths` and
//!   `ownership::is_ledger_owned_snippet_path`), and the language-independent `docs-only/`
//!   renders. Narrowing those would rewrite a whole-tree record as though one language were the
//!   whole tree, dropping every other language's ownership entry and permanently orphaning its
//!   files.
//!
//! So the split is: the report is always computed over the full configured language set, keeping
//! every shared artifact byte-identical to what an unfiltered run would write, and only the
//! per-language files it produced are narrowed here, on the way into the write batch.

use super::GeneratedSnippet;
use crate::e2e::fixture::canonical_language;
use std::collections::BTreeSet;

/// Keep only the snippets whose target language the CLI's `--lang` filter selected.
///
/// `selected` is the raw `--lang` list as typed, so it is compared through
/// [`canonical_language`] rather than by string equality: `--lang ffi` and a snippet rendered
/// under the `c` generator are the same target, and a bare `==` would silently drop it.
///
/// `None` means no filter was given and every snippet is kept — the unfiltered run must be
/// unchanged.
pub fn retain_selected_languages(
    snippets: Vec<GeneratedSnippet>,
    selected: Option<&[String]>,
) -> Vec<GeneratedSnippet> {
    let Some(selected) = selected else {
        return snippets;
    };
    let selected: BTreeSet<&str> = selected.iter().map(|language| canonical_language(language)).collect();
    snippets
        .into_iter()
        .filter(|snippet| selected.contains(canonical_language(&snippet.language)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use crate::e2e::fixture::SideEffectClass;

    fn snippet(language: &str) -> GeneratedSnippet {
        GeneratedSnippet {
            file: GeneratedFile {
                path: std::path::PathBuf::from(format!("docs/snippets/{language}/api/example.md")),
                content: String::new(),
                generated_header: false,
            },
            fixture_id: "example".into(),
            fixture_source: "fixtures/example.json".into(),
            language: language.into(),
            requirements: Vec::new(),
            side_effects: SideEffectClass::Safe,
        }
    }

    fn languages(snippets: &[GeneratedSnippet]) -> Vec<&str> {
        snippets.iter().map(|snippet| snippet.language.as_str()).collect()
    }

    #[test]
    fn no_filter_keeps_every_language() {
        let kept = retain_selected_languages(vec![snippet("rust"), snippet("python"), snippet("node")], None);

        assert_eq!(languages(&kept), vec!["rust", "python", "node"]);
    }

    #[test]
    fn a_filter_keeps_only_the_named_languages() {
        let kept = retain_selected_languages(
            vec![snippet("rust"), snippet("python"), snippet("node")],
            Some(&["rust".to_string(), "node".to_string()]),
        );

        assert_eq!(languages(&kept), vec!["rust", "node"]);
    }

    #[test]
    fn an_alias_spelling_still_selects_its_canonical_target() {
        let kept = retain_selected_languages(
            vec![snippet("c"), snippet("rust"), snippet("python")],
            Some(&["ffi".to_string(), "core".to_string()]),
        );

        assert_eq!(languages(&kept), vec!["c", "rust"]);
    }

    #[test]
    fn an_empty_filter_keeps_nothing() {
        let kept = retain_selected_languages(vec![snippet("rust"), snippet("python")], Some(&[]));

        assert!(kept.is_empty(), "an explicit empty --lang list selects no language");
    }
}
