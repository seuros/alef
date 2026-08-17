//! Detects breaking changes to a backend's emitted public function signatures between
//! generation runs, and flags hand-maintained consumer files (a test, an example, a
//! hand-written wrapper) that reference the changed symbol so a build failure there is not
//! silent.
//!
//! alef must never edit a hand-maintained file to keep it in sync with a regenerated
//! signature — that would clobber content it does not own. What it *can* do, and did not
//! do before this module existed, is say so: name the symbol, the old and new signature,
//! and the file(s) that will stop compiling.
//!
//! The baseline this compares against reuses [`cache::read_stage_paths`] /
//! [`cache::write_stage_hash`] — the same generic "line-per-entry manifest" mechanism the
//! binding-orphan sweep uses for its previous-run path list (see that mechanism's own doc
//! in `cli::cache`). A signature is encoded as one manifest line per function
//! (`encode_signature`/`decode_signature`), so no change to `cli::cache` itself is needed
//! to add this baseline. ~keep
//!
//! Coverage: only the Zig backend currently implements
//! [`crate::core::backend::Backend::public_function_signatures`] (see
//! `backends::zig::gen_bindings::mod::ZigBackend::public_function_signatures`) and only Zig
//! has a caller-file extension registered in [`scan_extensions_for`]. Every other language
//! is silently uncovered today — its baseline stays permanently empty because its backend
//! returns no signatures, so [`detect_breaking_changes`] never has anything to compare and
//! never fires. Adding a language is two steps: implement the trait method on that
//! backend, and add its source-file extension(s) to [`scan_extensions_for`].

use crate::cli::cache;
use crate::core::backend::EmittedSignature;
use crate::core::config::Language;
use std::path::{Path, PathBuf};

/// What changed about a symbol's signature between the previous run's baseline and this
/// run's emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureChangeKind {
    /// Only the return type changed (e.g. plain type to an error-union/Result wrapper).
    ReturnType,
    /// Only the parameter list changed (count, order, or a parameter's type).
    Params,
    /// Both the return type and the parameter list changed.
    ReturnTypeAndParams,
    /// The symbol existed in the previous baseline and is absent from this run's emission.
    Removed,
}

/// A single breaking change to a generated public symbol, detected by
/// [`detect_breaking_changes`]. `current` is `None` for [`SignatureChangeKind::Removed`].
#[derive(Debug, Clone)]
pub struct BreakingChange {
    pub symbol: String,
    pub kind: SignatureChangeKind,
    pub previous: EmittedSignature,
    pub current: Option<EmittedSignature>,
}

/// Compare a previous run's signature baseline against this run's emission and return
/// every breaking change: a symbol removed, or a symbol whose params or return type text
/// changed. A symbol present only in `current` (purely additive) is never reported — this
/// only ever iterates `previous`. ~keep
pub fn detect_breaking_changes(previous: &[EmittedSignature], current: &[EmittedSignature]) -> Vec<BreakingChange> {
    let current_by_symbol: std::collections::HashMap<&str, &EmittedSignature> =
        current.iter().map(|sig| (sig.symbol.as_str(), sig)).collect();

    previous
        .iter()
        .filter_map(|old| match current_by_symbol.get(old.symbol.as_str()) {
            None => Some(BreakingChange {
                symbol: old.symbol.clone(),
                kind: SignatureChangeKind::Removed,
                previous: old.clone(),
                current: None,
            }),
            Some(new) => {
                let return_changed = old.return_type != new.return_type;
                let params_changed = old.params != new.params;
                let kind = match (return_changed, params_changed) {
                    (true, true) => SignatureChangeKind::ReturnTypeAndParams,
                    (true, false) => SignatureChangeKind::ReturnType,
                    (false, true) => SignatureChangeKind::Params,
                    (false, false) => return None,
                };
                Some(BreakingChange {
                    symbol: old.symbol.clone(),
                    kind,
                    previous: old.clone(),
                    current: Some((*new).clone()),
                })
            }
        })
        .collect()
}

