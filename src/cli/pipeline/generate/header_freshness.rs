use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Fail generation when the on-disk C header and the freshly generated FFI
/// exports come from different runs.
///
/// The header is produced by cbindgen inside the *consumer's* `build.rs`, so it
/// only refreshes on `cargo build` — never on `alef generate`. Alef nonetheless
/// reads it back during generation (the Zig backend vendors it into
/// `packages/zig/include/`, and consumer build scripts fan it out further), so a
/// bare `alef generate` can publish a header describing the previous run's ABI
/// while reporting success. This check makes that divergence loud instead of
/// silent. ~keep
pub(super) fn check_ffi_header_freshness(
    results: &[(Language, Vec<GeneratedFile>)],
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> anyhow::Result<()> {
    let Some(source) = generated_ffi_source(results) else {
        return Ok(());
    };
    let exported = exported_symbols(source);
    if exported.is_empty() {
        return Ok(());
    }

    let header_path = ffi_header_path(config, base_dir);
    let Ok(header) = std::fs::read_to_string(&header_path) else {
        // A project generating for the first time has no header yet. Failing
        // closed here would break bootstrapping to fix a staleness bug, so an
        // absent header is reported and generation continues. ~keep
        tracing::warn!(
            "FFI header {} not found — skipping freshness check. Run a build so cbindgen emits it.",
            header_path.display()
        );
        return Ok(());
    };

    let prefix = config.ffi_prefix();
    let declared = header_declared_functions(&header, &prefix);

    let missing: Vec<&String> = exported.iter().filter(|name| !declared.contains(*name)).collect();
    let removed: Vec<&String> = declared.iter().filter(|name| !exported.contains(*name)).collect();

    if missing.is_empty() && removed.is_empty() {
        return Ok(());
    }

    Err(anyhow::anyhow!("{}", drift_message(&header_path, &missing, &removed)))
}

fn drift_message(header_path: &Path, missing: &[&String], removed: &[&String]) -> String {
    let mut message = format!(
        "generated FFI exports and the C header at {} are from different runs.\n\
         The header is produced by cbindgen in the crate's build.rs, not by alef generate.",
        header_path.display()
    );
    if !missing.is_empty() {
        message.push_str("\n  exported by the generated source but absent from the header:");
        for name in missing {
            message.push_str(&format!("\n    {name}"));
        }
    }
    if !removed.is_empty() {
        message.push_str("\n  declared by the header but no longer exported:");
        for name in removed {
            message.push_str(&format!("\n    {name}"));
        }
    }
    message.push_str("\n  Run a cargo build so cbindgen regenerates the header, then re-run generation.");
    message
}

/// Locate the generated FFI crate's `src/lib.rs` among this run's output.
fn generated_ffi_source(results: &[(Language, Vec<GeneratedFile>)]) -> Option<&str> {
    results
        .iter()
        .find(|(lang, _)| *lang == Language::Ffi)?
        .1
        .iter()
        .find(|file| file.path.ends_with("src/lib.rs"))
        .map(|file| file.content.as_str())
}

/// Resolve the header the same way the Zig backend does, so there is one
/// resolution rule rather than two. `ffi_header_name` already carries the
/// `.h` suffix. ~keep
fn ffi_header_path(config: &ResolvedCrateConfig, base_dir: &Path) -> PathBuf {
    let crate_path = config.ffi_crate_path();
    let crate_root = crate_path.strip_prefix("../../").unwrap_or(&crate_path);
    base_dir.join(crate_root).join("include").join(config.ffi_header_name())
}

/// Collect the `#[no_mangle] extern "C"` function names the generated source
/// exports.
fn exported_symbols(source: &str) -> BTreeSet<String> {
    let mut exports = BTreeSet::new();
    let mut no_mangle_seen = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            if trimmed.contains("no_mangle") {
                no_mangle_seen = true;
            }
            continue;
        }
        if let Some(name) = extern_c_fn_name(trimmed) {
            if no_mangle_seen {
                exports.insert(name);
            }
            no_mangle_seen = false;
            continue;
        }
        no_mangle_seen = false;
    }

    exports
}

