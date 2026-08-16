use crate::core::config::ResolvedCrateConfig;
use anyhow::Context as _;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Fail generation when the on-disk C header and FFI source come from different
/// runs.
///
/// The header is produced by cbindgen inside the *consumer's* `build.rs`, so it
/// only refreshes on `cargo build` — never on `alef generate`. Alef nonetheless
/// reads it back during generation (the Zig backend vendors it into
/// `packages/zig/include/`, and consumer build scripts fan it out further), so a
/// bare `alef generate` can publish a header describing the previous run's ABI
/// while reporting success. Call this after writing bindings: a failing run must
/// leave the new Rust source on disk so the requested Cargo build can refresh the
/// header rather than rebuilding the old source forever. ~keep
pub(crate) fn check_ffi_header_freshness(config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<()> {
    match ffi_header_freshness(config, base_dir)? {
        HeaderFreshness::Fresh => Ok(()),
        HeaderFreshness::Missing(header_path) => {
            tracing::warn!(
                "FFI header {} not found — skipping freshness check. Run a build so cbindgen emits it.",
                header_path.display()
            );
            Ok(())
        }
        HeaderFreshness::Stale(message) => Err(anyhow::anyhow!(message)),
    }
}

/// Refresh a missing or stale header before enforcing the freshness gate.
///
/// Full generation workflows use this after every FFI source-writing stage has
/// completed. The callback keeps command orchestration outside this module and
/// makes the refresh-before-validation ordering directly testable. ~keep
pub(crate) fn ensure_ffi_header_freshness(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    refresh: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    match ffi_header_freshness(config, base_dir)? {
        HeaderFreshness::Fresh => return Ok(()),
        HeaderFreshness::Missing(header_path) => {
            tracing::info!("Building FFI crate to create header {}", header_path.display());
        }
        HeaderFreshness::Stale(_) => {
            tracing::info!("Building FFI crate to refresh its cbindgen header");
        }
    }

    refresh().context("failed to refresh the generated FFI header")?;
    check_ffi_header_freshness(config, base_dir)
}

enum HeaderFreshness {
    Fresh,
    Missing(PathBuf),
    Stale(String),
}

/// Three independent dimensions are compared, because a header can drift from
/// its source along any of them without the others noticing:
///
/// 1. **Symbol names** — every `#[no_mangle] extern "C" fn` in the generated
///    source must have a matching declaration in the header, and vice versa.
/// 2. **cfg guard parity** — a source export carrying `#[cfg(feature = "x")]`
///    must be declared inside a header `#if defined(PREFIX_FEATURE_X)` guard;
///    an export with no cfg must be declared unconditionally. This is the
///    dimension the original defect lived in: a malformed cbindgen `[defines]`
///    key made every gated export's guard silently vanish while every name
///    still matched.
/// 3. **Typedef kind** — a type the source defines as a real `#[repr(...)]`
///    struct must be declared `typedef struct X X;` in the header, not a
///    scalar integer; and no header typedef name may flip kind between its
///    own conditional branches.
///
/// What this still cannot see, even with all three dimensions:
/// - Guard *polarity*. The macro-presence check only confirms the right
///   macro name is referenced inside an enclosing `#if`; it cannot tell
///   `#if defined(X)` from `#if !defined(X)`, so a source `not(feature = "x")`
///   predicate is not verified against the header's negation.
/// - Non-feature cfg predicates (`target_os`, `target_arch`,
///   `debug_assertions`, …). Macro names are only derivable for
///   `feature = "..."` predicates — the same limit cbindgen's own
///   `[defines]` table has for this codebase — so a symbol gated purely on
///   a non-feature predicate is not guard-checked at all.
/// - Struct field *layout* (order, size, padding) — only the struct-vs-scalar
///   *kind* is compared, never the fields inside a struct typedef.
/// - Opaque-handle types that never get a literal `pub type X = u64;` alias
///   in the generated source (the common case: they're boxed through the
///   shared `AlefHandle` registry and referenced inline, with no per-type
///   Rust alias to compare against). For those, only the header's own
///   internal kind-consistency is checked — there is no source-derived
///   expectation to check it against.
/// - Any type that has no header typedef at all is invisible to the typedef
///   check; only names present in *both* source and header are compared. ~keep
fn ffi_header_freshness(config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<HeaderFreshness> {
    let source_root = ffi_source_root(config, base_dir);
    let (exported, type_hints) = scan_generated_ffi_source(&source_root)?;
    if exported.is_empty() && type_hints.is_empty() {
        return Ok(HeaderFreshness::Fresh);
    }

    let header_path = ffi_header_path(config, base_dir);
    let Ok(header) = std::fs::read_to_string(&header_path) else {
        return Ok(HeaderFreshness::Missing(header_path));
    };

    let prefix = config.ffi_prefix();
    let prefix_upper = prefix.to_uppercase();
    let code = strip_c_comments(&header);

    let declared_occurrences = header_declared_function_occurrences(&code, &prefix);
    let declared_names: BTreeSet<String> = declared_occurrences.iter().map(|(_, name)| name.clone()).collect();

    let missing: Vec<&String> = exported.keys().filter(|name| !declared_names.contains(*name)).collect();
    let removed: Vec<&String> = declared_names
        .iter()
        .filter(|name| !exported.contains_key(name.as_str()))
        .collect();

    let guard_problems = function_guard_drift(&exported, &declared_occurrences, &code, &prefix_upper);
    let typedef_problems = typedef_kind_problems(&type_hints, &code, &prefix_upper);

    if missing.is_empty() && removed.is_empty() && guard_problems.is_empty() && typedef_problems.is_empty() {
        return Ok(HeaderFreshness::Fresh);
    }

    Ok(HeaderFreshness::Stale(drift_message(
        &header_path,
        &missing,
        &removed,
        &guard_problems,
        &typedef_problems,
    )))
}

fn drift_message(
    header_path: &Path,
    missing: &[&String],
    removed: &[&String],
    guard_problems: &[String],
    typedef_problems: &[String],
) -> String {
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
    if !guard_problems.is_empty() {
        message.push_str("\n  cfg guard drift between the generated source and the header's #if guards:");
        for problem in guard_problems {
            message.push_str(&format!("\n    {problem}"));
        }
    }
    if !typedef_problems.is_empty() {
        message.push_str("\n  typedef representation drift (opaque struct vs. scalar handle):");
        for problem in typedef_problems {
            message.push_str(&format!("\n    {problem}"));
        }
    }
    message.push_str("\n  Run a cargo build so cbindgen regenerates the header, then re-run generation.");
    message
}

fn ffi_source_root(config: &ResolvedCrateConfig, base_dir: &Path) -> PathBuf {
    ffi_crate_root(config, base_dir).join("src")
}

/// Resolve the header the same way the Zig backend does, so there is one
/// resolution rule rather than two. `ffi_header_name` already carries the
/// `.h` suffix. ~keep
fn ffi_header_path(config: &ResolvedCrateConfig, base_dir: &Path) -> PathBuf {
    ffi_crate_root(config, base_dir)
        .join("include")
        .join(config.ffi_header_name())
}

fn ffi_crate_root(config: &ResolvedCrateConfig, base_dir: &Path) -> PathBuf {
    let crate_path = config.ffi_crate_path();
    let crate_root = crate_path.strip_prefix("../../").unwrap_or(&crate_path);
    base_dir.join(crate_root)
}

fn rust_source_files(source_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut source_paths = walkdir::WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "rs") => {
                Some(Ok(entry.into_path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    source_paths.sort();
    Ok(source_paths)
}

/// Walk every generated `.rs` file once, collecting both the exported-symbol
/// map (name -> its source `#[cfg(...)]`, if any) and the typedef-kind hints
/// (Rust type name -> struct or scalar) in the same pass. ~keep
fn scan_generated_ffi_source(
    source_root: &Path,
) -> anyhow::Result<(BTreeMap<String, Option<String>>, BTreeMap<String, TypedefKind>)> {
    let mut exports = BTreeMap::new();
    let mut type_hints = BTreeMap::new();
    for source_path in rust_source_files(source_root)? {
        let source = std::fs::read_to_string(&source_path)
            .with_context(|| format!("failed to read generated FFI source at {}", source_path.display()))?;
        exports.extend(scan_exported_symbols(&source));
        type_hints.extend(rust_type_kind_hints(&source));
    }
    Ok((exports, type_hints))
}

/// Test-only convenience wrapper: production code needs the cfg-carrying map
/// from `scan_generated_ffi_source`, not a bare name set. ~keep
#[cfg(test)]
fn exported_symbols_in_dir(source_root: &Path) -> anyhow::Result<BTreeSet<String>> {
    Ok(scan_generated_ffi_source(source_root)?.0.into_keys().collect())
}

/// Collect the `#[no_mangle] extern "C"` function names the generated source
/// exports, paired with the source `#[cfg(...)]` predicate (if any) that
/// gates each one. cfg is always the attribute immediately above
/// `#[unsafe(no_mangle)]` in every alef FFI template — see
/// `backends/ffi/templates/free_function_header.jinja` and the sibling
/// `_len` companion in `backends/ffi/gen_bindings/functions/signatures.rs`.
/// A hand-written `[crates.custom_modules] ffi` file that instead wraps a
/// whole `mod`/`impl` block in `#[cfg(...)]` is invisible to this per-fn
/// scan — same blind spot the original name-only scanner already had. ~keep
fn scan_exported_symbols(source: &str) -> BTreeMap<String, Option<String>> {
    let mut exports = BTreeMap::new();
    let mut no_mangle_seen = false;
    let mut pending_cfg: Option<String> = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            if trimmed.contains("no_mangle") {
                no_mangle_seen = true;
            }
            if let Some(cfg) = extract_cfg_attribute(trimmed) {
                pending_cfg = Some(cfg);
            }
            continue;
        }
        if let Some(name) = extern_c_fn_name(trimmed) {
            if no_mangle_seen {
                exports.insert(name, pending_cfg.take());
            }
            no_mangle_seen = false;
            pending_cfg = None;
            continue;
        }
        no_mangle_seen = false;
        pending_cfg = None;
    }

    exports
}

fn extract_cfg_attribute(trimmed_line: &str) -> Option<String> {
    let rest = trimmed_line.strip_prefix("#[cfg(")?;
    let rest = rest.strip_suffix(")]")?;
    Some(rest.to_string())
}

/// Test-only convenience wrapper around `scan_exported_symbols` for tests
/// that only care about names, not cfg predicates. ~keep
#[cfg(test)]
fn exported_symbols(source: &str) -> BTreeSet<String> {
    scan_exported_symbols(source).into_keys().collect()
}

fn extern_c_fn_name(line: &str) -> Option<String> {
    const MARKER: &str = "extern \"C\" fn ";
    let start = line.find(MARKER)? + MARKER.len();
    let rest = &line[start..];
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// Collect prefixed snake_case function-name occurrences declared by the
/// header, with the byte offset (into the comment-stripped `code` passed in)
/// where each occurrence starts — the offset is what lets the guard-parity
/// check find which `#if` frames enclose a given declaration.
///
/// cbindgen emits function names in snake_case and type names in PascalCase,
/// so the case of the character after the prefix discriminates the two.
/// Whitespace before `(` is accepted because C formatters may split a long
/// declaration after its function name. ~keep
fn header_declared_function_occurrences(code: &str, prefix: &str) -> Vec<(usize, String)> {
    let mut declared = Vec::new();
    let needle = format!("{prefix}_");

    for (offset, _) in code.match_indices(&needle) {
        if offset > 0 && is_identifier_char(code.as_bytes()[offset - 1]) {
            continue;
        }
        let candidate: String = code[offset..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let suffix = &code[offset + candidate.len()..];
        let is_declaration = suffix.trim_start().starts_with('(');
        if is_declaration && !candidate.chars().any(|c| c.is_uppercase()) {
            declared.push((offset, candidate));
        }
    }

    declared
}

/// Test-only convenience wrapper: production code needs byte offsets (to
/// resolve guard membership), so it calls `header_declared_function_occurrences`
/// directly on an already comment-stripped `code` string. ~keep
#[cfg(test)]
fn header_declared_functions(header: &str, prefix: &str) -> BTreeSet<String> {
    let code = strip_c_comments(header);
    header_declared_function_occurrences(&code, prefix)
        .into_iter()
        .map(|(_, name)| name)
        .collect()
}

fn strip_c_comments(source: &str) -> String {
    let mut code = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_block_comment = false;

    while let Some(character) = chars.next() {
        if in_block_comment {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment = true;
            continue;
        }
        if character == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for comment_character in chars.by_ref() {
                if comment_character == '\n' {
                    code.push('\n');
                    break;
                }
            }
            continue;
        }
        code.push(character);
    }

    code
}

fn is_identifier_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

// --- cfg guard parity -------------------------------------------------------

/// What an enclosing `#if`/`#elif`/`#ifdef` contributes to a declaration's
/// guard: either a known set of `defined(MACRO)` names, or `Opaque` for a
/// condition this scanner can't attribute to specific macros (`#ifndef`, or
/// any `#if` whose condition has no literal `defined(...)`, such as a C23
/// `__STDC_VERSION__` check). `Opaque` still counts as "some guard is
/// active" — it just contributes no macro names to the prefix-scoped
/// membership test, so it can never falsely satisfy or falsely violate a
/// feature-macro check. ~keep
#[derive(Clone)]
enum GuardFrame {
    Defined(BTreeSet<String>),
    Opaque,
}

fn parse_guard_condition(condition: &str) -> GuardFrame {
    let mut macros = BTreeSet::new();
    let mut remainder = condition;
    while let Some(start) = remainder.find("defined(") {
        let after = &remainder[start + "defined(".len()..];
        let Some(end) = after.find(')') else { break };
        let name = after[..end].trim();
        if !name.is_empty() {
            macros.insert(name.to_string());
        }
        remainder = &after[end + 1..];
    }
    if macros.is_empty() {
        GuardFrame::Opaque
    } else {
        GuardFrame::Defined(macros)
    }
}

/// Active `#if`/`#ifdef`/`#else` nesting at each line of a comment-stripped
/// header, as the byte offset where the line begins paired with the guard
/// stack in effect for that line.
///
/// This is a line-oriented approximation, not a real preprocessor: `#elif`
/// forgets that prior branches were false (it just replaces the top frame),
/// and `#ifndef`/`#else` collapse to `GuardFrame::Opaque`. Both
/// simplifications are safe for this checker's purpose — they can only make
/// a guard look less specific than it is, never invent a macro membership
/// that isn't literally written in an `#if` condition. ~keep
fn guard_stack_by_line(code: &str) -> Vec<(usize, Vec<GuardFrame>)> {
    let mut stack: Vec<GuardFrame> = Vec::new();
    let mut lines = Vec::new();
    let mut offset = 0usize;

    for line in code.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(condition) = trimmed.strip_prefix("#if ") {
            stack.push(parse_guard_condition(condition));
        } else if let Some(condition) = trimmed.strip_prefix("#elif ") {
            stack.pop();
            stack.push(parse_guard_condition(condition));
        } else if let Some(macro_name) = trimmed.strip_prefix("#ifdef ") {
            stack.push(GuardFrame::Defined(BTreeSet::from([macro_name.trim().to_string()])));
        } else if trimmed.starts_with("#ifndef") {
            stack.push(GuardFrame::Opaque);
        } else if trimmed.starts_with("#else") {
            stack.pop();
            stack.push(GuardFrame::Opaque);
        } else if trimmed.starts_with("#endif") {
            stack.pop();
        }
        lines.push((offset, stack.clone()));
        offset += line.len();
    }

    lines
}

fn active_prefix_macros(
    stack_by_line: &[(usize, Vec<GuardFrame>)],
    byte_offset: usize,
    prefix_upper: &str,
) -> BTreeSet<String> {
    let feature_prefix = format!("{prefix_upper}_FEATURE_");
    let stack = stack_by_line
        .iter()
        .rev()
        .find(|(line_start, _)| *line_start <= byte_offset)
        .map(|(_, stack)| stack.as_slice())
        .unwrap_or(&[]);

    stack
        .iter()
        .filter_map(|frame| match frame {
            GuardFrame::Defined(names) => Some(names.iter().cloned()),
            GuardFrame::Opaque => None,
        })
        .flatten()
        .filter(|name| name.starts_with(&feature_prefix))
        .collect()
}

/// Mirrors the feature-macro derivation in
/// `backends/ffi/gen_bindings/helpers.rs::cbindgen_feature_defines` /
/// `collect_cfg_feature_names`, which this module cannot import (that module
/// owns FFI generation, not the freshness gate — see the ownership split in
/// this crate's generation pipeline). Every `feature = "x"` substring inside
/// a source `#[cfg(...)]` predicate — regardless of whether it sits under
/// `all(...)`, `any(...)`, or `not(...)` — becomes the macro
/// `{PREFIX_UPPER}_FEATURE_{X_UPPER_SNAKE}` that cbindgen's `[defines]` table
/// maps into the header's `#if defined(...)` guard. If that naming rule ever
/// changes in gen_bindings, it must change here too — the two are
/// intentionally duplicated rather than shared, because a freshness *gate*
/// re-deriving its expectation from the generator it's checking would no
/// longer catch a bug in that derivation itself. ~keep
fn required_feature_macros(cfg: &str, prefix_upper: &str) -> BTreeSet<String> {
    const FEATURE_PREFIX: &str = "feature = \"";
    let mut macros = BTreeSet::new();
    let mut remainder = cfg;
    while let Some(start) = remainder.find(FEATURE_PREFIX) {
        let value = &remainder[start + FEATURE_PREFIX.len()..];
        let Some(end) = value.find('"') else { break };
        if end > 0 {
            let feature = &value[..end];
            let macro_suffix: String = feature
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            macros.insert(format!("{prefix_upper}_FEATURE_{macro_suffix}"));
        }
        remainder = &value[end + 1..];
    }
    macros
}

fn format_defined_guard<'a>(macro_names: impl Iterator<Item = &'a String>) -> String {
    macro_names
        .map(|macro_name| format!("defined({macro_name})"))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// For every header-declared function that alef also generated, confirm the
/// header's `#if` guard (if any) matches the source `#[cfg(...)]` (if any).
/// A function with cfg but no matching guard is exactly the original defect
/// — malformed `[defines]` keys silently dropping the guard while every
/// symbol name still lined up. A function with a guard but no source cfg is
/// the symmetric drift: the header is gating something the source no longer
/// restricts. ~keep
fn function_guard_drift(
    exported: &BTreeMap<String, Option<String>>,
    declared_occurrences: &[(usize, String)],
    code: &str,
    prefix_upper: &str,
) -> Vec<String> {
    let stack_by_line = guard_stack_by_line(code);
    let mut problems = Vec::new();

    for (offset, name) in declared_occurrences {
        let Some(cfg) = exported.get(name) else { continue };
        let active = active_prefix_macros(&stack_by_line, *offset, prefix_upper);

        match cfg {
            Some(cfg_text) => {
                let required = required_feature_macros(cfg_text, prefix_upper);
                if required.is_empty() {
                    continue;
                }
                let missing: Vec<&String> = required
                    .iter()
                    .filter(|macro_name| !active.contains(*macro_name))
                    .collect();
                if !missing.is_empty() {
                    let macro_list = format_defined_guard(missing.iter().copied());
                    problems.push(format!(
                        "{name} is exported under #[cfg({cfg_text})] but the header declares it without the \
                         matching #if {macro_list} guard"
                    ));
                }
            }
            None => {
                if !active.is_empty() {
                    let macro_list = format_defined_guard(active.iter());
                    problems.push(format!(
                        "{name} has no #[cfg(...)] in the generated source but the header declares it inside a \
                         #if {macro_list} guard"
                    ));
                }
            }
        }
    }

    problems
}

// --- typedef kind ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TypedefKind {
    Struct,
    Scalar,
}

const SCALAR_TYPEDEF_TYPES: &[&str] = &[
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uintptr_t",
    "intptr_t",
    "size_t",
];

/// Collect `(byte_offset, name, kind)` for every `typedef struct X X;`
/// (opaque forward declaration) or `typedef <fixed-width int> X;` (scalar
/// handle) statement in a comment-stripped header. cbindgen only emits
/// alef's own types in the bare forward-declaration form — never a
/// full struct body — per `gen_bindings/helpers.rs`'s `forward_decls`
/// builder, so a struct-with-fields typedef is deliberately out of scope
/// here; this checker cannot see a field added to or removed from a value
/// struct, only the struct-vs-scalar kind of the typedef itself. ~keep
fn header_typedef_kinds(code: &str) -> Vec<(usize, String, TypedefKind)> {
    let mut found = Vec::new();

    for (offset, _) in code.match_indices("typedef") {
        if offset > 0 && is_identifier_char(code.as_bytes()[offset - 1]) {
            continue;
        }
        let rest = code[offset + "typedef".len()..].trim_start();
        if let Some(name) = parse_struct_typedef(rest) {
            found.push((offset, name, TypedefKind::Struct));
            continue;
        }
        if let Some(name) = parse_scalar_typedef(rest) {
            found.push((offset, name, TypedefKind::Scalar));
        }
    }

    found
}

fn parse_struct_typedef(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix("struct")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let tag: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let rest = rest[tag.len()..].trim_start();
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let rest = rest[name.len()..].trim_start();
    (!tag.is_empty() && tag == name && rest.starts_with(';')).then_some(name)
}

fn parse_scalar_typedef(rest: &str) -> Option<String> {
    for scalar_type in SCALAR_TYPEDEF_TYPES {
        let Some(after_type) = rest.strip_prefix(scalar_type) else {
            continue;
        };
        if !after_type.starts_with(char::is_whitespace) {
            continue;
        }
        let after_type = after_type.trim_start();
        let name: String = after_type
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let after_name = after_type[name.len()..].trim_start();
        if !name.is_empty() && after_name.starts_with(';') {
            return Some(name);
        }
    }
    None
}

const RUST_SCALAR_HANDLE_TYPES: &[&str] = &[
    "u8",
    "u16",
    "u32",
    "u64",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "isize",
    "AlefHandle",
];

/// Source-derived typedef-kind hints, keyed by the *bare* Rust type name
/// (the header name is `{PREFIX_UPPER}{RustName}`, applied by the caller).
///
/// A `#[repr(...)] pub struct Name { .. }` in the generated source is
/// alef's "value-type" binding strategy (IR field `TypeDef::is_opaque ==
/// false`) — it must stay a `typedef struct` in the header. A literal
/// `pub type Name = u64;` (or `= AlefHandle;`) alias is the "opaque-handle"
/// strategy exposed directly as its own name — it must stay a scalar
/// typedef. The common case, an IR type with `is_opaque == true` that is
/// simply boxed through the shared `AlefHandle` registry with no per-type
/// Rust alias at all, produces no hint here — see the module-level blind
/// spots list. ~keep
fn rust_type_kind_hints(source: &str) -> BTreeMap<String, TypedefKind> {
    let mut hints = BTreeMap::new();
    let mut repr_seen = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            if trimmed.starts_with("#[repr(") {
                repr_seen = true;
            }
            continue;
        }
        if let Some(name) = pub_struct_name(trimmed) {
            if repr_seen {
                hints.insert(name, TypedefKind::Struct);
            }
            repr_seen = false;
            continue;
        }
        if let Some(name) = scalar_type_alias_name(trimmed) {
            hints.insert(name, TypedefKind::Scalar);
            repr_seen = false;
            continue;
        }
        repr_seen = false;
    }

    hints
}

