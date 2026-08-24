//! A published Rust snippet may only read bindings its own body creates.
//!
//! ~keep The call emitter (`test_file::test_function`) decides what the call binds; the docs
//! renderer (`super::render_docs_snippet`) decides what the snippet's tail reads. When those two
//! answers are derived independently they drift, and the drift ships as
//! `E0425: cannot find value ... in this scope` in every affected snippet — a documentation page
//! whose example cannot compile. The last such drift (a derived presentation the emitter could
//! not see) was closed by `presentation::apply_derived_shows`; this pins the invariant itself
//! rather than that one mechanism, so the next generator that reintroduces a private answer is
//! caught here instead of in a consumer's snippet run.
//!
//! Compiling the rendered snippets outright is not possible from a unit test: `snippets::
//! validators::rust` needs a real dependency crate and a `cargo check` with network access, so
//! the invariant is checked structurally on the emitted text instead.

use crate::core::config::NewAlefConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::rust::RustE2eCodegen;
use crate::e2e::fixture::Fixture;
use std::collections::HashSet;

fn ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    (
        vec![TypeDef {
            name: "SampleResult".into(),
            fields: vec![
                FieldDef {
                    name: "content".into(),
                    ty: TypeRef::String,
                    optional: false,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "items".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: false,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        }],
        vec![FunctionDef {
            name: "convert".into(),
            return_type: TypeRef::Named("SampleResult".into()),
            ..FunctionDef::default()
        }],
    )
}

fn snippet_body(call_extra: &str, fixture_json: serde_json::Value, with_ir: bool) -> String {
    let config_text = format!(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_core"
result_var = "result"
args = [{{ name = "html", field = "html", type = "string" }}]
{call_extra}
"#
    );
    let config: NewAlefConfig = toml::from_str(&config_text).expect("config parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e config");
    let resolved = config.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(fixture_json).expect("fixture parses");
    let (type_defs, functions) = if with_ir { ir() } else { (Vec::new(), Vec::new()) };
    RustE2eCodegen
        .render_snippet_body_with_functions(&fixture, &e2e, &resolved, &type_defs, &[], &functions, &[])
        .expect("rust snippet body renders")
}

fn fixture_json(assertions: serde_json::Value, docs: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello</p>"},
        "assertions": assertions,
        "docs": docs,
    })
}

const STREAMING_CALL: &str = "async = true\nreturns_result = true\n\
     [crates.e2e.call.streaming]\nenabled = true\nitem_type = \"String\"";

/// A streaming call binds `stream`, drains it into `chunks`, and never binds `result`. The
/// snippet must present the collection the reader is actually handed. ~keep
#[test]
fn a_streaming_snippet_presents_the_collection_its_body_binds() {
    let body = snippet_body(
        STREAMING_CALL,
        fixture_json(
            serde_json::json!([{"type": "equals", "field": "content", "value": "Hello"}]),
            serde_json::json!({"topic": "smoke", "stem": "sample_fixture"}),
        ),
        true,
    );

    assert!(
        body.contains("println!(\"{:?}\", chunks);"),
        "a streaming snippet shows the drained collection:\n{body}"
    );
    assert!(
        !body.contains("result"),
        "a streaming body binds no `result`, so nothing in it may name one:\n{body}"
    );
}

/// The error branch is result-rooted too: the template's `match {result_var}` reads a binding a
/// streaming body never creates, and the emitter withholds the `.expect` that would have made the
/// stream drainable in the first place. ~keep
#[test]
fn a_streaming_error_snippet_neither_matches_nor_reads_an_unbound_result() {
    let body = snippet_body(
        STREAMING_CALL,
        fixture_json(
            serde_json::json!([{"type": "error", "value": "boom"}]),
            serde_json::json!({"topic": "smoke", "stem": "sample_fixture"}),
        ),
        true,
    );

    assert!(
        !body.contains("match result"),
        "no `match` may be rendered against a binding the body never creates:\n{body}"
    );
    assert!(
        body.contains("let stream = convert(html).await.expect(\"call failed\");"),
        "the stream must be unwrapped before it is drained:\n{body}"
    );
}

