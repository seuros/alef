//! Which files `alef verify`'s ownership walk actually opens, and how much of the tree that
//! leaves unexamined.
//!
//! Split out of `helpers.rs` rather than added to it: that file sits at this repository's
//! 1,000-line cap, and "the scan set of the ownership walk" is a self-contained concern -- the
//! two allowlists, the directory prune, the walk itself, and the coverage tally the walk
//! produces all change together and for the same reason. ~keep

/// Build/cache directories the verify walk never descends into.
const VERIFY_SKIP_DIRS: &[&str] = &[
    ".git",
    ".alef",
    "target",
    "node_modules",
    "_build",
    "deps",
    "parsers",
    "dist",
    "dist-node",
    "vendor",
    ".venv",
    ".cache",
    ".remote-cache",
    "__pycache__",
    "build",
    "tmp",
    "out",
    ".idea",
    ".vscode",
    // A nested git worktree (`git worktree add .claude/worktrees/<name>`) is a second, complete
    // checkout of the same repository. Walking it would report another branch's stamps as this
    // tree's, and it only became reachable once `.claude` was taken off the blanket dot-directory
    // prune below. A worktree's `.git` is a FILE, not a directory, so the `.git` entry above does
    // not stop the descent. ~keep
    "worktrees",
];

/// Dot-directories [`collect_alef_hashes`] descends into despite its blanket dot-directory prune.
///
/// The prune exists to keep the walk out of tool caches, but it is a proxy — "starts with a dot"
/// is not "is a cache" — and alef writes stamped, alef-owned output into several dot-directories:
/// `.cargo/config.toml` from [`crate::scaffold::scaffold`], and every `SKILL.md` under an agent
/// skills root. Those files were stamped and then never read back: the walk pruned their parent
/// before opening them, so `alef verify` could not report them stale no matter how far they
/// drifted. Refusing to *stamp* them instead would be worse — the stamp is also what makes poly's
/// built-in generated-file skip leave them alone, so unstamping them hands their formatting to
/// poly and their staleness to nobody.
///
/// Incomplete by construction, and knowingly so: skills roots are pure configuration
/// (`DocsSkillsConfig::outputs`), so a consumer that writes skills into a dot-directory not named
/// here is still invisible to the walk. Closing that fully requires the walk to consult the
/// resolved config, which it currently has no access to. ~keep
const VERIFY_SCAN_DOT_DIRS: &[&str] = &[
    ".cargo", ".github",
    // Agent-skill roots observed in consumer ownership records (`.alef-ownership.toml` lists
    // `.agents/skills/*/SKILL.md` and `.claude/skills/*/SKILL.md`), so these are not speculative:
    // alef is already writing stamped `SKILL.md` files under them. ~keep
    ".agents", ".claude", ".codex", ".cursor", ".gemini",
];

/// Extensions the ownership walk will open. A generated file whose extension is absent here is
/// invisible to `alef verify` entirely — not reported stale, not reported missing, and not
/// visible to [`super::helpers::find_stamp_disagreement`] either.
///
/// This list is only ONE of two filters. [`collect_alef_hashes`] needs a scanned extension AND
/// an `alef:hash:` line, so adding an extension does nothing for a language whose emitted files
/// carry no stamp at all — measured in a consumer repo, `packages/java` and `packages/go` had
/// ZERO stamped files while `java`/`go` were already listed here. Those are unreachable by any
/// extension change; see the task tracking per-file stamping.
///
/// Scope of what a passing verify proves, because "verify passed" reads as the stronger claim
/// downstream: the hash covers generation INPUTS, not output bytes. One stamped manifest per
/// crate therefore detects input drift for that crate's outputs even when the outputs are
/// unstamped — but a hand-edit to an emitted file leaves inputs untouched and still verifies
/// fresh. Demonstrated in tslp: a dependency bumped inside a stamped, alef-generated
/// `Cargo.toml` reports fresh while the committed bytes differ from what alef would emit.
/// Freshness means the inputs have not moved, not that the file is what the generator writes.
///
/// `zig`/`dart`/`kt`/`kts`/`swift`/`gleam` were missing, which meant the cross-artifact straddle
/// gate could not see the zig side of a zig-vs-FFI-header straddle — the exact artifact pair it
/// exists to protect. `properties`/`pro`/`sh`/`props` were also stamped-but-unscanned, and
/// `packages/csharp/Directory.Build.props` is the ONLY stamped file in that whole package, so
/// csharp's freshness claim rested entirely on a file this walk never opened. Any new emitting
/// backend must add its extension here or its output silently leaves the
/// freshness claim.
///
/// This list must stay a **superset** of everything
/// [`crate::cli::pipeline::generate::write::marker_header_syntax`] can stamp. The walk filters on
/// extension *before* it reads any content, so an unlisted extension is invisible no matter what
/// marker the file carries. `xml`/`csproj`/`zon`/`cmake`/`gemspec` were added to that emit table
/// while missing here, which made their freshness claim unverifiable rather than merely
/// unverified — a stamped file nothing ever checks. ~keep
const VERIFY_SCAN_EXTENSIONS: &[&str] = &[
    "rs",
    "py",
    "pyi",
    "ts",
    "tsx",
    "js",
    "mjs",
    "cjs",
    "rb",
    "rbs",
    "php",
    "phpstub",
    "go",
    "java",
    "cs",
    "ex",
    "exs",
    "R",
    "r",
    "toml",
    "json",
    "md",
    "h",
    "c",
    "yaml",
    "yml",
    "zig",
    "dart",
    "kt",
    "kts",
    "swift",
    "gleam",
    "properties",
    "pro",
    "sh",
    "props",
    "xml",
    "csproj",
    "zon",
    "cmake",
    "gemspec",
];

