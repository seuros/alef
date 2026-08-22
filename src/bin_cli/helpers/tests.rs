use super::*;
use crate::core::config::Language;

fn resolved_test_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
command = "pytest"

[crates.test.rust]
e2e = "cargo test"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Seed one stamped file per name and return what the ownership walk actually opened.
fn scanned_names(names: &[&str]) -> Vec<String> {
    let directory = tempfile::tempdir().expect("temporary project");
    for name in names {
        let path = directory.path().join(name);
        let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
    }
    let mut found: Vec<String> = collect_alef_hashes(directory.path())
        .into_iter()
        .filter_map(|(path, _, _)| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

/// Seed one stamped file per relative path (creating parent directories) and return the
/// repository-relative paths the ownership walk actually opened, sorted.
fn scanned_relative_paths(relative_paths: &[&str]) -> Vec<String> {
    let directory = tempfile::tempdir().expect("temporary project");
    for relative in relative_paths {
        let path = directory.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("seeded path has a parent")).expect("seed parent directory");
        let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
    }
    let mut found: Vec<String> = collect_alef_hashes(directory.path())
        .into_iter()
        .filter_map(|(path, _, _)| {
            path.strip_prefix(directory.path())
                .ok()?
                .to_str()
                .map(|value| value.replace('\\', "/"))
        })
        .collect();
    found.sort();
    found
}

/// alef stamps files inside dot-directories (`.cargo/config.toml`, agent-skill `SKILL.md`s)
/// that the walk used to prune wholesale, so those stamps were written and never read: no
/// amount of drift could make `alef verify` report them.
///
/// Paired control in one run, because a walk that finds nothing and a walk that finds
/// everything are the same green: a stamped file in an ordinary directory must be found (the
/// walk works at all), a stamped file in `.cargo` must now also be found (the fix), and a
/// stamped file in `.venv` must still be missed (the prune still keeps the walk out of tool
/// caches — the fix is an allowlist, not a removal).
#[test]
fn the_ownership_walk_reaches_the_dot_directories_alef_stamps() {
    let found = scanned_relative_paths(&[
        "packages/reachable.toml",
        ".cargo/config.toml",
        ".github/skills/api/SKILL.md",
        ".venv/lib/cached.toml",
    ]);

    assert!(
        found.contains(&"packages/reachable.toml".to_string()),
        "control: a stamped file outside every dot-directory must be found, else this test \
         proves nothing about the dot-directory cases; walk returned {found:?}"
    );
    assert!(
        found.contains(&".cargo/config.toml".to_string()),
        "alef writes and stamps `.cargo/config.toml` itself; a stamp nothing ever reads back \
         is not a freshness check. Walk returned {found:?}"
    );
    assert!(
        found.contains(&".github/skills/api/SKILL.md".to_string()),
        "generated agent skills are stamped alef output and must be verifiable; walk returned \
         {found:?}"
    );
    assert!(
        !found.contains(&".venv/lib/cached.toml".to_string()),
        "the dot-directory prune must still keep the walk out of tool caches -- the fix is an \
         allowlist of the dot-directories alef writes into, not a removal of the prune. Walk \
         returned {found:?}"
    );
}

/// A nested git worktree is a second checkout of the same repository. It became reachable the
/// moment `.claude` came off the blanket prune, and walking it reports another branch's
/// stamps as this tree's.
#[test]
fn the_ownership_walk_does_not_descend_into_a_nested_worktree() {
    let found = scanned_relative_paths(&[".claude/skills/api/SKILL.md", ".claude/worktrees/other/config.toml"]);

    assert!(
        found.contains(&".claude/skills/api/SKILL.md".to_string()),
        "control: `.claude` must be walked, else the exclusion below is vacuous; walk \
         returned {found:?}"
    );
    assert!(
        !found.contains(&".claude/worktrees/other/config.toml".to_string()),
        "a nested worktree is a different checkout of this repository; its stamps are not \
         this tree's. Walk returned {found:?}"
    );
}

/// THE AGREEMENT CANARY. `alef verify`'s frozen-file report and `alef adopt`'s candidate
/// set are the report and the remedy for one fact, so a path in one and not the other
/// sends a reader to a command that refuses them. They diverged exactly that way: each
/// was built from its own hand-maintained stage list, adopt's missing service/public API
/// and both missing e2e, test apps, READMEs and docs — which is why `alef adopt` on an
/// e2e snippet glob bailed with "no alef-managed output matches" while 15,677 snippets
/// sat frozen and unreported.
///
/// Asserting on behaviour would need a full extraction + generation pass against a real
/// crate, which is what made the divergence invisible to the test suite in the first
/// place. This asserts on the structure that produced it instead: each consumer derives
/// its set from [`collect_managed_surface`] and enumerates no stage of its own. It fails
/// the moment either one grows a private list again, which is the regression. ~keep
#[test]
fn both_consumers_build_their_managed_set_only_from_the_shared_surface() {
    // Every stage entry point `collect_managed_surface` composes. A consumer that
    // names one inside its own region is re-deriving the surface instead of sharing
    // it. `generate(` carries its parenthesis because `generate_` prefixes several
    // of the others. ~keep
    let stage_calls = [
        "pipeline::generate(",
        "pipeline::generate_stubs(",
        "pipeline::generate_service_api(",
        "pipeline::generate_public_api(",
        "pipeline::scaffold(",
        "pipeline::readme(",
        "e2e::generate_e2e(",
        "docs::generate_docs_stage(",
    ];
    // Each region is the consumer's own code, cut so it excludes the shared collector
    // itself (which must name every stage) and, for `aux_commands`, the unrelated
    // `Commands::Init` arm, which legitimately generates and writes. ~keep
    let regions = [
        (
            "alef verify's frozen report",
            include_str!("../helpers.rs")
                .split("pub(crate) fn collect_managed_surface")
                .next()
                .expect("helpers splits on the shared collector"),
        ),
        (
            "alef adopt's candidate set",
            include_str!("../aux_commands.rs")
                .split("Commands::Adopt {")
                .nth(1)
                .and_then(|rest| rest.split("Commands::Migrate {").next())
                .expect("aux_commands splits on the adopt arm"),
        ),
    ];
    for (name, region) in regions {
        for call in stage_calls {
            assert!(
                !region.contains(call),
                "{name} calls {call} directly -- the frozen report and the candidate set \
                 must not enumerate generation stages separately, or they disagree again"
            );
        }
        assert!(
            region.contains("collect_managed_surface("),
            "{name} must derive its managed set from the shared surface"
        );
    }
}

/// THE CANARY. Every name here is stamped by `marker_header_syntax` on the emit side, so a
/// file alef wrote carries a hash this walk must be able to re-read. Before the list was
/// widened these were stamped and then never opened — which reads as covered rather than as
/// missing, and is why the gap survived its own doc comment's warning. ~keep
#[test]
fn ownership_walk_opens_every_extension_the_emit_side_stamps() {
    assert_eq!(
        scanned_names(&[
            "foo-config.cmake",
            "app.csproj",
            "gem.gemspec",
            "build.zig.zon",
            "pom.xml"
        ]),
        vec![
            "app.csproj",
            "build.zig.zon",
            "foo-config.cmake",
            "gem.gemspec",
            "pom.xml"
        ],
    );
}

/// The makefiles and `Rakefile` have no extension at all, and `go.mod`'s is the far-too-broad
/// `mod` — shared with unrelated binary formats — so all of them are keyed on file name on the
/// emit side and must be keyed the same way here.
///
/// Every entry of `VERIFY_SCAN_FILENAMES` that is not a dotfile appears below, so an addition
/// to that list without a matching read-side check fails here rather than passing quietly.
///
/// `makefile` gets its own directory: macOS and Windows resolve it and `Makefile` to the same
/// path, so seeding both in one directory silently writes a single file and the lowercase
/// entry would look unscanned when it is only unwritten. ~keep
#[test]
fn ownership_walk_opens_the_filename_keyed_files_the_emit_side_stamps() {
    assert_eq!(scanned_names(&["makefile"]), vec!["makefile"]);
    assert_eq!(
        scanned_names(&[
            "Makefile",
            "GNUmakefile",
            "Rakefile",
            "Makevars",
            "Makevars.in",
            "Makevars.win.in",
            "go.mod"
        ]),
        vec![
            "GNUmakefile",
            "Makefile",
            "Makevars",
            "Makevars.in",
            "Makevars.win.in",
            "Rakefile",
            "go.mod"
        ],
    );
}

/// The other half of the predicate: widening the allowlist must not turn the walk into
/// "open everything". Without this, both tests above would still pass if the filter were
/// deleted outright. ~keep
#[test]
fn ownership_walk_still_skips_an_extension_alef_never_stamps() {
    assert!(scanned_names(&["notes.rtf", "archive.tar"]).is_empty());
}

#[test]
fn default_log_level_maps_verbosity_to_levels() {
    assert_eq!(default_log_level(0, false), "info");
    assert_eq!(default_log_level(1, false), "debug");
    assert_eq!(default_log_level(2, false), "trace");
    assert_eq!(default_log_level(9, false), "trace");
    // --quiet wins over any -v count.
    assert_eq!(default_log_level(0, true), "error");
    assert_eq!(default_log_level(3, true), "error");
}

#[test]
fn resolve_test_languages_allows_explicit_test_only_language() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, Some(&["rust".to_string()]), true).unwrap();
    assert_eq!(langs, vec![Language::Rust]);
}