/// Source-file extensions (without the leading dot) a hand-maintained caller for
/// `language` could live in. An empty slice means no caller scan is wired up for that
/// language yet — see the module doc's coverage note. ~keep
fn scan_extensions_for(language: Language) -> &'static [&'static str] {
    match language {
        Language::Zig => &["zig"],
        _ => &[],
    }
}

/// Directory names never worth descending into while looking for a hand-maintained
/// caller: they hold vendored/generated/cache content, not source a consumer wrote. ~keep
const SCAN_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".alef",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "__pycache__",
    ".zig-cache",
    "zig-out",
];

/// Find files under `base_dir` (matching one of `extensions`) that reference `symbol` as a
/// whole token and do not carry an alef marker — i.e. files alef did not generate and will
/// not overwrite, so a breaking change to `symbol` is a real, silent build risk for them.
///
/// This is a plain substring/token scan, not a parser for any target language: it cannot
/// tell a real call site from a comment or an unrelated identifier that happens to match.
/// That trade favors naming a file that turns out fine over missing one that breaks. ~keep
pub fn find_hand_maintained_callers(base_dir: &Path, symbol: &str, extensions: &[&str]) -> Vec<PathBuf> {
    if extensions.is_empty() || symbol.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let mut stack = vec![base_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let is_excluded = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| SCAN_EXCLUDED_DIRS.contains(&name));
                if !is_excluded {
                    stack.push(path);
                }
                continue;
            }
            let matches_extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext));
            if !matches_extension {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if crate::core::hash::content_has_alef_marker(&content) {
                continue;
            }
            if references_symbol(&content, symbol) {
                hits.push(path);
            }
        }
    }
    hits.sort();
    hits
}

/// Whole-token scan for `symbol` in `content` — a match on `foo_bar` does not match a
/// substring inside `foo_bar_baz`.
fn references_symbol(content: &str, symbol: &str) -> bool {
    content
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|token| token == symbol)
}

fn format_signature(sig: &EmittedSignature) -> String {
    format!("({}) {}", sig.params, sig.return_type)
}

/// `WARN`, never fail-the-run: a consumer may have intended the change and already updated
/// its own callers out-of-band (or is mid-migration), so failing generation on this would
/// be worse than the prior silence. The value here is attribution, not enforcement. Only
/// reports when at least one hand-maintained caller was found — a breaking change whose
/// only references are in alef-owned (regenerated) files is not this consumer's problem.
/// ~keep
fn report_breaking_change(language: Language, change: &BreakingChange, callers: &[PathBuf]) {
    if callers.is_empty() {
        return;
    }
    let callers_display = callers
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    match &change.current {
        Some(current) => {
            tracing::warn!(
                language = %language,
                symbol = %change.symbol,
                old_signature = %format_signature(&change.previous),
                new_signature = %format_signature(current),
                callers = %callers_display,
                "breaking signature change to a generated public function; hand-maintained caller(s) \
                 will not build against the regenerated signature: {callers_display}"
            );
        }
        None => {
            tracing::warn!(
                language = %language,
                symbol = %change.symbol,
                old_signature = %format_signature(&change.previous),
                callers = %callers_display,
                "generated public function removed; hand-maintained caller(s) reference it and will \
                 not build: {callers_display}"
            );
        }
    }
}

fn signature_stage_name(language: Language) -> String {
    format!("public-signatures-{language}")
}

/// A signature encoded as a single manifest line, reusing [`cache::read_stage_paths`] /
/// [`cache::write_stage_hash`]'s "one entry per line" format without teaching that
/// mechanism anything about signatures. Tab-separated: none of `symbol`/`params`/
/// `return_type` can contain a literal tab or newline in any backend's rendering today.
fn encode_signature(sig: &EmittedSignature) -> PathBuf {
    PathBuf::from(format!("{}\t{}\t{}", sig.symbol, sig.params, sig.return_type))
}