/// Dotfiles alef stamps that [`VERIFY_SCAN_EXTENSIONS`] structurally cannot reach: `Path::extension`
/// returns `None` for a name that is entirely a leading-dot stem, so `.gitignore` has no extension
/// to match and would stay invisible no matter what is added to that list. Matched on the whole
/// file name instead.
///
/// Extensionless *stamped* files belong here for the same structural reason, not just dotfiles:
/// `Makefile`, `Rakefile` and `Makevars*` carry a `#` marker but have no extension to match, and
/// `go.mod` is matched by name rather than by its `mod` extension deliberately — `.mod` is shared
/// with unrelated binary formats (Fortran module files, tracker music), so listing the extension
/// would pull those into the walk. ~keep
const VERIFY_SCAN_FILENAMES: &[&str] = &[
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    "Makefile",
    "GNUmakefile",
    "makefile",
    "go.mod",
    "Rakefile",
    "Makevars",
    "Makevars.in",
    "Makevars.win.in",
];

/// Walk `base_dir` and return every alef-owned file paired with its optional
/// `alef:hash:<hex>` stamp. Skips build/cache directories, every directory git considers
/// ignored (see [`super::verify_gitignore::gitignored_dirs`]), and files without the Alef
/// ownership marker. Shared by [`super::helpers::verify_walk`] and [`super::helpers::verify_walk_multi`] so both see the same
/// file set, and by [`super::verify_orphans::find_orphaned_generated_files`] so the orphan
/// check sees the identical disk-side file set too. ~keep
pub(crate) fn collect_alef_hashes(base_dir: &std::path::Path) -> Vec<(std::path::PathBuf, Option<String>, String)> {
    let mut found = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];
    // A gitignored dependency-fetch cache (a zig package manager's local package cache) or
    // build-output directory (wasm-pack's own `pkg/`, which copies the crate's real,
    // alef-marked README into a tree this walk otherwise has no reason to open) is neither
    // this run's generated output nor its generation input -- it must never be read as either.
    // See that module's doc for the incident this closes. ~keep
    let gitignored_dirs = super::verify_gitignore::gitignored_dirs(base_dir);

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let pruned_as_dotfile = name.starts_with('.') && !VERIFY_SCAN_DOT_DIRS.contains(&name);
                let pruned_as_gitignored = path
                    .strip_prefix(base_dir)
                    .is_ok_and(|relative| gitignored_dirs.contains(relative));
                if VERIFY_SKIP_DIRS.contains(&name) || pruned_as_dotfile || pruned_as_gitignored {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name_ok = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| VERIFY_SCAN_FILENAMES.contains(&n));
            let ext_ok = name_ok
                || path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        VERIFY_SCAN_EXTENSIONS
                            .iter()
                            .any(|allowed| allowed.eq_ignore_ascii_case(e))
                    })
                    .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if crate::core::hash::content_has_alef_marker(&content) {
                found.push((path, crate::core::hash::extract_hash(&content), content));
            }
        }
    }
    found
}
