//! Carry `#[cfg(...)]` gates from `lib.rs` free functions into the
//! flutter_rust_bridge-generated `frb_generated.rs` wire dispatch.
//!
//! flutter_rust_bridge has no concept of Cargo features: its `#[frb]` macro
//! expansion runs exactly once, against whichever feature set was active for
//! that codegen run (the crate's `default` features), and bakes every
//! function it saw — gated or not — into `frb_generated.rs` unconditionally.
//! That includes a numeric `func_id => wire__crate__<name>_impl(...)`
//! dispatch arm and the `fn wire__crate__<name>_impl(...)` wrapper that calls
//! straight into `crate::<name>(...)`.
//!
//! `frb_generated.rs` is generated once and committed; the *same* file is
//! then compiled against every platform's feature subset (for example
//! Android's `--no-default-features --features android-target`). When that
//! subset excludes a feature that gates a function in `lib.rs`, the wire
//! wrapper still calls it unconditionally, which fails to compile with
//! `E0425: cannot find function ... in the crate root`.
//!
//! [`cfg_gated_free_functions`] scans `lib.rs` for top-level (`col 0`)
//! `#[cfg(...)]`-gated `pub fn` / `pub async fn` items and returns their
//! `(name, gate)` pairs, where `gate` is the exact, possibly multi-line
//! `#[cfg(...)]` attribute text. [`inject_frb_cfg_gates`] then adds a copy of
//! that same gate directly above both the `fn wire__crate__<name>_impl(...)`
//! wrapper and its dispatch-table arm in `frb_generated.rs`.
//!
//! ## Wire-ID stability
//!
//! `frb_generated.rs`'s dispatch table is a `match func_id { <literal> =>
//! ..., ... }` — each arm is keyed by an explicit numeric literal assigned
//! once, when the (single, committed) `frb_generated.rs` was generated, not
//! by its position in the match. Gating an arm out under a reduced feature
//! set removes that one arm; it never renumbers any sibling arm, because
//! sibling arms don't derive their ID from position. So every function that
//! *does* survive a given feature subset keeps the exact wire ID it was
//! assigned at generation time, and Dart-side code compiled against that
//! same `frb_generated.rs` — the file is never regenerated per platform —
//! always dispatches to the right function.
//!
//! This module only recognizes top-level free functions. `#[cfg(...)]` gates
//! on `impl` blocks (opaque-type methods, e.g. `#[cfg(feature = "presets")]
//! impl MetaSchema { ... }`) are not covered; extending this rewrite to that
//! shape needs a matching `wire__crate__<Type>_<method>_impl` lookup and is
//! left for when a gated method actually triggers this failure mode.

/// Scan `lib_rs_source` for top-level `#[cfg(...)]`-gated `pub fn` /
/// `pub async fn` free functions and return their `(name, gate)` pairs.
///
/// `gate` is the exact attribute text, including the `#[cfg(` / `)]`
/// delimiters, exactly as written in `lib.rs` (multi-line `cfg(any(...))`
/// predicates are preserved verbatim). Only functions declared at column 0
/// are considered — gates on fields, enum variants, or `impl` blocks (all
/// indented) are ignored, since they don't correspond to a free-function wire
/// wrapper in `frb_generated.rs`.
pub fn cfg_gated_free_functions(lib_rs_source: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = lib_rs_source.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("#[cfg(") {
            if let Some(end) = find_attribute_end(&lines, i) {
                if let Some(name) = lines.get(end + 1).and_then(|l| free_pub_fn_name(l)) {
                    result.push((name, lines[i..=end].join("\n")));
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }

    result
}

/// Starting at `start` (a line beginning with `#[cfg(`), find the index of
/// the line where the attribute's parens balance out and the line ends with
/// `)]`. Returns `None` if the parens never balance before the source ends.
fn find_attribute_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut depth = paren_delta(lines[start]);
    let mut end = start;
    while depth > 0 {
        end += 1;
        let line = *lines.get(end)?;
        depth += paren_delta(line);
    }
    (depth == 0 && lines[end].trim_end().ends_with(")]")).then_some(end)
}

fn paren_delta(line: &str) -> i32 {
    line.matches('(').count() as i32 - line.matches(')').count() as i32
}

/// Returns the function name if `line` is a top-level (`col 0`) `pub fn` or
/// `pub async fn` declaration.
fn free_pub_fn_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub async fn ").or_else(|| line.strip_prefix("pub fn "))?;
    let name_end = rest.find('(')?;
    Some(rest[..name_end].to_string())
}