fn extern_c_fn_name(line: &str) -> Option<String> {
    const MARKER: &str = "extern \"C\" fn ";
    let start = line.find(MARKER)? + MARKER.len();
    let rest = &line[start..];
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// Collect prefixed snake_case function names declared by the header.
///
/// cbindgen emits function names in snake_case and type names in PascalCase, so
/// the case of the character after the prefix discriminates the two. Lines
/// opening or continuing a doc comment are skipped, so a symbol merely mentioned
/// in prose is not mistaken for a declaration. ~keep
fn header_declared_functions(header: &str, prefix: &str) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    let needle = format!("{prefix}_");

    for line in header.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        for (offset, _) in trimmed.match_indices(&needle) {
            if offset > 0 && is_identifier_char(trimmed.as_bytes()[offset - 1]) {
                continue;
            }
            let candidate: String = trimmed[offset..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let follows = trimmed[offset + candidate.len()..].starts_with('(');
            if follows && !candidate.chars().any(|c| c.is_uppercase()) {
                declared.insert(candidate);
            }
        }
    }

    declared
}

fn is_identifier_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn my_lib_open(handle: AlefHandle) -> AlefHandle {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn my_lib_close(handle: AlefHandle) {
}

// A helper that is deliberately not exported.
pub extern "C" fn my_lib_internal_helper() {
}
"#;

    #[test]
    fn should_collect_only_no_mangle_exports() {
        let exports = exported_symbols(SOURCE);
        assert_eq!(
            exports,
            BTreeSet::from(["my_lib_open".to_owned(), "my_lib_close".to_owned()]),
            "only #[no_mangle] functions are part of the C ABI"
        );
    }

    #[test]
    fn should_collect_declared_functions_ignoring_types_and_prose() {
        let header = r#"
/* This file is auto-generated by cbindgen. DO NOT EDIT. */

typedef struct MyLibHandle MyLibHandle;

/**
 * Doc comment mentioning my_lib_removed(handle) in prose.
 */
AlefHandle my_lib_open(AlefHandle handle);

void my_lib_close(AlefHandle handle);
"#;
        let declared = header_declared_functions(header, "my_lib");
        assert_eq!(
            declared,
            BTreeSet::from(["my_lib_open".to_owned(), "my_lib_close".to_owned()]),
            "prose mentions and PascalCase type names must not count as declarations"
        );
    }

    #[test]
    fn should_report_export_missing_from_stale_header() {
        let header = "AlefHandle my_lib_open(AlefHandle handle);\n";
        let exported = exported_symbols(SOURCE);
        let declared = header_declared_functions(header, "my_lib");
        let missing: Vec<&String> = exported.iter().filter(|name| !declared.contains(*name)).collect();
        let removed: Vec<&String> = declared.iter().filter(|name| !exported.contains(*name)).collect();

        assert_eq!(missing, vec![&"my_lib_close".to_owned()]);
        assert!(removed.is_empty());

        let message = drift_message(Path::new("include/my_lib.h"), &missing, &removed);
        assert!(
            message.contains("my_lib_close"),
            "the failure must name the drifting symbol, got:\n{message}"
        );
        assert!(
            message.contains("cbindgen"),
            "the failure must explain that a build regenerates the header, got:\n{message}"
        );
    }

    #[test]
    fn should_report_symbol_the_header_still_declares_after_removal() {
        let header = "AlefHandle my_lib_open(AlefHandle handle);\n\
                      void my_lib_close(AlefHandle handle);\n\
                      void my_lib_removed(AlefHandle handle);\n";
        let exported = exported_symbols(SOURCE);
        let declared = header_declared_functions(header, "my_lib");
        let removed: Vec<&String> = declared.iter().filter(|name| !exported.contains(*name)).collect();

        assert_eq!(removed, vec![&"my_lib_removed".to_owned()]);
    }

    #[test]
    fn should_pass_when_header_matches_generated_exports() {
        let header = "AlefHandle my_lib_open(AlefHandle handle);\n\
                      void my_lib_close(AlefHandle handle);\n";
        let exported = exported_symbols(SOURCE);
        let declared = header_declared_functions(header, "my_lib");

        assert!(exported.iter().all(|name| declared.contains(name)));
        assert!(declared.iter().all(|name| exported.contains(name)));
    }
}