/// Rendered across every call/fixture shape this generator distinguishes, no snippet may read a
/// name it never bound. ~keep
#[test]
fn no_rendered_snippet_reads_a_binding_it_never_created() {
    let calls = [
        ("plain", ""),
        ("returns_result", "returns_result = true"),
        ("async", "async = true"),
        ("async_result", "async = true\nreturns_result = true"),
        ("returns_void", "returns_void = true"),
        ("result_is_simple", "result_is_simple = true"),
        ("result_is_vec", "result_is_vec = true"),
        ("result_is_option", "result_is_option = true"),
        ("streaming", STREAMING_CALL),
        ("result_fields", "returns_result = true\nresult_fields = [\"content\"]"),
    ];
    let assertion_sets = [
        (
            "declared_field",
            serde_json::json!([{"type": "equals", "field": "content", "value": "Hello"}]),
        ),
        (
            "assertion_namespace",
            serde_json::json!([{"type": "equals", "field": "transport.header_sent", "value": true}]),
        ),
        ("not_error", serde_json::json!([{"type": "not_error"}])),
        ("error", serde_json::json!([{"type": "error", "value": "boom"}])),
        ("none", serde_json::json!([])),
        (
            "streaming_virtual_field",
            serde_json::json!([{"type": "count_min", "field": "chunks", "value": 2}]),
        ),
    ];
    let docs_sets = [
        (
            "derived",
            serde_json::json!({"topic": "smoke", "stem": "sample_fixture"}),
        ),
        (
            "authored_shows",
            serde_json::json!({"topic": "smoke", "stem": "sample_fixture", "shows": ["content"]}),
        ),
        (
            "authored_iterate",
            serde_json::json!({
                "topic": "smoke",
                "stem": "sample_fixture",
                "presentation": {"operations": [
                    {"op": "iterate", "path": "items", "item": "item", "fields": ["item"]}
                ]}
            }),
        ),
    ];

    let mut unbound = Vec::new();
    for (call_name, call_extra) in &calls {
        for (assertion_name, assertions) in &assertion_sets {
            for (docs_name, docs) in &docs_sets {
                for with_ir in [false, true] {
                    let body = snippet_body(call_extra, fixture_json(assertions.clone(), docs.clone()), with_ir);
                    let names = unbound_names(&body);
                    if !names.is_empty() {
                        unbound.push(format!(
                            "call={call_name} assertions={assertion_name} docs={docs_name} ir={with_ir} \
                             reads unbound {names:?}\n{body}"
                        ));
                    }
                }
            }
        }
    }

    assert!(
        unbound.is_empty(),
        "{} snippet(s) read a binding they never created:\n\n{}",
        unbound.len(),
        unbound.join("\n----\n")
    );
}

/// The scanner itself must be able to fail: a body that reads a name nothing bound has to be
/// reported, or every assertion above is vacuous. ~keep
#[test]
fn the_scanner_reports_a_name_no_statement_binds() {
    let body = "use example_core::convert;\n\nfn main() {\n    let html = r#\"<p>Hello</p>\"#;\n    \
                let _ = convert(html);\n    println!(\"{:?}\", result.content);\n}\n";

    assert_eq!(unbound_names(body), vec!["result".to_string()]);
}

/// ...and must stay quiet about the things a compiler resolves elsewhere: imported items, the
/// callee of a call, field and method segments, and a fixture's own string input. ~keep
#[test]
fn the_scanner_ignores_imports_callees_field_segments_and_literals() {
    let body = "use example_core::convert;\n\nfn main() {\n    let html = r#\"result content\"#;\n    \
                let result = convert(html).expect(\"call failed\");\n    \
                println!(\"{:?}\", result.content);\n    for item in result.items {\n        \
                println!(\"{:?}\", item);\n    }\n}\n";

    assert_eq!(unbound_names(body), Vec::<String>::new());
}