#[test]
fn resolve_test_languages_appends_e2e_only_languages() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, None, true).unwrap();
    assert_eq!(langs, vec![Language::Python, Language::Rust]);
}

#[test]
fn resolve_test_languages_omits_e2e_only_languages_without_e2e() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, None, false).unwrap();
    assert_eq!(langs, vec![Language::Python]);
}

fn gen_file(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: true,
    }
}

#[test]
fn generated_files_match_disk_true_when_bodies_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
    let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
    assert!(generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_ignores_embedded_hash_line() {
    let dir = tempfile::tempdir().unwrap();
    let generated = "// This file is auto-generated by alef — DO NOT EDIT.\npackage x\n\nvar a = 1\n";
    std::fs::write(
        dir.path().join("binding.go"),
        "// This file is auto-generated by alef — DO NOT EDIT.\n// alef:hash:deadbeef\npackage x\n\nvar a = 1\n",
    )
    .unwrap();
    let files = vec![gen_file("binding.go", generated)];
    assert!(generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_false_when_body_differs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
    let files = vec![gen_file("binding.go", "package x\n\nimport \"fmt\"\n\nvar a = 1\n")];
    assert!(!generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_false_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![gen_file("binding.go", "package x\n")];
    assert!(!generated_files_match_disk(&files, dir.path()));
}

fn gen_file_unheadered(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: false,
    }
}

/// The defect this closes: a backend that would produce a file (e.g. one
/// Java/C# file per public type) is invisible to a pure disk walk when the
/// file was never written — `alef verify` must catch that, not just an
/// existing file whose hash drifted.
#[test]
fn missing_managed_paths_reports_an_absent_headered_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    let missing = missing_managed_paths(&files, dir.path());

    assert_eq!(missing, vec![dir.path().join("SomeType.java").display().to_string()]);
}

/// Positive control: an up-to-date tree (every generated path already
/// present on disk) must report nothing missing, regardless of the file's
/// actual content — content drift is `verify_walk`'s job, not this check's.
#[test]
fn missing_managed_paths_reports_nothing_when_every_headered_file_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("SomeType.java"),
        "final class SomeType { /* stale */ }\n",
    )
    .unwrap();
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    assert!(missing_managed_paths(&files, dir.path()).is_empty());
}

