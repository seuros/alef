//! `cargo sort --check` conformance for the KEY order inside the dependency tables of every
//! `Cargo.toml` alef emits.
//!
//! Consumers gate CI on `cargo sort --check --workspace`. cargo-sort compares the bare
//! dependency NAME: it sorts each dependency table with `toml_edit`'s `Table::sort_values`,
//! i.e. `IndexMap::sort_keys` over `Key: Ord`, and `Key::cmp` compares `Key::get()` -- the
//! decoded text of a single key segment. `tracing.workspace = true` therefore parses into one
//! key `tracing`, and `"clap" = "4"` compares unquoted.
//!
//! Sorting the rendered line text instead disagrees exactly when one dependency name is a
//! prefix of another and the shorter one uses a dotted form, because `-` (0x2D) precedes
//! `.` (0x2E). Those are the only inputs that discriminate a correct comparator from the broken
//! one, so every pair in [`DISCRIMINATING_PAIRS`] is one, and the corpus carries a self-test
//! proving byte-wise line comparison really does order each pair the other way. ~keep

use super::*;
use crate::test_support::cargo_sort_order::assert_dependency_keys_sorted;

/// Every dotted and quoted key form alef can put into a dependency table, with the sort key
/// cargo-sort assigns it. The value side is deliberately varied -- a version literal contains a
/// `.`, and an inline table contains both `=` and `.` -- so a comparator that scans past the
/// key text is caught here rather than in the field. ~keep
const KEY_FORMS: &[(&str, &str)] = &[
    ("plain = \"1\"", "plain"),
    ("hyphen-name = \"1.2.3\"", "hyphen-name"),
    ("under_score = \"1\"", "under_score"),
    (
        "inline = { version = \"1.2\", path = \"../inline\", features = [\"a\"] }",
        "inline",
    ),
    ("suffix.workspace = true", "suffix"),
    ("suffix.version = \"1.2\"", "suffix"),
    ("suffix.path = \"../suffix\"", "suffix"),
    ("suffix.features = [\"a\", \"b\"]", "suffix"),
    ("suffix.default-features = false", "suffix"),
    ("suffix.optional = true", "suffix"),
    ("\"quoted-basic\".workspace = true", "quoted-basic"),
    ("'quoted-literal'.workspace = true", "quoted-literal"),
    ("\"quoted-basic\" = \"1\"", "quoted-basic"),
    ("\"has.dot\" = \"1\"", "has.dot"),
    ("  indented.workspace = true", "indented"),
];

/// `(description, must_come_first, must_come_second)` -- pairs whose by-name order is the
/// REVERSE of their raw-line-text order. A pair of unrelated names such as `serde`/`tokio`
/// would pass with the broken comparator and prove nothing. ~keep
const DISCRIMINATING_PAIRS: &[(&str, &str, &str)] = &[
    (
        "inherited dep vs hyphenated sibling",
        "tracing.workspace = true",
        "tracing-core = \"0.1\"",
    ),
    (
        "both sides inherited",
        "futures.workspace = true",
        "futures-util.workspace = true",
    ),
    (
        "dotted version suffix vs hyphenated sibling",
        "tokio.version = \"1\"",
        "tokio-stream = \"0.1\"",
    ),
    (
        "dotted path suffix vs hyphenated sibling",
        "alpha.path = \"../alpha\"",
        "alpha-beta = { path = \"../alpha-beta\" }",
    ),
    (
        "bare prefix vs hyphenated sibling written as a quoted key",
        "clap = \"4\"",
        "\"clap-verbosity-flag\".workspace = true",
    ),
    (
        "a quoted key containing a dot is one segment, not a dotted key",
        "zeta-one = \"1\"",
        "\"zeta.two\" = \"1\"",
    ),
];

#[test]
fn dependency_sort_key_is_the_bare_name_for_every_emittable_key_form() {
    for (line, expected) in KEY_FORMS {
        assert_eq!(
            dependency_sort_key(line),
            *expected,
            "`{line}` must sort under the bare dependency name `{expected}`"
        );
    }
}

/// Self-test for the corpus: each pair must actually discriminate, i.e. byte-wise line
/// comparison must order it the opposite way. Without this, a pair both comparators agree on
/// could silently join the table and weaken every guard below. ~keep
#[test]
fn every_discriminating_pair_is_ordered_the_other_way_by_raw_line_text() {
    for (description, first, second) in DISCRIMINATING_PAIRS {
        assert!(
            first > second,
            "{description}: `{first}` / `{second}` do not discriminate -- raw line text already \
             orders them the way the by-name rule does, so this pair would pass with the broken \
             comparator"
        );
    }
}

#[test]
fn sort_dependency_lines_orders_by_dependency_name_not_raw_line_text() {
    for (description, first, second) in DISCRIMINATING_PAIRS {
        let mut lines = vec![(*second).to_owned(), (*first).to_owned()];
        sort_dependency_lines(&mut lines);
        assert_eq!(
            lines,
            vec![(*first).to_owned(), (*second).to_owned()],
            "{description}: cargo-sort compares the bare dependency name, so `{first}` must be \
             emitted before `{second}`"
        );
    }
}

/// Ties the corpus to cargo-sort's own comparison machinery: a manifest carrying a pair in
/// by-name order must satisfy the `toml_edit`-backed checker, and the same manifest with only
/// those two lines swapped -- the mutation the consumer ran to isolate this defect -- must fail
/// it.
#[test]
fn cargo_sort_accepts_only_the_by_name_order_for_every_discriminating_pair() {
    for (description, first, second) in DISCRIMINATING_PAIRS {
        let sorted = format!("[package]\nname = \"demo\"\n\n[dependencies]\n{first}\n{second}\n");
        assert_eq!(
            assert_dependency_keys_sorted(description, &sorted),
            2,
            "{description}: both keys must have been compared"
        );

        let swapped = format!("[package]\nname = \"demo\"\n\n[dependencies]\n{second}\n{first}\n");
        let result = std::panic::catch_unwind(|| assert_dependency_keys_sorted(description, &swapped));
        assert!(
            result.is_err(),
            "{description}: cargo-sort must reject `{second}` before `{first}`, otherwise this \
             pair proves nothing:\n{swapped}"
        );
    }
}

/// Sweep: every `Cargo.toml` the scaffold emits, for every language, must already satisfy
/// cargo-sort's key order. Driven off [`Language::ALL`] so a language added later is covered
/// without editing this test; the manifest-emitting languages are asserted afterwards so the
/// sweep can never report success while checking nothing.
#[test]
fn scaffolded_cargo_manifests_have_dependency_keys_in_cargo_sort_order() {
    let api = test_api();
    let config = test_config();
    let mut checked: Vec<Language> = Vec::new();
    let mut compared = 0usize;

    for language in Language::ALL {
        let Ok(files) = scaffold(&api, &config, &[language]) else {
            continue;
        };
        for file in &files {
            if file.path.file_name().is_some_and(|name| name == "Cargo.toml") {
                compared +=
                    assert_dependency_keys_sorted(&format!("{language} -> {}", file.path.display()), &file.content);
                checked.push(language);
            }
        }
    }

    for language in super::cargo_table_order::MANIFEST_EMITTING_LANGUAGES {
        assert!(
            checked.contains(language),
            "{language} emits a binding-crate Cargo.toml but none was checked -- the sweep went \
             vacuous. Checked: {checked:?}"
        );
    }
    assert!(compared > 0, "the sweep compared no dependency keys at all");
}