fn decode_signature(entry: &Path) -> Option<EmittedSignature> {
    let text = entry.to_string_lossy();
    let mut parts = text.splitn(3, '\t');
    let symbol = parts.next()?.to_string();
    let params = parts.next()?.to_string();
    let return_type = parts.next()?.to_string();
    Some(EmittedSignature {
        symbol,
        params,
        return_type,
    })
}

fn read_signatures(crate_name: &str, stage: &str) -> Vec<EmittedSignature> {
    cache::read_stage_paths(crate_name, stage)
        .iter()
        .filter_map(|entry| decode_signature(entry))
        .collect()
}

fn write_signatures(crate_name: &str, stage: &str, signatures: &[EmittedSignature]) -> anyhow::Result<()> {
    let encoded: Vec<PathBuf> = signatures.iter().map(encode_signature).collect();
    let joined = encoded
        .iter()
        .map(|entry| entry.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    let stage_hash = cache::hash_content(&joined);
    cache::write_stage_hash(crate_name, stage, &stage_hash, &encoded)
}

/// Read the previous run's signature baseline for `language`, compare it against
/// `current`, and `WARN` about every breaking change that has at least one
/// hand-maintained caller — then persist `current` as the new baseline.
///
/// Must be called with the baseline read happening before this run's write, same ordering
/// discipline as the binding-orphan sweep's stage manifest (see the module doc). Skips
/// entirely when both the previous baseline and `current` are empty, so a language no
/// backend has wired up yet (see [`scan_extensions_for`]) never accumulates an empty
/// manifest file per crate. ~keep
pub fn check_signature_breakage(language: Language, crate_name: &str, base_dir: &Path, current: &[EmittedSignature]) {
    let stage = signature_stage_name(language);
    let previous = read_signatures(crate_name, &stage);
    if previous.is_empty() && current.is_empty() {
        return;
    }

    let changes = detect_breaking_changes(&previous, current);
    let extensions = scan_extensions_for(language);
    if !changes.is_empty() && extensions.is_empty() {
        tracing::warn!(
            language = %language,
            changed = changes.len(),
            "detected breaking signature change(s) for this language, but no consumer-file scan is \
             wired up to attribute hand-maintained callers yet -- see `scan_extensions_for`"
        );
    } else {
        for change in &changes {
            let callers = find_hand_maintained_callers(base_dir, &change.symbol, extensions);
            report_breaking_change(language, change, &callers);
        }
    }

    if let Err(error) = write_signatures(crate_name, &stage, current) {
        tracing::debug!(%error, language = %language, "failed to persist signature baseline");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    fn sig(symbol: &str, params: &str, return_type: &str) -> EmittedSignature {
        EmittedSignature {
            symbol: symbol.to_string(),
            params: params.to_string(),
            return_type: return_type.to_string(),
        }
    }

    #[test]
    fn breaking_return_type_change_is_reported() {
        let previous = vec![sig("do_thing", "a: i32", "void")];
        let current = vec![sig("do_thing", "a: i32", "error{OutOfMemory}!void")];

        let changes = detect_breaking_changes(&previous, &current);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "do_thing");
        assert_eq!(changes[0].kind, SignatureChangeKind::ReturnType);
        assert_eq!(changes[0].previous.return_type, "void");
        assert_eq!(
            changes[0].current.as_ref().unwrap().return_type,
            "error{OutOfMemory}!void"
        );
    }

    #[test]
    fn additive_new_symbol_is_not_reported() {
        let previous = vec![sig("existing", "a: i32", "void")];
        let current = vec![sig("existing", "a: i32", "void"), sig("brand_new", "", "void")];

        let changes = detect_breaking_changes(&previous, &current);

        assert!(
            changes.is_empty(),
            "a purely additive symbol must not be reported: {changes:?}"
        );
    }

    #[test]
    fn unchanged_signature_is_not_reported() {
        let previous = vec![sig("stable", "a: i32", "void")];
        let current = vec![sig("stable", "a: i32", "void")];

        assert!(detect_breaking_changes(&previous, &current).is_empty());
    }

    #[test]
    fn removed_symbol_is_reported_as_removed() {
        let previous = vec![sig("gone", "a: i32", "void")];
        let current: Vec<EmittedSignature> = vec![];

        let changes = detect_breaking_changes(&previous, &current);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, SignatureChangeKind::Removed);
        assert!(changes[0].current.is_none());
    }

    #[test]
    fn params_and_return_type_both_changing_is_categorized_as_both() {
        let previous = vec![sig("both", "a: i32", "void")];
        let current = vec![sig("both", "a: i32, b: i32", "error{OutOfMemory}!void")];

        let changes = detect_breaking_changes(&previous, &current);

        assert_eq!(changes[0].kind, SignatureChangeKind::ReturnTypeAndParams);
    }

    #[test]
    fn only_hand_maintained_caller_is_attributed_alef_owned_caller_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hand_written = dir.path().join("hand_written_test.zig");
        std::fs::write(&hand_written, "test \"calls do_thing\" {\n    try do_thing(1);\n}\n").expect("write");

        let alef_owned = dir.path().join("generated.zig");
        let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
        let stamped = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
        std::fs::write(&alef_owned, format!("{stamped}\npub fn do_thing(a: i32) void {{}}\n")).expect("write");

        let callers = find_hand_maintained_callers(dir.path(), "do_thing", &["zig"]);

        assert_eq!(
            callers,
            vec![hand_written.clone()],
            "must name the hand-maintained file and must not name the alef-owned one"
        );
    }

    #[test]
    fn caller_referencing_only_a_substring_of_the_symbol_is_not_attributed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let unrelated = dir.path().join("unrelated.zig");
        std::fs::write(&unrelated, "pub fn do_thing_else() void {}\n").expect("write");

        let callers = find_hand_maintained_callers(dir.path(), "do_thing", &["zig"]);

        assert!(
            callers.is_empty(),
            "a substring match must not count as a reference: {callers:?}"
        );
    }

    #[test]
    #[traced_test]
    fn report_breaking_change_is_silent_when_no_hand_maintained_caller_references_it() {
        let change = BreakingChange {
            symbol: "solo".to_string(),
            kind: SignatureChangeKind::ReturnType,
            previous: sig("solo", "", "void"),
            current: Some(sig("solo", "", "error{OutOfMemory}!void")),
        };
        report_breaking_change(Language::Zig, &change, &[]);
        assert!(
            !logs_contain("breaking signature change"),
            "a change with no hand-maintained caller must not warn"
        );
    }

    #[test]
    #[traced_test]
    fn report_breaking_change_warns_naming_symbol_signatures_and_caller_when_a_hand_maintained_caller_exists() {
        let change = BreakingChange {
            symbol: "do_thing".to_string(),
            kind: SignatureChangeKind::ReturnType,
            previous: sig("do_thing", "a: i32", "void"),
            current: Some(sig("do_thing", "a: i32", "error{OutOfMemory}!void")),
        };
        report_breaking_change(
            Language::Zig,
            &change,
            &[PathBuf::from("packages/zig/test/lib_test.zig")],
        );

        assert!(logs_contain("breaking signature change"));
        assert!(logs_contain("do_thing"));
        assert!(logs_contain("packages/zig/test/lib_test.zig"));
    }

    #[test]
    fn round_trips_signature_through_the_manifest_encoding() {
        let original = sig("my_fn", "a: i32, b: []const u8", "error{OutOfMemory}!MyStruct");
        let decoded = decode_signature(&encode_signature(&original)).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn check_signature_breakage_skips_persisting_when_language_has_no_signatures_at_all() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(dir.path());

        check_signature_breakage(Language::Python, "no-python-coverage", dir.path(), &[]);

        let manifest = dir
            .path()
            .join(".alef/no-python-coverage/hashes/public-signatures-python.manifest");
        let exists = manifest.exists();

        assert!(
            !exists,
            "an uncovered language must not accumulate an empty baseline file"
        );
    }
}