/// The required negative control: a legitimately user-owned, unheadered
/// scaffold-once file (`Cargo.toml`, `package.json`, gemspec, lockfiles —
/// see `verify_walk`'s doc comment) that is absent must NOT be reported
/// missing. Getting this wrong would fail verify on every clean repo whose
/// scaffold-once files simply haven't been (re-)generated locally.
#[test]
fn missing_managed_paths_ignores_an_absent_unheadered_scaffold_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = vec![gen_file_unheadered("Cargo.toml", "[package]\nname = \"demo\"\n")];

    assert!(missing_managed_paths(&files, dir.path()).is_empty());
}

#[test]
fn marker_line_finds_the_line_carrying_the_provenance_marker() {
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);

    assert_eq!(
        marker_line(&header),
        Some("// This file is auto-generated by alef — DO NOT EDIT.")
    );
}

#[test]
fn marker_line_finds_nothing_in_content_without_a_marker() {
    assert_eq!(marker_line("final class SomeType {}\n"), None);
}

/// The defect this closes: a pre-existing file at a path alef would emit
/// and mark, but that predates the marker system, deadlocks the write
/// guard forever (see `FrozenFile`'s doc). `alef verify` must surface it
/// even though it never carries a hash to compare, which is why this is a
/// distinct check from `verify_walk`'s stale-hash comparison.
#[test]
fn frozen_managed_paths_reports_an_unmarked_pre_existing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("SomeType.java"), "final class SomeType {}\n").unwrap();
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(frozen.len(), 1);
    assert_eq!(frozen[0].path, dir.path().join("SomeType.java").display().to_string());
    assert_eq!(
        frozen[0].remedy.as_deref(),
        Some("// This file is auto-generated by alef — DO NOT EDIT.")
    );
    assert_eq!(
        frozen[0].near_miss, None,
        "plain hand-written content has no near miss to report"
    );
}