/// Add a copy of each `(name, gate)` pair's `#[cfg(...)]` gate directly above
/// the matching `fn wire__crate__<name>_impl(...)` definition and its
/// `<id> => wire__crate__<name>_impl(...)` dispatch-table arm in
/// `frb_generated_source`.
///
/// Idempotent: running this twice on the same input does not duplicate a gate
/// that is already present immediately above a matching line.
pub fn inject_frb_cfg_gates(frb_generated_source: &str, cfg_gated_fns: &[(String, String)]) -> String {
    if cfg_gated_fns.is_empty() {
        return frb_generated_source.to_string();
    }

    let lines: Vec<&str> = frb_generated_source.lines().collect();
    let mut result = String::with_capacity(frb_generated_source.len());

    for (idx, line) in lines.iter().enumerate() {
        if let Some(gate) = gate_for_wire_line(line, cfg_gated_fns) {
            if !already_gated(&lines, idx, gate) {
                let indent = leading_whitespace(line);
                for gate_line in gate.lines() {
                    result.push_str(&indent);
                    result.push_str(gate_line);
                    result.push('\n');
                }
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if !frb_generated_source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Combines [`cfg_gated_free_functions`] and [`inject_frb_cfg_gates`]: read
/// the gates straight out of the already-generated `lib.rs` and carry them
/// into `frb_generated.rs`.
pub fn carry_lib_rs_cfg_gates_into_frb_generated(lib_rs_source: &str, frb_generated_source: &str) -> String {
    let gated = cfg_gated_free_functions(lib_rs_source);
    inject_frb_cfg_gates(frb_generated_source, &gated)
}

fn leading_whitespace(line: &str) -> String {
    line.chars().take_while(|c| c.is_whitespace()).collect()
}

fn gate_for_wire_line<'a>(line: &str, cfg_gated_fns: &'a [(String, String)]) -> Option<&'a str> {
    let trimmed = line.trim_start();
    cfg_gated_fns.iter().find_map(|(name, gate)| {
        let def = format!("fn wire__crate__{name}_impl(");
        let arm = format!("=> wire__crate__{name}_impl(");
        (trimmed.starts_with(&def) || trimmed.contains(&arm)).then_some(gate.as_str())
    })
}

fn already_gated(lines: &[&str], idx: usize, gate: &str) -> bool {
    let gate_last_line = gate.lines().last().unwrap_or(gate).trim();
    idx > 0 && lines[idx - 1].trim() == gate_last_line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_gated_free_functions_finds_single_line_gate() {
        let lib_rs =
            "#[cfg(feature = \"transcription\")]\npub fn build_decoder_prompt_tokens(x: i64) -> i64 {\n    x\n}\n";
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(
            found,
            vec![(
                "build_decoder_prompt_tokens".to_string(),
                "#[cfg(feature = \"transcription\")]".to_string()
            )]
        );
    }

    #[test]
    fn cfg_gated_free_functions_finds_async_fn() {
        let lib_rs = "#[cfg(feature = \"url-ingestion\")]\npub async fn map_url(uri: String) -> String {\n    uri\n}\n";
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(found, vec![("map_url".to_string(), "#[cfg(feature = \"url-ingestion\")]".to_string())]);
    }

    #[test]
    fn cfg_gated_free_functions_preserves_multiline_predicate() {
        let lib_rs = concat!(
            "#[cfg(any(\n",
            "    any(feature = \"late-interaction-presets\", feature = \"late-interaction\"),\n",
            "    feature = \"late-interaction-presets\"\n",
            "))]\n",
            "pub fn max_sim_score(a: i64, b: i64) -> f64 {\n",
            "    0.0\n",
            "}\n",
        );
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "max_sim_score");
        assert!(found[0].1.starts_with("#[cfg(any(\n"));
        assert!(found[0].1.ends_with("))]"));
    }

    #[test]
    fn cfg_gated_free_functions_ignores_indented_gates() {
        let lib_rs = concat!(
            "#[cfg(feature = \"presets\")]\n",
            "impl MetaSchema {\n",
            "    #[cfg(feature = \"presets\")]\n",
            "    pub fn compile(json: String) -> MetaSchema {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
        );
        // The impl-level gate at column 0 is followed by `impl MetaSchema {`, not a
        // `pub fn`, so it must not be picked up; the indented method-level gate is
        // not at column 0 at all.
        assert!(cfg_gated_free_functions(lib_rs).is_empty());
    }

    #[test]
    fn cfg_gated_free_functions_ignores_ungated_functions() {
        let lib_rs = "pub fn always_present() -> i32 {\n    1\n}\n";
        assert!(cfg_gated_free_functions(lib_rs).is_empty());
    }

    #[test]
    fn inject_frb_cfg_gates_gates_definition_and_dispatch_arm() {
        let frb_generated = concat!(
            "fn wire__crate__build_decoder_prompt_tokens_impl(\n",
            "    port_: i32,\n",
            ") {\n",
            "}\n",
            "fn dispatch(func_id: i32) {\n",
            "    match func_id {\n",
            "        28 => wire__crate__build_decoder_prompt_tokens_impl(port, ptr, rust_vec_len, data_len),\n",
            "        29 => wire__crate__classify_chunks_impl(port, ptr, rust_vec_len, data_len),\n",
            "    }\n",
            "}\n",
        );
        let gated = vec![(
            "build_decoder_prompt_tokens".to_string(),
            "#[cfg(feature = \"transcription\")]".to_string(),
        )];

        let result = inject_frb_cfg_gates(frb_generated, &gated);

        assert!(
            result.contains("#[cfg(feature = \"transcription\")]\nfn wire__crate__build_decoder_prompt_tokens_impl("),
            "missing gate above the wire wrapper definition: {result}"
        );
        let expected_arm_gate = "        #[cfg(feature = \"transcription\")]\n        \
            28 => wire__crate__build_decoder_prompt_tokens_impl(";
        assert!(
            result.contains(expected_arm_gate),
            "missing gate above the dispatch arm, or wrong indentation: {result}"
        );
        assert!(
            !result.contains("#[cfg(feature = \"transcription\")]\n        29 =>"),
            "unrelated dispatch arm must not be gated: {result}"
        );
    }

    #[test]
    fn inject_frb_cfg_gates_is_idempotent() {
        let frb_generated = concat!(
            "fn wire__crate__timestamp_token_to_ms_impl(\n",
            "    port_: i32,\n",
            ") {\n",
            "}\n",
        );
        let gated = vec![("timestamp_token_to_ms".to_string(), "#[cfg(feature = \"transcription\")]".to_string())];

        let once = inject_frb_cfg_gates(frb_generated, &gated);
        let twice = inject_frb_cfg_gates(&once, &gated);

        assert_eq!(once, twice);
        assert_eq!(once.matches("#[cfg(feature = \"transcription\")]").count(), 1);
    }

    #[test]
    fn inject_frb_cfg_gates_noop_when_no_gated_functions() {
        let frb_generated = "fn wire__crate__always_present_impl() {}\n";
        assert_eq!(inject_frb_cfg_gates(frb_generated, &[]), frb_generated);
    }

    #[test]
    fn carry_lib_rs_cfg_gates_into_frb_generated_end_to_end() {
        let lib_rs = "#[cfg(feature = \"transcription\")]\npub fn timestamp_token_to_ms(id: i64) -> i64 {\n    id\n}\n";
        let frb_generated = concat!(
            "fn wire__crate__timestamp_token_to_ms_impl(port_: i32) {}\n",
            "fn dispatch(func_id: i32) {\n",
            "    match func_id {\n",
            "        287 => wire__crate__timestamp_token_to_ms_impl(port, ptr, rust_vec_len, data_len),\n",
            "    }\n",
            "}\n",
        );

        let result = carry_lib_rs_cfg_gates_into_frb_generated(lib_rs, frb_generated);

        assert!(result.contains("#[cfg(feature = \"transcription\")]\nfn wire__crate__timestamp_token_to_ms_impl("));
        assert!(result.contains("        #[cfg(feature = \"transcription\")]\n        287 =>"));
    }
}