fn pub_struct_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub struct ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

fn scalar_type_alias_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub type ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let rest = rest[name.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let value: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let after_value = rest[value.len()..].trim_start();
    let is_known_scalar = RUST_SCALAR_HANDLE_TYPES.contains(&value.as_str());
    let is_scalar_alias = !name.is_empty() && after_value.starts_with(';') && is_known_scalar;
    is_scalar_alias.then_some(name)
}

/// Two independent typedef-kind problems, both reported by name-not-position
/// because a typedef (unlike a function) legitimately repeats across `#if`
/// branches:
///
/// 1. **Internal inconsistency** — the same header typedef name declared as
///    both a struct and a scalar somewhere in the file. A C23
///    `#if __STDC_VERSION__ >= 202311L … #else … #endif` pair that repeats
///    the *same* kind on both branches is correct and never trips this (the
///    two occurrences collapse to one kind in the set); only a genuine
///    kind flip does.
/// 2. **Source/header mismatch** — a name with an unambiguous source-derived
///    expectation (see `rust_type_kind_hints`) whose single header kind
///    doesn't match it. Names already flagged as internally inconsistent are
///    skipped here to avoid reporting the same drift twice. ~keep
fn typedef_kind_problems(
    source_hints: &BTreeMap<String, TypedefKind>,
    header_code: &str,
    prefix_upper: &str,
) -> Vec<String> {
    let occurrences = header_typedef_kinds(header_code);
    let mut kinds_by_name: BTreeMap<&str, BTreeSet<TypedefKind>> = BTreeMap::new();
    for (_, name, kind) in &occurrences {
        kinds_by_name.entry(name.as_str()).or_default().insert(*kind);
    }

    let mut problems = Vec::new();
    for (name, kinds) in &kinds_by_name {
        if kinds.len() > 1 {
            problems.push(format!(
                "{name} is declared as both an opaque struct and a scalar handle in the header (a single \
                 typedef name must not change representation between conditional branches)"
            ));
        }
    }

    for (rust_name, expected_kind) in source_hints {
        let header_name = format!("{prefix_upper}{rust_name}");
        let Some(kinds) = kinds_by_name.get(header_name.as_str()) else {
            continue;
        };
        if kinds.len() == 1 && !kinds.contains(expected_kind) {
            let (actual_description, expected_description) = match expected_kind {
                TypedefKind::Struct => ("a scalar handle", "an opaque struct"),
                TypedefKind::Scalar => ("an opaque struct", "a scalar handle"),
            };
            problems.push(format!(
                "{header_name} is generated from Rust `{rust_name}`, which should stay {expected_description} in \
                 the header, but the header declares it as {actual_description}"
            ));
        }
    }

    problems
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
    fn should_collect_lifetime_bearing_exports() {
        let source = r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sample_node_context_tag_name<'context>(
    context: &'context SampleNodeContext<'context>,
) -> *const std::ffi::c_char {
    std::ptr::null()
}
"#;

        assert_eq!(
            exported_symbols(source),
            BTreeSet::from(["sample_node_context_tag_name".to_owned()])
        );
    }

    #[test]
    fn should_collect_exports_from_service_modules() {
        let directory = tempfile::tempdir().expect("temporary FFI source root");
        std::fs::write(
            directory.path().join("lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_open() {}\n",
        )
        .expect("write root module");
        std::fs::write(
            directory.path().join("service.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_app_register() {}\n",
        )
        .expect("write service module");

        assert_eq!(
            exported_symbols_in_dir(directory.path()).expect("collect module exports"),
            BTreeSet::from(["sample_app_register".to_owned(), "sample_open".to_owned()])
        );
    }

    #[test]
    fn should_collect_cfg_gated_export_alongside_its_predicate() {
        let source = r#"
#[cfg(feature = "document-render")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sample_render() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sample_ping() {}
"#;
        let exports = scan_exported_symbols(source);
        assert_eq!(
            exports.get("sample_render"),
            Some(&Some(r#"feature = "document-render""#.to_owned()))
        );
        assert_eq!(exports.get("sample_ping"), Some(&None));
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
    fn should_collect_exact_symbols_from_wrapped_declarations() {
        let header = r#"
/** sample_node_context_tag_name() and sample_options_default() are documented here. */
const char *sample_node_context_tag_name
(
    const struct SampleNodeContext *context
);

SampleAlefHandle sample_options_default (void);
SampleAlefHandle sample_options_default_value(void);
"#;

        assert_eq!(
            header_declared_functions(header, "sample"),
            BTreeSet::from([
                "sample_node_context_tag_name".to_owned(),
                "sample_options_default".to_owned(),
                "sample_options_default_value".to_owned(),
            ])
        );
    }

    #[test]
    fn should_not_accept_partial_or_commented_symbol_matches() {
        let header = r#"
// void sample_options_default(void);
void prefix_sample_options_default(void);
void sample_options_default_suffix(void);
"#;

        let declared = header_declared_functions(header, "sample");
        assert!(!declared.contains("sample_options_default"));
        assert!(declared.contains("sample_options_default_suffix"));
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

        let message = drift_message(Path::new("include/my_lib.h"), &missing, &removed, &[], &[]);
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

    #[test]
    fn should_refresh_stale_header_before_validating_full_generation() {
        let directory = tempfile::tempdir().expect("temporary project");
        let config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let crate_root = directory.path().join("crates/sample-ffi");
        std::fs::create_dir_all(crate_root.join("src")).expect("create FFI source directory");
        std::fs::create_dir_all(crate_root.join("include")).expect("create FFI include directory");
        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_current() {}\n",
        )
        .expect("write generated FFI source");
        std::fs::write(crate_root.join("include/sample.h"), "void sample_previous(void);\n")
            .expect("write stale FFI header");

        let mut refreshed = false;
        ensure_ffi_header_freshness(&config, directory.path(), || {
            refreshed = true;
            std::fs::write(crate_root.join("include/sample.h"), "void sample_current(void);\n")?;
            Ok(())
        })
        .expect("full generation should refresh before validating");

        assert!(refreshed, "a stale header must trigger the build callback");
    }

    #[test]
    fn should_refresh_header_after_generation_adds_an_export() {
        let directory = tempfile::tempdir().expect("temporary project");
        let config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let crate_root = directory.path().join("crates/sample-ffi");
        std::fs::create_dir_all(crate_root.join("src")).expect("create FFI source directory");
        std::fs::create_dir_all(crate_root.join("include")).expect("create FFI include directory");
        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_create() {}\n\
             #[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_options_default() {}\n",
        )
        .expect("write regenerated FFI source");
        std::fs::write(crate_root.join("include/sample.h"), "void sample_create(void);\n")
            .expect("write header from the preceding generation");

        ensure_ffi_header_freshness(&config, directory.path(), || {
            std::fs::write(
                crate_root.join("include/sample.h"),
                "void sample_create(void);\nvoid sample_options_default(void);\n",
            )?;
            Ok(())
        })
        .expect("generation should refresh a header made stale by its new export");

        check_ffi_header_freshness(&config, directory.path()).expect("refreshed header must match generated source");
    }

    fn sample_config(directory: &Path) -> (ResolvedCrateConfig, PathBuf) {
        let config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let crate_root = directory.join("crates/sample-ffi");
        std::fs::create_dir_all(crate_root.join("src")).expect("create FFI source directory");
        std::fs::create_dir_all(crate_root.join("include")).expect("create FFI include directory");
        (config, crate_root)
    }

    #[test]
    fn should_fail_when_gated_export_is_declared_without_a_guard() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[cfg(feature = \"document-render\")]\n\
             #[unsafe(no_mangle)]\n\
             pub unsafe extern \"C\" fn sample_render() {}\n",
        )
        .expect("write generated FFI source");
        // Reproduces the exact original defect: the symbol name matches, but
        // cbindgen's malformed `[defines]` key dropped the `#if` guard.
        std::fs::write(crate_root.join("include/sample.h"), "void sample_render(void);\n")
            .expect("write unguarded header");

        let error = check_ffi_header_freshness(&config, directory.path())
            .expect_err("an unguarded declaration of a gated export must fail the gate");
        let message = error.to_string();
        assert!(
            message.contains("sample_render"),
            "message must name the drifting symbol, got:\n{message}"
        );
        assert!(
            message.contains("SAMPLE_FEATURE_DOCUMENT_RENDER"),
            "message must name the expected guard macro, got:\n{message}"
        );
    }

    #[test]
    fn should_pass_when_gated_export_is_declared_inside_matching_guard() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[cfg(feature = \"document-render\")]\n\
             #[unsafe(no_mangle)]\n\
             pub unsafe extern \"C\" fn sample_render() {}\n",
        )
        .expect("write generated FFI source");
        std::fs::write(
            crate_root.join("include/sample.h"),
            "#if defined(SAMPLE_FEATURE_DOCUMENT_RENDER)\nvoid sample_render(void);\n#endif\n",
        )
        .expect("write correctly guarded header");

        check_ffi_header_freshness(&config, directory.path()).expect("a correctly guarded export must pass");
    }

    #[test]
    fn should_fail_when_ungated_export_is_declared_inside_a_stray_guard() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_ping() {}\n",
        )
        .expect("write generated FFI source");
        std::fs::write(
            crate_root.join("include/sample.h"),
            "#if defined(SAMPLE_FEATURE_DOCUMENT_RENDER)\nvoid sample_ping(void);\n#endif\n",
        )
        .expect("write header with a stray guard");

        let error = check_ffi_header_freshness(&config, directory.path())
            .expect_err("a stray guard around an ungated export must fail the gate");
        assert!(error.to_string().contains("sample_ping"));
    }

    #[test]
    fn should_pass_when_header_differs_only_by_cbindgen_argument_formatting() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_configure(handle: AlefHandle, value: i32) {}\n",
        )
        .expect("write generated FFI source");
        // `[fn] args = "vertical"` reflow: one argument per line, nothing else changed.
        std::fs::write(
            crate_root.join("include/sample.h"),
            "void sample_configure(\n    uint64_t handle,\n    int32_t value\n);\n",
        )
        .expect("write vertically-reflowed header");

        check_ffi_header_freshness(&config, directory.path())
            .expect("argument reflow alone must not be treated as drift");
    }

    #[test]
    fn should_fail_when_typedef_kind_flips_from_struct_to_scalar() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[repr(C)]\npub struct Visitor {\n    pub field: u32,\n}\n\n\
             #[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_ping() {}\n",
        )
        .expect("write generated FFI source with a value-type struct");
        // The pointer -> scalar-handle migration from the bug report, reproduced
        // in miniature: a value-type struct's typedef becomes a scalar handle.
        std::fs::write(
            crate_root.join("include/sample.h"),
            "typedef uint64_t SAMPLEVisitor;\nvoid sample_ping(void);\n",
        )
        .expect("write header with the migrated typedef kind");

        let error = check_ffi_header_freshness(&config, directory.path())
            .expect_err("a struct-to-scalar typedef kind flip must fail the gate");
        let message = error.to_string();
        assert!(
            message.contains("SAMPLEVisitor"),
            "message must name the drifting type, got:\n{message}"
        );
        assert!(
            message.contains("opaque struct"),
            "message must describe the expected kind, got:\n{message}"
        );
    }

    #[test]
    fn should_fail_on_internal_typedef_kind_inconsistency_without_needing_a_source_hint() {
        let directory = tempfile::tempdir().expect("temporary project");
        let (config, crate_root) = sample_config(directory.path());

        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn sample_ping() {}\n",
        )
        .expect("write generated FFI source");
        std::fs::write(
            crate_root.join("include/sample.h"),
            "#if defined(SOME_OTHER_MACRO)\n\
             typedef struct SAMPLEThing SAMPLEThing;\n\
             #else\n\
             typedef uint64_t SAMPLEThing;\n\
             #endif\n\
             void sample_ping(void);\n",
        )
        .expect("write header with a self-contradicting typedef");

        let error = check_ffi_header_freshness(&config, directory.path())
            .expect_err("a typedef declared as two different kinds must fail the gate");
        assert!(error.to_string().contains("SAMPLEThing"));
    }

    #[test]
    fn should_not_flag_legitimate_c23_dual_declared_typedef() {
        let header = "#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 202311L\n\
                       typedef struct SAMPLEFoo SAMPLEFoo;\n\
                       #else\n\
                       typedef struct SAMPLEFoo SAMPLEFoo;\n\
                       #endif\n";
        let code = strip_c_comments(header);

        let problems = typedef_kind_problems(&BTreeMap::new(), &code, "SAMPLE");
        assert!(
            problems.is_empty(),
            "a same-kind typedef repeated across a C23 __STDC_VERSION__ guard must not be flagged, got: {problems:?}"
        );
    }

    #[test]
    fn required_feature_macros_covers_conjunctions_of_features() {
        let macros = required_feature_macros(r#"all(feature = "document-render", feature = "native")"#, "SAMPLE");
        assert_eq!(
            macros,
            BTreeSet::from([
                "SAMPLE_FEATURE_DOCUMENT_RENDER".to_owned(),
                "SAMPLE_FEATURE_NATIVE".to_owned()
            ])
        );
    }

    #[test]
    fn required_feature_macros_ignores_non_feature_predicates() {
        assert!(required_feature_macros(r#"target_os = "windows""#, "SAMPLE").is_empty());
    }
}