/// A pre-existing file whose leading lines look like a failed attempt at a marker (mentions
/// both "alef" and "generated" without matching `content_has_alef_marker`) is still frozen,
/// but the report should name what's already there, not just what's missing.
#[test]
fn frozen_managed_paths_reports_a_near_miss_when_one_is_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("SomeType.java"),
        "// This alef-generated file should not be edited.\nfinal class SomeType {}\n",
    )
    .unwrap();
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(frozen.len(), 1);
    assert_eq!(
        frozen[0].near_miss.as_deref(),
        Some("// This alef-generated file should not be edited.")
    );
}

/// A managed file that already carries the marker is stale-or-fresh
/// territory (`verify_walk`'s job), never frozen — the guard that would
/// deadlock a write never engages once a marker is present.
#[test]
fn frozen_managed_paths_reports_nothing_when_the_existing_file_already_carries_the_marker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marked = format!(
        "{}final class SomeType {{}}\n",
        crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash)
    );
    std::fs::write(dir.path().join("SomeType.java"), &marked).unwrap();
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    assert!(frozen_managed_paths(&files, dir.path()).is_empty());
}

/// A managed file that does not yet exist is `missing_managed_paths`'
/// territory, not frozen's -- there is nothing on disk to be frozen.
#[test]
fn frozen_managed_paths_reports_nothing_when_the_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    assert!(frozen_managed_paths(&files, dir.path()).is_empty());
}

/// The required negative control: a legitimately user-owned, unmarked
/// scaffold-once file (`Cargo.toml`, `package.json`, gemspec, lockfiles)
/// must never be reported frozen, even though it exists on disk without a
/// marker -- exactly the shape a naive "unmarked file that looks
/// generated" heuristic would misfire on. Getting this wrong would tell
/// users to hand ownership of their own hand-edited files to alef.
#[test]
fn frozen_managed_paths_ignores_a_hand_written_scaffold_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    let files = vec![gen_file_unheadered("Cargo.toml", "[package]\nname = \"demo\"\n")];

    assert!(frozen_managed_paths(&files, dir.path()).is_empty());
}

/// A self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig headers,
/// `docs::render`'s `.md` pages) bakes its literal header straight into
/// `GeneratedFile::content` regardless of `generated_header`. The remedy
/// must be read from that content, not reconstructed from the path -- a
/// path-derived generic header would be the wrong text to hand back here.
#[test]
fn frozen_managed_paths_reads_the_remedy_from_self_marked_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Foo.swift"), "struct Foo {}\n").unwrap();
    let files = vec![gen_file_unheadered(
        "Foo.swift",
        "// Generated by alef. Do not edit by hand.\nstruct Foo {}\n",
    )];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(frozen.len(), 1);
    assert_eq!(
        frozen[0].remedy.as_deref(),
        Some("// Generated by alef. Do not edit by hand.")
    );
}

