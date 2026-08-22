//! Structural guard over direct reads of [`CallConfig::function`](super::CallConfig::function).
//!
//! The base `function` is legitimately empty — a call may name itself only through
//! `overrides.<language>.function` — so code that reads the field directly decides on `""`
//! and fails silently: an adapter lookup matches nothing, a name-prefix classifier matches no
//! prefix, a type name derives from the empty string. One instance of this aborted a release;
//! the rest produced wrong generated code that nothing flagged.
//!
//! Making the field private is not available: roughly a hundred call sites, most of them the
//! legitimate per-language override chains and their tests, set or read it across every
//! backend, so the rename would collide with all of them at once. This guard buys the same
//! property the weaker way — a direct read is still *possible*, but a new one fails a test
//! instead of surfacing as the next outage.

/// Every direct read of the raw base `function` still present in the tree, as
/// `path\ttrimmed source line`.
///
/// **This list may only shrink.** Each entry is a place that still decides something from a
/// field that can be empty. They are pinned by source text rather than line number so the
/// list does not churn when unrelated code moves above them.
const KNOWN_RAW_FUNCTION_READS: &[&str] = &[
    // Fixture-inclusion gate. `overrides.contains_key(language)` tests for the presence of an
    // override *block*, not for a `function` inside it, so a block that sets only `module`
    // admits a nameless call and every generator then emits an empty identifier. Tightening
    // this to `effective_function(language).is_none()` is the right shape but would also
    // exclude C trait-bridge fixtures, whose identity is synthesized downstream by
    // `trait_bridge_derived_c_identity`; that needs a corpus run to confirm.
    "src/e2e/codegen/mod.rs\tif !is_http_fixture && call_config.function.is_empty() && !call_config.overrides.contains_key(language) {",
    // Adapter lookups keyed on the raw base — the same defect fixed in the ruby, php, kotlin,
    // java, dart, csharp and go generators. Left for the agent that owns the elixir backend.
    "src/e2e/codegen/elixir/test_case.rs\t.find(|a| a.name == call_config.function.as_str())",
    "src/e2e/codegen/elixir/snippet.rs\t.find(|value| value.name == call.function)",
    "src/e2e/codegen/elixir/test_case.rs\tfunction_from_override.is_some() || !call_config.function.is_empty()",
    // C's last-resort result-type name. Still derived from the raw base, but no longer invented
    // where it matters: `unresolved_result_type_name` fails generation when the IR was available
    // and the call did not resolve, naming the `result_type` override that fixes it. The base is
    // the right input here — a C override names a prefixed C export, which would pascal-case into
    // a doubled-prefix type.
    "src/e2e/codegen/c.rs\tlet result_type = call.function.to_pascal_case();",
    "src/e2e/codegen/c.rs\tcall = %call.function,",
    // Pre-generation export validation. Widening it to a resolved name risks a hard `bail!` on
    // overrides that name a binding symbol rather than a Rust one (C exports carry a prefix),
    // so an empty base currently costs a diagnostic rather than producing wrong output.
    "src/bin_cli/all_commands.rs\tif call_config.function.is_empty() || call_config.module.is_empty() {",
    "src/bin_cli/all_commands.rs\tlet function_name = &call_config.function;",
    // The resolver itself.
    "src/core/config/e2e/call.rs\tlet base = self.function.trim();",
    // Reads on `CallOverride::function` (an `Option<String>`, never the empty-base hazard) and
    // test assertions on an already-resolved call.
    "src/e2e/codegen/brew/category.rs\tif let Some(override_fn) = &brew_override.function {",
    "src/e2e/snippets/tests/coverage.rs\tassert!(e2e.call.function.is_empty());",
    "src/e2e/codegen/wasm/snippet.rs\tbase `function` nor `overrides.wasm.function` supplies one\",",
    "src/e2e/codegen/java/tests.rs\tassert_eq!(resolved_call.function, \"batchScrape\");",
    "src/e2e/codegen/java/tests.rs\tassert_eq!(resolved_default.function, \"scrape\");",
    "src/e2e/codegen/csharp/tests.rs\tassert_eq!(resolved_call.function, \"BatchScrape\");",
    "src/e2e/codegen/csharp/tests.rs\tassert_eq!(resolved_default.function, \"Scrape\");",
    "src/core/config/e2e/tests.rs\tassert_eq!(resolved.function, \"crawl\");",
    "src/core/config/e2e/tests.rs\tassert_eq!(resolved.function, \"scrape\");",
];

