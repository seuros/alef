//! Fixtures and assertion helpers shared by the `snippet` test modules.

use crate::e2e::fixture::Fixture;

pub(super) fn fixture() -> Fixture {
    Fixture {
        id: "quick_start".into(),
        description: "Quick start".into(),
        input: serde_json::Value::Null,
        ..Fixture::default()
    }
}

pub(super) fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
    body.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
        .trim()
}