/// A managed path whose format has no comment syntax at all (`.json`) and carries no
/// `.alef-ownership.toml` record either is still reported frozen when `generated_header`
/// claims ownership, with no literal marker line to hand back -- there is nothing to
/// paste in, and (unlike the paired positive control below) alef genuinely has no proof
/// of authorship for this path yet, so the write guard would refuse it too.
#[test]
fn frozen_managed_paths_reports_no_remedy_for_an_unmarkable_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("manifest.json"), "{}\n").unwrap();
    let files = vec![gen_file("manifest.json", "{}\n")];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(frozen.len(), 1);
    assert_eq!(frozen[0].remedy, None);
}

/// The defect this closes (alef #-frozen-verify-disagreement): a `.json`-style path that
/// cannot carry a marker but that alef *has* durably recorded owning -- exactly what
/// `alef adopt` or a delete-and-regenerate leaves behind -- must not be reported frozen.
/// Before `frozen_managed_paths` consulted `is_owned_by_ownership_record`, this positive
/// control failed identically to the negative control above: the function only ever
/// checked the content marker, so a file the write guard would happily accept stayed
/// "frozen" in `alef verify`'s report forever.
#[test]
fn frozen_managed_paths_reports_nothing_for_an_unmarkable_extension_with_a_committed_ownership_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("manifest.json");
    std::fs::write(&path, "{}\n").unwrap();
    crate::cli::cache::record_scaffold_owned_path(dir.path(), &path).expect("record ownership");
    let files = vec![gen_file("manifest.json", "{}\n")];

    assert!(
        frozen_managed_paths(&files, dir.path()).is_empty(),
        "a path the committed ownership record already proves alef owns must agree with \
         the write guard, which would happily overwrite it"
    );
}

/// Defect B: `.clang-format` is YAML underneath (`#` line comments), so once
/// `marker_header_syntax` recognizes it by file name, a pre-existing unmarked copy must
/// get a real, pasteable remedy -- not the "no comment syntax" message that is only true
/// for genuinely unmarkable formats like `.json`/`DESCRIPTION`. Reporting an impossible
/// remedy for a format that can actually carry one is exactly the failure mode this
/// closes.
#[test]
fn frozen_managed_paths_offers_a_real_remedy_for_clang_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".clang-format"), "---\nBasedOnStyle: LLVM\n").unwrap();
    let files = vec![gen_file(".clang-format", "---\nBasedOnStyle: LLVM\n")];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(frozen.len(), 1);
    assert_eq!(
        frozen[0].remedy.as_deref(),
        Some("# This file is auto-generated by alef — DO NOT EDIT.")
    );
}

/// Defect: `carries_alef_marker()` is `generated_header || content_has_alef_marker`, so a
/// `GeneratedFile` with `generated_header: false` whose content embeds no marker at all --
/// exactly the PHP backend's `config.m4` (`generate_config_m4`,
/// `src/backends/php/gen_bindings/rust_items.rs`, emitted with `generated_header: false`
/// alongside `.m4` content that carries no alef marker text) -- is filtered out by
/// `managed_generated_files` before `frozen_managed_paths` ever runs its own
/// ownership-record fallback (`is_owned_by_ownership_record`) over it. The write guard
/// (`write_files_report`, `src/cli/pipeline/generate/write.rs`) still refuses to overwrite
/// such a path once it exists without a committed ownership record -- so this was a real
/// write refusal `alef generate` reports but `alef verify` had no way to see, because the
/// candidate never reached the frozen check at all. ~keep
#[test]
fn frozen_managed_paths_reports_an_unmarkable_generated_header_false_file_with_no_ownership_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("config.m4"), "dnl old content\n").unwrap();
    let files = vec![gen_file_unheadered(
        "config.m4",
        "dnl Configuration for Rust-based PHP extension via ext-php-rs.\n",
    )];

    let frozen = frozen_managed_paths(&files, dir.path());

    assert_eq!(
        frozen.len(),
        1,
        "an unmarkable, generated_header: false, unmarked-content file the write guard would \
         refuse must be surfaced by alef verify too"
    );
    assert_eq!(frozen[0].path, dir.path().join("config.m4").display().to_string());
    assert_eq!(
        frozen[0].remedy, None,
        "`.m4` has no comment syntax alef stamps, so there is no literal marker line to hand back"
    );
}

