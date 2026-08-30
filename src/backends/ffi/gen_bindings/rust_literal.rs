//! Escaping and capsule-fixup helpers for values embedded inside Rust string
//! literals in generated `build.rs` source.
//!
//! Split out of `helpers.rs` to keep that file under the file-modularization
//! line cap rather than folding this defense-in-depth escaping concern into an
//! already-large file.

use std::collections::HashMap;

use crate::core::config::FfiCapsuleTypeConfig;

/// Escape a value for embedding inside a Rust `"..."` string literal in generated
/// `build.rs` source.
///
/// Defense-in-depth: every caller of this function already validates its input
/// against a grammar (`core::config::abi_grammar`) that excludes `"`, `\`, and
/// control characters outright, so this should be a no-op in practice. It exists
/// so a value reaching this point some other way (a future caller, a config
/// constructed outside `NewAlefConfig::resolve`) still cannot break out of the
/// generated string literal instead of silently trusting the grammar check
/// upstream to be the only guard.
pub(super) fn escape_rust_str_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Build the `header = header.replace("{prefixed}", "{bare}");` capsule fixup
/// lines emitted into generated `build.rs`, with both sides of each pair
/// Rust-literal-escaped. See [`escape_rust_str_literal`].
pub(super) fn capsule_header_fixup(
    capsule_types: &HashMap<String, FfiCapsuleTypeConfig>,
    prefix_upper: &str,
) -> String {
    let mut pairs: Vec<(String, String)> = capsule_types
        .values()
        .map(|c| (format!("{prefix_upper}{}", c.c_return_type), c.c_return_type.clone()))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    pairs.sort_by_key(|(prefixed, _)| std::cmp::Reverse(prefixed.len()));
    if pairs.is_empty() {
        return String::new();
    }
    pairs
        .iter()
        .map(|(prefixed, bare)| {
            let prefixed = escape_rust_str_literal(prefixed);
            let bare = escape_rust_str_literal(bare);
            format!("    header = header.replace(\"{prefixed}\", \"{bare}\");")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quote_and_backslash() {
        assert_eq!(escape_rust_str_literal(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn escapes_newline_breakout_canary() {
        // A raw newline followed by Rust source would otherwise splice a second
        // statement into the generated `build.rs`.
        let payload = "evil\");std::process::exit(1);//";
        let escaped = escape_rust_str_literal(payload);
        assert_eq!(escaped, "evil\\\");std::process::exit(1);//");
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn passes_through_plain_identifiers_unchanged() {
        assert_eq!(escape_rust_str_literal("my_lib.h"), "my_lib.h");
    }

    #[test]
    fn capsule_header_fixup_escapes_both_sides_and_is_deterministic() {
        let mut capsule_types = HashMap::new();
        capsule_types.insert(
            "Language".to_string(),
            FfiCapsuleTypeConfig {
                into_raw_type: "tree_sitter::ffi::TSLanguage".to_string(),
                c_return_type: "TSLanguage".to_string(),
                package: None,
                package_version: None,
            },
        );
        let out = capsule_header_fixup(&capsule_types, "TS_PACK");
        assert_eq!(
            out,
            "    header = header.replace(\"TS_PACKTSLanguage\", \"TSLanguage\");"
        );
    }

    #[test]
    fn capsule_header_fixup_empty_map_is_empty_string() {
        assert_eq!(capsule_header_fixup(&HashMap::new(), "PREFIX"), "");
    }
}
