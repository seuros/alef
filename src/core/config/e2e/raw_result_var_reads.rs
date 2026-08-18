//! Structural guard over direct reads of [`CallConfig::result_var`](super::CallConfig::result_var).
//!
//! `result_var` is blank whenever the call was not built from TOML, and blank spliced into a
//! binding emits `val  = Sample.process()` — a binding with no identifier, which no target
//! language parses. Four emitters had each re-derived a local `if is_empty() { "result" }`
//! fallback and the thirty-odd others had not, so one call rendered under two different names
//! depending on which emitter you read.
//!
//! [`CallConfig::effective_result_var`](super::CallConfig::effective_result_var) is now the only
//! place that rule lives, and `tests/e2e_result_var_defaulting.rs` proves every backend in
//! `all_generators` honours it. That test can only reach the code paths its probe fixture
//! reaches, though — a raw read added on a streaming or error-shaped path would pass it. This
//! guard covers the rest: a new direct read fails a test instead of shipping invalid source.
//!
//! Deliberately not shared with the sibling `raw_function_reads` scanner. That field is an
//! override chain whose base is legitimately empty, so its predicate has to exempt
//! `unwrap_or`/`and_then` tails and `Option` methods; `result_var` has neither an override nor an
//! `Option` form, and folding the two predicates together would mean each field's guard exempts
//! shapes that only matter for the other.

/// Every direct read of the raw `result_var` still present in the tree, as
/// `path\ttrimmed source line`.
///
/// **This list may only shrink.** Entries are pinned by source text rather than line number so
/// the list does not churn when unrelated code moves above them.
const KNOWN_RAW_RESULT_VAR_READS: &[&str] = &[
    // The resolver itself.
    "src/core/config/e2e/call.rs\tif self.result_var.trim().is_empty() {",
    "src/core/config/e2e/call.rs\t&self.result_var",
    // Assertions on the stored field, which are how the tests tell "serde populated it" apart
    // from "the accessor papered over a blank".
    "src/core/config/e2e/tests.rs\tassert_eq!(call.result_var, \"captured\");",
    "src/core/config/e2e/tests.rs\tassert_eq!(call.result_var, \"result\");",
    "src/core/config/e2e/tests.rs\tfrom_serde.result_var, \"result\",",
];

/// This file's own path, skipped so the allowlist above does not match itself.
const GUARD_FILE: &str = "src/core/config/e2e/raw_result_var_reads.rs";

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// True when `line` reads a `.result_var` field rather than writing or naming one.
fn reads_the_raw_result_var(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    let bytes = line.as_bytes();
    line.match_indices(".result_var").any(|(index, _)| {
        let tail = &line[index + ".result_var".len()..];
        // `.result_variants` and friends are unrelated fields that merely share a prefix.
        if tail.as_bytes().first().is_some_and(|byte| is_ident_char(*byte)) {
            return false;
        }
        // An assignment writes the field; only reads can emit anything from it.
        let trimmed = tail.trim_start();
        if trimmed.starts_with('=') && !trimmed.starts_with("==") {
            return false;
        }
        // A `.result_var` inside a string literal is generated source or a TOML key.
        if bytes[..index].iter().filter(|byte| **byte == b'"').count() % 2 == 1 {
            return false;
        }
        // The receiver must be an identifier: `call.result_var`, never `foo().result_var`.
        bytes[..index].last().is_some_and(|byte| is_ident_char(*byte))
    })
}

fn collect_rust_files(directory: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(directory).expect("the source tree must be readable");
    for entry in entries {
        let path = entry.expect("a directory entry must be readable").path();
        if path.is_dir() {
            collect_rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn no_new_code_emits_an_identifier_from_the_raw_result_var() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rust_files(&root.join("src"), &mut files);
    assert!(
        files.len() > 100,
        "the source scan found only {} files — the walk is broken, and a guard that examines \
         nothing passes for a healthy tree",
        files.len()
    );

    let mut found: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in &files {
        let relative = path
            .strip_prefix(root)
            .expect("every scanned file lives under the manifest directory")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == GUARD_FILE {
            continue;
        }
        let source = std::fs::read_to_string(path).expect("a Rust source file must be readable");
        for line in source.lines() {
            if reads_the_raw_result_var(line) {
                found.insert(format!("{relative}\t{}", line.trim()));
            }
        }
    }

    let known: std::collections::BTreeSet<String> = KNOWN_RAW_RESULT_VAR_READS
        .iter()
        .map(|entry| (*entry).to_string())
        .collect();
    let added: Vec<&String> = found.difference(&known).collect();
    let removed: Vec<&String> = known.difference(&found).collect();

    assert!(
        added.is_empty(),
        "new direct read(s) of the raw `result_var`. It is blank whenever the call was not built \
         from TOML, so splicing it into a binding emits an identifier-less `val  = ...`. Use \
         `CallConfig::effective_result_var`:\n{added:#?}"
    );
    assert!(
        removed.is_empty(),
        "KNOWN_RAW_RESULT_VAR_READS lists entries that no longer exist — delete them so the list \
         keeps shrinking:\n{removed:#?}"
    );
}

#[test]
fn the_guard_recognises_the_shapes_it_is_meant_to_catch() {
    assert!(reads_the_raw_result_var("    let result_var = &call.result_var;"));
    assert!(reads_the_raw_result_var("            result_var => call.result_var,"));
    assert!(reads_the_raw_result_var(
        "    let result_is_tree = call_config.result_var == \"tree\";"
    ));
    assert!(reads_the_raw_result_var("    if call.result_var.is_empty() {"));

    assert!(
        !reads_the_raw_result_var("        e2e.call.result_var = \"result\".into();"),
        "an assignment writes the field rather than emitting from it"
    );
    assert!(
        !reads_the_raw_result_var("        let variants = &metadata.result_variants;"),
        "an unrelated field that merely shares the prefix is not a read of this one"
    );
    assert!(
        !reads_the_raw_result_var("        let field = \"input.result_var\";"),
        "a path inside a string literal is not a field read"
    );
    assert!(
        !reads_the_raw_result_var("    // call.result_var must never be read directly"),
        "prose about the field is not a read of it"
    );
    assert!(
        !reads_the_raw_result_var("    let name = call.effective_result_var();"),
        "the resolver is the fix, not an instance of the defect: its name carries no dot \
         immediately before `result_var`, so adopting it must not trip the guard"
    );
}