/// The paired positive control: once `.alef-ownership.toml` durably records this exact
/// path -- what `alef generate`'s writer itself does on the run that first authors it
/// (`write_files_report`'s `record_scaffold_owned_path` call) -- the write guard would
/// happily accept the file again, so `alef verify` must agree and stay silent.
#[test]
fn frozen_managed_paths_reports_nothing_for_an_unmarkable_generated_header_false_file_with_a_committed_ownership_record()
 {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.m4");
    std::fs::write(&path, "dnl old content\n").unwrap();
    crate::cli::cache::record_scaffold_owned_path(dir.path(), &path).expect("record ownership");
    let files = vec![gen_file_unheadered(
        "config.m4",
        "dnl Configuration for Rust-based PHP extension via ext-php-rs.\n",
    )];

    assert!(
        frozen_managed_paths(&files, dir.path()).is_empty(),
        "a path the committed ownership record already proves alef owns must agree with the \
         write guard, which would happily overwrite it"
    );
}

#[test]
fn verify_walk_detects_an_edited_generated_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("binding.rs");
    let inputs_hash = "generation-inputs";
    let original = "// This file is auto-generated by alef — DO NOT EDIT.\nfn value() -> u8 { 1 }\n";
    let hash = crate::core::hash::compute_file_hash(inputs_hash, original);
    let finalized = crate::core::hash::inject_hash_line(original, &hash);
    std::fs::write(&path, finalized.replace("{ 1 }", "{ 2 }")).expect("edit generated file");

    let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, path.display().to_string());
}

/// Regression coverage for a hole reported against `alef verify`: "the inputs-hash
/// covers generation inputs, not output bytes, so a dependency bumped inside a
/// stamped, alef-generated manifest reports fresh." That does not hold for markable,
/// stamped files -- `compute_file_hash` folds the file's own content into the embedded
/// hash (see its doc and `core::hash`'s module doc), so a hand-edited dependency
/// version -- e.g. `cargo upgrade --incompatible` bumping `base64` in place inside a
/// generated JNI/FFI `Cargo.toml` -- is exactly the "content changed, inputs did not"
/// case this test pins down: `inputs_hash` is identical before and after, only the
/// on-disk bytes move. If `compute_file_hash`/`verify_walk` ever regress to hashing
/// `inputs_hash` alone, this must start failing. ~keep
#[test]
fn verify_walk_detects_a_hand_edited_dependency_version_in_a_generated_manifest() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("Cargo.toml");
    let inputs_hash = "generation-inputs";
    let original = "# This file is auto-generated by alef — DO NOT EDIT.\n\
                     [dependencies]\n\
                     base64 = \"0.22\"\n";
    let hash = crate::core::hash::compute_file_hash(inputs_hash, original);
    let finalized = crate::core::hash::inject_hash_line(original, &hash);
    // Same generation inputs throughout -- only the on-disk bytes are hand-edited,
    // as `cargo upgrade --incompatible` would do to a generated manifest.
    std::fs::write(&path, finalized.replace("0.22", "0.23")).expect("hand-edit generated manifest");

    let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

    assert_eq!(
        stale.len(),
        1,
        "a hand-edited dependency version must be reported stale even though inputs_hash is unchanged"
    );
    assert_eq!(stale[0].path, path.display().to_string());
}

#[test]
fn verify_walk_detects_a_mixed_stamped_and_unstamped_generated_tree() {
    let directory = tempfile::tempdir().expect("tempdir");
    let inputs_hash = "generation-inputs";
    let path = directory.path().join("unstamped.rs");
    std::fs::write(
        &path,
        "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
    )
    .expect("write generated file");
    let stamped_path = directory.path().join("stamped.rs");
    let stamped_body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn stamped() {}\n";
    let stamped_hash = crate::core::hash::compute_file_hash(inputs_hash, stamped_body);
    std::fs::write(
        &stamped_path,
        crate::core::hash::inject_hash_line(stamped_body, &stamped_hash),
    )
    .expect("write stamped generated file");

    let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, path.display().to_string());
    assert_eq!(stale[0].embedded, "<missing>");
}