/// Names a rendered body reads as plain variables without ever binding them.
///
/// Deliberately blunt about what it ignores — imported items, callees, field/method segments,
/// macro names, keywords and string-literal contents — because the one thing it must never do is
/// go quiet: an over-eager filter here is how a check that never fails gets written. ~keep
fn unbound_names(body: &str) -> Vec<String> {
    let imports: HashSet<String> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("use "))
        .filter_map(|path| path.trim_end_matches(';').rsplit("::").next())
        .map(str::to_string)
        .collect();
    let mut bound: HashSet<String> = HashSet::new();
    let mut unbound: Vec<String> = Vec::new();
    for line in body.lines() {
        for (name, kind) in identifiers(line) {
            match kind {
                IdentifierKind::Binding => {
                    bound.insert(name);
                }
                IdentifierKind::Read => {
                    if !bound.contains(&name) && !imports.contains(&name) && !unbound.contains(&name) {
                        unbound.push(name);
                    }
                }
            }
        }
    }
    unbound
}

enum IdentifierKind {
    Binding,
    Read,
}

/// Words that resolve to something other than a local binding, so reading one proves nothing.
const RESOLVED_ELSEWHERE: &[&str] = &[
    "as", "async", "await", "else", "fn", "for", "if", "in", "let", "match", "move", "mut", "return", "use", "while",
    "Ok", "Err", "Some", "None", "Vec", "String",
];

/// Split one emitted line into the identifiers it binds and the ones it reads.
fn identifiers(line: &str) -> Vec<(String, IdentifierKind)> {
    let code = strip_string_literals(line);
    let trimmed = code.trim();
    if trimmed.starts_with("use ") || trimmed.starts_with("//") || trimmed.starts_with('#') {
        return Vec::new();
    }
    let characters: Vec<char> = code.chars().collect();
    let mut found = Vec::new();
    let mut index = 0;
    let mut previous_word: Option<String> = None;
    while index < characters.len() {
        if !(characters[index].is_ascii_alphabetic() || characters[index] == '_') {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && (characters[index].is_alphanumeric() || characters[index] == '_') {
            index += 1;
        }
        let word: String = characters[start..index].iter().collect();
        let preceded_by_path_segment = start > 0 && (characters[start - 1] == '.' || characters[start - 1] == ':');
        let is_callee_or_macro = matches!(characters.get(index), Some('(') | Some('!') | Some(':'));
        if word == "_" {
            previous_word = Some(word);
            continue;
        }
        // `Ok(value) =>` / `Err(error) =>` bind a match arm's payload; `|item|` binds a closure
        // parameter. Neither is a read, and both are in scope for the rest of their expression. ~keep
        let binds_pattern_payload = start > 0
            && ((characters[start - 1] == '('
                && matches!(previous_word.as_deref(), Some("Ok") | Some("Err") | Some("Some")))
                || characters[start - 1] == '|');
        if binds_pattern_payload || matches!(previous_word.as_deref(), Some("let") | Some("mut") | Some("for")) {
            found.push((word.clone(), IdentifierKind::Binding));
        } else if !preceded_by_path_segment && !is_callee_or_macro && !RESOLVED_ELSEWHERE.contains(&word.as_str()) {
            found.push((word.clone(), IdentifierKind::Read));
        }
        previous_word = Some(word);
    }
    found
}

/// Blank out `"..."` and `r#"..."#` contents so a fixture's own input text is never mistaken for
/// code. Mirrors the raw-string handling in [`super::find_function_end`], which exists for the
/// same reason. ~keep
fn strip_string_literals(line: &str) -> String {
    let characters: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == 'r'
            && characters
                .get(index + 1)
                .is_some_and(|next| *next == '"' || *next == '#')
        {
            let mut hashes = 0;
            while characters.get(index + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            if characters.get(index + 1 + hashes) == Some(&'"') {
                index += 2 + hashes;
                let closing: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
                let rest: String = characters[index..].iter().collect();
                index += rest.find(&closing).map_or(rest.len(), |at| at + closing.len());
                continue;
            }
        }
        if characters[index] == '"' {
            index += 1;
            while index < characters.len() && characters[index] != '"' {
                index += if characters[index] == '\\' { 2 } else { 1 };
            }
            index += 1;
            continue;
        }
        out.push(characters[index]);
        index += 1;
    }
    out
}