/// This file's own path, skipped so the allowlist above does not match itself.
const GUARD_FILE: &str = "src/core/config/e2e/raw_function_reads.rs";

/// Idioms that make a `.function` read part of a resolution chain rather than a decision.
///
/// `unwrap_or*` / `or_else` / `then_some` / `.or(` are the tails of the per-language override
/// chains, where the base is the documented fallback. `and_then` marks a read of
/// `CallOverride::function`, which is an `Option<String>` and carries no empty-base hazard.
const RESOLUTION_IDIOMS: &[&str] = &["unwrap_or", "or_else", "then_some", ".or(", "and_then"];

/// `Option` methods that only exist on `CallOverride::function`, never on the `String` base.
const OPTION_METHODS: &[&str] = &[".as_ref", ".as_deref", ".is_some", ".is_none"];

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// True when `line` reads a `.function` field in a way that could be the raw base.
fn reads_a_function_field(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    if RESOLUTION_IDIOMS.iter().any(|idiom| line.contains(idiom)) {
        return false;
    }
    let bytes = line.as_bytes();
    line.match_indices(".function").any(|(index, _)| {
        let tail = &line[index + ".function".len()..];
        if tail.as_bytes().first().is_some_and(|byte| is_ident_char(*byte)) {
            return false;
        }
        if OPTION_METHODS.iter().any(|method| tail.starts_with(method)) {
            return false;
        }
        // An assignment writes the field; only reads can decide anything from it.
        let assigned = tail.trim_start().starts_with('=') && !tail.trim_start().starts_with("==");
        if assigned {
            return false;
        }
        // A `.function` inside a string literal is a fixture field path, not a field read.
        if bytes[..index].iter().filter(|byte| **byte == b'"').count() % 2 == 1 {
            return false;
        }
        // The receiver must be an identifier: `x.function`, never `foo().function`.
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
fn no_new_code_decides_anything_from_the_raw_base_function_name() {
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
            if reads_a_function_field(line) {
                found.insert(format!("{relative}\t{}", line.trim()));
            }
        }
    }

    let known: std::collections::BTreeSet<String> = KNOWN_RAW_FUNCTION_READS
        .iter()
        .map(|entry| (*entry).to_string())
        .collect();
    let added: Vec<&String> = found.difference(&known).collect();
    let removed: Vec<&String> = known.difference(&found).collect();

    assert!(
        added.is_empty(),
        "new direct read(s) of the raw base `function`. The base is legitimately empty when a \
         call names itself only per language, so this decides on `\"\"` and fails silently. Use \
         `CallConfig::effective_function` for the symbol a binding emits, or \
         `CallConfig::core_lookup_name` for a key into an adapter/IR table:\n{added:#?}"
    );
    assert!(
        removed.is_empty(),
        "KNOWN_RAW_FUNCTION_READS lists entries that no longer exist — delete them so the list \
         keeps shrinking:\n{removed:#?}"
    );
}

#[test]
fn the_guard_recognises_the_shapes_it_is_meant_to_catch() {
    assert!(reads_a_function_field(
        "    let adapter = adapters.iter().find(|a| a.name == call_config.function.as_str());"
    ));
    assert!(reads_a_function_field("    let fn_name = call.function.as_str();"));
    assert!(reads_a_function_field(
        "    if call_config.function.is_empty() { return; }"
    ));

    assert!(
        !reads_a_function_field("        .unwrap_or_else(|| call_config.function.clone())"),
        "the tail of a per-language override chain is the documented use of the base"
    );
    assert!(
        !reads_a_function_field("        .and_then(|o| o.function.clone())"),
        "`CallOverride::function` is an Option and carries no empty-base hazard"
    );
    assert!(
        !reads_a_function_field("    e2e.call.function = \"convert\".into();"),
        "an assignment writes the field rather than deciding from it"
    );
    assert!(
        !reads_a_function_field("    let field = \"tool_calls[0].function.name\";"),
        "a fixture field path is not a field read"
    );
    assert!(
        !reads_a_function_field("    // reading call.function here would be wrong"),
        "prose about the field is not a read of it"
    );
}