/// `find_stamp_disagreement` walks `collect_alef_hashes`, which only yields files that
/// carry an `alef:hash:` line — so a fixture bearing only a stamp is invisible to it and
/// every assertion over it passes vacuously. Both lines must be injected, in that order,
/// to produce a file shaped like one a backend actually emits. ~keep
fn write_stamped(dir: &std::path::Path, name: &str, key: &str, value: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n";
    let stamped = crate::core::hash::inject_stamp_line(body, key, value);
    let hash = crate::core::hash::compute_file_hash("test-inputs-hash", &stamped);
    std::fs::write(&path, crate::core::hash::inject_hash_line(&stamped, &hash)).expect("write stamped file");
    path
}

/// Guards the fixture itself, because the bug this replaces was a fixture bug, not a
/// logic bug: if `write_stamped` ever stops producing a file the hash walk can see, the
/// disagreement tests below go quietly vacuous instead of failing. ~keep
#[test]
fn write_stamped_produces_a_file_the_hash_walk_actually_collects() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "1");

    let collected = collect_alef_hashes(dir.path());
    assert_eq!(
        collected.len(),
        1,
        "the stamped fixture must be visible to the hash walk"
    );
    assert_eq!(
        crate::core::hash::extract_stamp(&collected[0].2, "handle-abi").as_deref(),
        Some("1"),
        "the stamp must survive alongside the hash line"
    );
}

/// The concrete cross-artifact ABI straddle this closes: an FFI-side file
/// stamped for one ABI generation coexisting with a binding-side file
/// stamped for a different one must be reported, even though each file's
/// own `alef:hash:` may be perfectly fresh relative to current inputs.
#[test]
fn find_stamp_disagreement_reports_two_distinct_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "1");
    write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

    let disagreement =
        find_stamp_disagreement(dir.path(), "handle-abi").expect("two distinct stamp values must be reported");

    assert_eq!(disagreement.key, "handle-abi");
    assert_eq!(disagreement.examples.len(), 2);
    let values: Vec<&str> = disagreement.examples.iter().map(|(_, v)| v.as_str()).collect();
    assert!(values.contains(&"1"));
    assert!(values.contains(&"2"));
}

/// Positive control: every stamped file agreeing must not be reported —
/// this is the healthy, fully-regenerated-together state.
#[test]
fn find_stamp_disagreement_is_none_when_every_stamped_file_agrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "2");
    write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

/// The required negative control for the rollout gap the task describes:
/// a tree where no backend has started emitting the stamp yet (every
/// consumer repo today) must not be reported as disagreeing — there is
/// nothing to compare, not a proven mismatch.
#[test]
fn find_stamp_disagreement_is_none_when_nothing_is_stamped() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("header.h"),
        "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
    )
    .expect("write unstamped file");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

/// A file stamped under a different key must not be mistaken for a
/// `handle-abi` disagreement — `find_stamp_disagreement` is keyed, not a
/// blanket "does this file have any stamp" check.
#[test]
fn find_stamp_disagreement_ignores_a_different_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "some-other-marker", "1");
    write_stamped(dir.path(), "binding.zig", "some-other-marker", "2");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

fn stage_failure(paths: &[&str]) -> StageFailure {
    StageFailure {
        stage: "e2e",
        message: "56 e2e assertion(s) reference a field the availability oracle cannot resolve".to_owned(),
        paths: paths.iter().map(std::path::PathBuf::from).collect(),
    }
}

/// THE REGRESSION. `alef adopt packages/dart/rust/Cargo.toml` deadlocked because a
/// pending e2e strict-assertion failure aborted `collect_managed_surface` before the
/// ownership-only `Cargo.toml` target was ever considered, even though that target
/// has no relationship to e2e. `affects_any` is the predicate that now lets `Commands::Adopt`
/// tell the two cases apart: this asserts the tolerant half -- an e2e failure whose
/// rendered paths are all snippet/test-app output must not be judged to affect an
/// unrelated `Cargo.toml` target, whatever glob shape the operator typed. ~keep
#[test]
fn a_stage_failure_confined_to_e2e_paths_does_not_affect_an_unrelated_ownership_target() {
    let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

    assert!(!failure.affects_any(&["packages/dart/rust/Cargo.toml".to_owned()]));
    assert!(!failure.affects_any(&["packages/**/*.gemspec".to_owned()]));
}

/// The control for the test above: when a requested target genuinely falls under the
/// failing stage's own output, `affects_any` must say so, literal path or glob alike,
/// so `alef adopt` still refuses to answer for a target it cannot render correctly
/// rather than silently tolerating every e2e failure regardless of relevance.
#[test]
fn a_stage_failure_that_rendered_the_requested_target_does_affect_it() {
    let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

    assert!(failure.affects_any(&["e2e/python/test_smoke.py".to_owned()]));
    assert!(failure.affects_any(&["e2e/python/*.py".to_owned()]));
    // Mixed: one target unrelated, one that matches -- still affects, because a
    // multi-target `alef adopt` run answers for every target it was given. ~keep
    assert!(failure.affects_any(&[
        "packages/dart/rust/Cargo.toml".to_owned(),
        "e2e/go/smoke_test.go".to_owned(),
    ]));
}

/// `alef verify` passes no targets at all -- every tolerated failure is unconditional
/// debt for a read-only report, never excused by "no target asked for it". An empty
/// `targets` slice must therefore never affect anything, which is the same fact
/// `Commands::Adopt` relies on for a target list that turned out to filter down to
/// nothing upstream.
#[test]
fn a_stage_failure_never_affects_an_empty_target_list() {
    let failure = stage_failure(&["e2e/python/test_smoke.py"]);

    assert!(!failure.affects_any(&[]));
}

fn swift_only_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// THE REGRESSION (latent): `alef verify` never runs post-build steps
/// (`complete_generated_artifacts` is `Commands::Generate`/`Commands::All`-only), so a path a
/// post-build step owns unguarded -- see `PostBuildStep::owned_paths` -- can never appear in
/// `collect_managed_surface`'s in-memory surface. Left out of `managed_paths`, that path would
/// misreport as an orphan on every single `alef verify` run the moment such a step writes an
/// alef-marked file there. Swift's `MaterializeSwiftBridge` is the real post-build step this
/// exercises: `SwiftBridgeCore.swift` is a path it owns (`PostBuildStep::owned_paths`) but that
/// `collect_managed_surface`'s in-band bindings stage never emits as a `GeneratedFile` -- see
/// `emit_swift_bridge_files`'s doc, which reads real `target/` build output only when called
/// from the post-build step, never from `alef generate`'s own in-memory render. No shipped
/// backend's `owned_paths` output actually carries an alef marker today (this file's own writer
/// never headers it), which is why this is latent rather than a live false positive -- but a
/// marked file at this exact path must still resolve as owned once one does. ~keep
#[test]
fn a_post_build_owned_path_not_produced_in_band_is_not_reported_as_an_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = swift_only_config();
    let api = crate::core::ir::ApiSurface::default();
    let config_path = dir.path().join("alef.toml");

    // Matches `SwiftBackend::build_config_with_config`'s `package_root` fallback when no prior
    // build has populated `<package_root>/Sources` yet: `Sources/RustBridge/SwiftBridgeCore.swift`
    // directly under `base_dir`.
    let owned_path = dir.path().join("Sources/RustBridge/SwiftBridgeCore.swift");
    std::fs::create_dir_all(owned_path.parent().unwrap()).expect("create Sources/RustBridge");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let marked = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&owned_path, marked).expect("write post-build-owned file");

    let found = find_missing_and_frozen_generated_files(&[Language::Swift], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed over a swift-only crate");

    assert!(
        found.managed_paths.contains(&owned_path),
        "post-build-owned paths must be folded into the managed surface: {:?}",
        found.managed_paths
    );

    let orphans = super::super::verify_orphans::find_orphaned_generated_files(dir.path(), &found.managed_paths);
    assert!(
        orphans.is_empty(),
        "a path a post-build step owns unguarded must never be reported as an orphan just \
         because `alef verify` cannot run that step itself: {orphans:?}"
    );
}
