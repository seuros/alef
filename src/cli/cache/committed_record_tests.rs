//! Unit tests for the committed provenance records: the ownership manifest and the
//! TOML-merge provenance manifest, plus the `untracked_required_records` guard that exists
//! because each of them degrades silently when left uncommitted.
//!
//! Split out of `cache/tests.rs`, which crossed the 1,000-line cap. The boundary is the
//! concept one: cache keys and output manifests above, committed records here.

use super::*;
use std::path::Path;

/// Initialise a git work tree in `base_dir`, or `None` when git is unavailable.
///
/// Nothing here commits: `git ls-files --error-unmatch` answers from the index, so `git
/// add` alone is enough to make a path tracked for the purpose under test. ~keep
fn init_git_work_tree(base_dir: &Path) -> Option<()> {
    let status = crate::test_support::git_command(base_dir)
        .args(["init", "--quiet"])
        .status()
        .ok()?;
    status.success().then_some(())
}

fn git_add(base_dir: &Path, relative: &str) {
    let status = crate::test_support::git_command(base_dir)
        .args(["add", "--", relative])
        .status()
        .expect("git add");
    assert!(status.success(), "git add {relative} failed");
}

/// THE new regression: alef writes `.alef-ownership.toml`, depends on it for every
/// unmarkable file it is allowed to rewrite, tells the reader inside the file to commit
/// it -- and never once notices that nobody did. A run is then green only because of a
/// file no other checkout has, and a fresh clone or CI refuses everything the record
/// vouches for. The condition has to be observable from outside alef, which is what this
/// query is for. ~keep
#[test]
fn untracked_required_records_reports_a_record_git_does_not_track() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    if init_git_work_tree(base).is_none() {
        return;
    }
    record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");

    assert_eq!(
        untracked_required_records(base),
        vec![OWNERSHIP_MANIFEST],
        "a record alef just created and now depends on must be reported as untracked"
    );
}

/// The other half: once the operator stages it, the condition is gone and must stop
/// being reported. A check that fires unconditionally is a check nobody reads, which is
/// how the original one-shot notice failed in the first place. ~keep
#[test]
fn untracked_required_records_is_silent_once_the_record_is_staged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    if init_git_work_tree(base).is_none() {
        return;
    }
    record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");
    git_add(base, OWNERSHIP_MANIFEST);

    assert!(
        untracked_required_records(base).is_empty(),
        "a staged record is tracked; reporting it anyway trains the operator to ignore the warning"
    );
}

/// Never cry wolf. Outside a git work tree "untracked" is not a defect, it is a
/// question with no answer -- an export tarball, a vendored copy, a container with no
/// git. Reporting there would fire on every such run forever with nothing the operator
/// could do about it. ~keep
#[test]
fn untracked_required_records_is_silent_outside_a_git_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");
    assert!(base.join(OWNERSHIP_MANIFEST).is_file(), "sanity: the record exists");

    assert!(
        untracked_required_records(base).is_empty(),
        "with no repository to ask, tracked-ness is unanswerable and must not be reported as a fault"
    );
}

/// A record alef has never had reason to write is not a hidden dependency, so an empty
/// repository must stay quiet. ~keep
#[test]
fn untracked_required_records_ignores_a_record_that_does_not_exist_yet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    if init_git_work_tree(base).is_none() {
        return;
    }

    assert!(untracked_required_records(base).is_empty());
}

#[test]
fn scaffold_owned_path_round_trips_and_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let target = base.join("packages/java/pom.xml");

    assert!(!is_scaffold_owned_path(base, &target), "must start unrecorded");

    record_scaffold_owned_path(base, &target).expect("record");
    record_scaffold_owned_path(base, &target).expect("record again (idempotent)");

    assert!(is_scaffold_owned_path(base, &target));
    let manifest = std::fs::read_to_string(base.join(OWNERSHIP_MANIFEST)).expect("read manifest");
    assert_eq!(
        manifest.matches("packages/java/pom.xml").count(),
        1,
        "recording the same path twice must not duplicate it, got:\n{manifest}"
    );
    assert!(
        !base.join(".alef").join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST).exists(),
        "the gitignored legacy record must no longer be written, got:\n{manifest}"
    );
}

/// The batch entry point must be indistinguishable in outcome from the per-path
/// one — same entries, same order, same idempotence, and existing entries left
/// alone — because it exists purely to collapse N manifest parses into one for a
/// bulk `alef adopt`. If it ever diverges in *result*, the fast path is silently
/// recording something different from what the reviewed path would have. ~keep
#[test]
fn batch_recording_matches_per_path_recording_entry_for_entry() {
    let batched = tempfile::tempdir().expect("tempdir");
    let one_at_a_time = tempfile::tempdir().expect("tempdir");
    let relatives = [
        "docs/snippets/python/api/z.md",
        "packages/node/package.json",
        "docs/snippets/python/api/a.md",
        "packages/java/pom.xml",
    ];

    record_scaffold_owned_path(batched.path(), &batched.path().join("pre/existing.json")).expect("seed");
    record_scaffold_owned_path(one_at_a_time.path(), &one_at_a_time.path().join("pre/existing.json"))
        .expect("seed");

    let joined: Vec<PathBuf> = relatives.iter().map(|rel| batched.path().join(rel)).collect();
    let refs: Vec<&Path> = joined.iter().map(PathBuf::as_path).collect();
    record_scaffold_owned_paths(batched.path(), &refs).expect("batch record");
    record_scaffold_owned_paths(batched.path(), &refs).expect("batch record again (idempotent)");
    for relative in relatives {
        record_scaffold_owned_path(one_at_a_time.path(), &one_at_a_time.path().join(relative)).expect("record");
    }

    assert_eq!(
        std::fs::read_to_string(batched.path().join(OWNERSHIP_MANIFEST)).expect("batched manifest"),
        std::fs::read_to_string(one_at_a_time.path().join(OWNERSHIP_MANIFEST)).expect("sequential manifest"),
    );
    for relative in relatives {
        assert!(is_scaffold_owned_path(batched.path(), &batched.path().join(relative)));
    }
    assert!(
        is_scaffold_owned_path(batched.path(), &batched.path().join("pre/existing.json")),
        "a batch must extend the record, never replace it"
    );
}

/// The record must be a file `git add` picks up, not one alef itself
/// gitignores. `ensure_gitignore` writes `.alef/` into every consumer's
/// `.gitignore` (`cli::pipeline::extract::gitignore`), so a record stored
/// under that directory can never travel with the commit it describes --
/// which is the entire #80 reproducibility hole. Asserting the location and
/// the parseability together, because a committed file nobody can parse is
/// worth no more than an ignored one. ~keep
#[test]
fn ownership_record_lives_outside_the_gitignored_cache_and_is_valid_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();

    record_scaffold_owned_path(base, &base.join("packages/typescript/package.json")).expect("record");

    let manifest_path = base.join(OWNERSHIP_MANIFEST);
    assert!(manifest_path.exists(), "the record must exist at the repo root");
    assert!(
        !manifest_path.starts_with(base.join(CACHE_DIR)),
        "the record must not live under the gitignored `{CACHE_DIR}` directory"
    );
    let content = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let parsed: OwnershipManifest = toml::from_str(&content).expect("the record must be valid TOML");
    assert_eq!(parsed.owned_paths, vec!["packages/typescript/package.json".to_owned()]);
}

/// A fresh clone carries the committed record but no `.alef/` cache at all.
/// Simulated by recording into one `base_dir` and reading the manifest back
/// from a second, cache-less one -- the machine-local half of the answer is
/// absent there by construction, so a `true` can only have come from the
/// committed file.
#[test]
fn committed_record_answers_identically_on_a_cache_less_clone() {
    let warm = tempfile::tempdir().expect("tempdir warm");
    let clone = tempfile::tempdir().expect("tempdir clone");
    let relative = std::path::Path::new("packages/typescript/package.json");

    record_scaffold_owned_path(warm.path(), &warm.path().join(relative)).expect("record");
    std::fs::copy(
        warm.path().join(OWNERSHIP_MANIFEST),
        clone.path().join(OWNERSHIP_MANIFEST),
    )
    .expect("check out the committed record");

    assert!(
        !clone.path().join(CACHE_DIR).exists(),
        "the simulated clone must have no machine-local cache"
    );
    assert!(
        is_scaffold_owned_path(clone.path(), &clone.path().join(relative)),
        "a fresh clone must agree with the warm machine about what alef owns"
    );
}

/// One bad hand-edit must cost the edit, not the record. `record_scaffold_owned_paths`
/// rewrites the manifest whole from what it read back, so before this was fixed an
/// unparseable line made the read return an empty `Vec` and the write persist only the current
/// batch -- silently un-owning every path recorded before it, in a committed file.
///
/// The assertion is on the bytes on disk after the call, because that is what is destroyed.
/// A test on the error text alone would pass just as well against code that emitted the
/// message and then truncated the file anyway. ~keep
#[test]
fn malformed_ownership_record_refuses_rather_than_dropping_recorded_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    record_scaffold_owned_path(base, &base.join("packages/java/pom.xml")).expect("seed the record");

    let manifest_path = base.join(OWNERSHIP_MANIFEST);
    let seeded = std::fs::read_to_string(&manifest_path).expect("read the seeded record");
    let corrupted = format!("{seeded}this line is not toml\n");
    std::fs::write(&manifest_path, &corrupted).expect("hand-edit the record into invalid TOML");

    let newly_scaffolded = base.join("packages/node/package.json");
    let error = record_scaffold_owned_paths(base, &[newly_scaffolded.as_path()])
        .expect_err("recording against an unreadable record must fail rather than rewrite it");

    assert_eq!(
        std::fs::read_to_string(&manifest_path).expect("read the record after the refusal"),
        corrupted,
        "the refused run must leave the record byte-identical, keeping every recorded path"
    );
    assert!(
        error.to_string().contains(OWNERSHIP_MANIFEST),
        "the failure must name the file the operator has to repair, got: {error}"
    );
}

/// An unparseable record must read as "alef owns nothing" rather than
/// panicking or, far worse, being treated as ownership of everything.
#[test]
fn unparseable_ownership_record_claims_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(base.join(OWNERSHIP_MANIFEST), "this is not = = valid toml [[[").expect("write junk");

    assert!(!is_scaffold_owned_path(
        base,
        &base.join("packages/typescript/package.json")
    ));
}

/// The record is itself a `.toml` file at the repo root, so `alef verify`'s walk
/// reaches it. Its explanatory header must not read as a provenance marker
/// ([`crate::core::hash::content_has_alef_marker`] matches the substrings
/// "auto-generated by alef" / "Generated by alef" anywhere in the first ten lines):
/// a file that claims to be alef-stamped but is outside the generated-file hash
/// pipeline has no computable hash, so it would surface as permanently stale. The
/// header is prose a human wrote and is easy to reword into a false positive, which
/// is why this is pinned rather than left to care. ~keep
#[test]
fn ownership_record_header_does_not_read_as_a_provenance_marker() {
    let rendered = render_ownership_manifest(&["packages/typescript/package.json".to_owned()]);
    assert!(
        !crate::core::hash::content_has_alef_marker(&rendered),
        "the record's own header must not look like an alef provenance marker, got:\n{rendered}"
    );
}

/// A path containing a quote or a backslash (a Windows-spelled key, a perverse but
/// legal filename) must survive the hand-rolled TOML writer. Escaping it wrongly
/// produces a manifest that no longer parses, and an unparseable manifest reads as
/// "alef owns nothing" -- so the failure would not be loud, it would quietly un-own
/// every path in the repo at once. ~keep
#[test]
fn ownership_record_escapes_paths_that_need_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let awkward = "packages/we\"ird\\name.json";

    record_scaffold_owned_path(base, &base.join(awkward)).expect("record");
    record_scaffold_owned_path(base, &base.join("packages/plain.json")).expect("record plain");

    let content = std::fs::read_to_string(base.join(OWNERSHIP_MANIFEST)).expect("read manifest");
    let parsed: OwnershipManifest = toml::from_str(&content).expect("manifest must stay parseable");
    assert!(
        parsed.owned_paths.iter().any(|path| path == awkward),
        "the awkward path must round-trip unchanged, got: {:?}",
        parsed.owned_paths
    );
    assert!(is_scaffold_owned_path(base, &base.join(awkward)));
    assert!(
        is_scaffold_owned_path(base, &base.join("packages/plain.json")),
        "a bad escape must not take the rest of the record down with it"
    );
}

#[test]
fn scaffold_owned_path_is_scoped_to_base_dir() {
    let dir_a = tempfile::tempdir().expect("tempdir a");
    let dir_b = tempfile::tempdir().expect("tempdir b");
    let target = std::path::PathBuf::from("packages/java/pom.xml");

    record_scaffold_owned_path(dir_a.path(), &dir_a.path().join(&target)).expect("record in a");

    assert!(!is_scaffold_owned_path(dir_b.path(), &dir_b.path().join(&target)));
}

/// Regression: a record written with an *absolute* `base_dir`
/// (`std::env::current_dir()`, what most `bin_cli` commands pass) must
/// still be found by a lookup that expresses `base_dir` *relatively*
/// (`PathBuf::from(".")`, what `version_regen.rs`'s regen helpers pass)
/// when both name the same directory -- and vice versa. Before
/// `scaffold_owned_path_key` normalized the stored key back to
/// `file.path`, the two representations produced different
/// `base_dir.join(path)` strings for the same file, so
/// `is_scaffold_owned_path` read as permanently `false` for any path
/// whose owning write and later check happened to come from commands
/// that spell `base_dir` differently -- which most real multi-command
/// sequences do (e.g. `alef all` establishes ownership, a later
/// `alef version` bump checks it), making the manifest effectively inert
/// even though it was being written and read from the exact same file on
/// disk the whole time.
#[test]
fn scaffold_owned_path_matches_across_absolute_and_relative_base_dir_spellings() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let _cwd = crate::test_support::CwdGuard::enter(tmp.path());

    let absolute_base = std::env::current_dir().expect("absolute cwd");
    let relative_base = Path::new(".");
    let relative_target = relative_base.join("packages/java/pom.xml");

    let result = (|| -> anyhow::Result<(bool, bool)> {
        // Written as an absolute-`base_dir` caller (e.g. a `bin_cli` command) would.
        record_scaffold_owned_path(&absolute_base, &absolute_base.join("packages/java/pom.xml"))?;
        // Checked as a relative-`base_dir` caller (e.g. `version_regen.rs`) would.
        let found_from_relative = is_scaffold_owned_path(relative_base, &relative_target);
        // And the reverse direction: written relatively, checked absolutely.
        record_scaffold_owned_path(relative_base, &relative_base.join("packages/csharp/foo.csproj"))?;
        let found_from_absolute =
            is_scaffold_owned_path(&absolute_base, &absolute_base.join("packages/csharp/foo.csproj"));
        Ok((found_from_relative, found_from_absolute))
    })();

    let (found_from_relative, found_from_absolute) = result.expect("record/check round-trip");
    assert!(
        found_from_relative,
        "a record written with an absolute base_dir must be found by a relative-base_dir lookup"
    );
    assert!(
        found_from_absolute,
        "a record written with a relative base_dir must be found by an absolute-base_dir lookup"
    );
}

/// The record must be a file `git add` picks up, not one alef itself gitignores --
/// same #80-shaped concern as [`ownership_record_lives_outside_the_gitignored_cache_and_is_valid_toml`],
/// applied to the merge-provenance baseline. ~keep
#[test]
fn toml_merge_provenance_record_lives_outside_the_gitignored_cache_and_is_valid_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let mut arrays = std::collections::BTreeMap::new();
    arrays.insert(
        "discovery.exclude".to_string(),
        vec!["target/**".to_string(), "docs/assets/**".to_string()],
    );

    write_toml_merge_provenance(base, Path::new("poly.toml"), &arrays).expect("write provenance");

    let manifest_path = base.join(TOML_MERGE_PROVENANCE_MANIFEST);
    assert!(manifest_path.exists(), "the record must exist at the repo root");
    assert!(
        !manifest_path.starts_with(base.join(CACHE_DIR)),
        "the record must not live under the gitignored `{CACHE_DIR}` directory"
    );
    let content = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let parsed: TomlMergeProvenanceFile = toml::from_str(&content).expect("the record must be valid TOML");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].relative_path, "poly.toml");
    assert_eq!(parsed.entries[0].key_path, "discovery.exclude");
    assert_eq!(
        parsed.entries[0].values,
        vec!["target/**".to_string(), "docs/assets/**".to_string()]
    );
}

/// A fresh clone carries the committed record but no `.alef/` cache at all. Simulated
/// by writing into one `base_dir` and reading the manifest back from a second,
/// cache-less one -- mirrors [`committed_record_answers_identically_on_a_cache_less_clone`]
/// for the merge-provenance baseline.
#[test]
fn toml_merge_provenance_answers_identically_on_a_cache_less_clone() {
    let warm = tempfile::tempdir().expect("tempdir warm");
    let clone = tempfile::tempdir().expect("tempdir clone");
    let mut arrays = std::collections::BTreeMap::new();
    arrays.insert("discovery.exclude".to_string(), vec!["docs/assets/**".to_string()]);

    write_toml_merge_provenance(warm.path(), Path::new("poly.toml"), &arrays).expect("write provenance");
    std::fs::copy(
        warm.path().join(TOML_MERGE_PROVENANCE_MANIFEST),
        clone.path().join(TOML_MERGE_PROVENANCE_MANIFEST),
    )
    .expect("check out the committed record");

    assert!(
        !clone.path().join(CACHE_DIR).exists(),
        "the simulated clone must have no machine-local cache"
    );
    assert_eq!(
        read_toml_merge_provenance(warm.path(), Path::new("poly.toml")),
        read_toml_merge_provenance(clone.path(), Path::new("poly.toml")),
        "a fresh clone must agree with the warm machine about alef's prior proposal"
    );
}

/// An unparseable record must read as "no prior proposal for anything" -- the prune
/// step then removes nothing, rather than panicking or, far worse, guessing.
#[test]
fn unparseable_toml_merge_provenance_record_prunes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(
        base.join(TOML_MERGE_PROVENANCE_MANIFEST),
        "this is not = = valid toml [[[",
    )
    .expect("write junk");

    assert_eq!(
        read_toml_merge_provenance(base, Path::new("poly.toml")),
        std::collections::BTreeMap::new()
    );
}

/// The record is itself a `.toml` file at the repo root, so `alef verify`'s walk
/// reaches it. Its explanatory header must not read as a provenance marker, for the
/// same reason pinned in [`ownership_record_header_does_not_read_as_a_provenance_marker`].
#[test]
fn toml_merge_provenance_header_does_not_read_as_a_provenance_marker() {
    assert!(
        !crate::core::hash::content_has_alef_marker(TOML_MERGE_PROVENANCE_HEADER),
        "the record's own header must not look like an alef provenance marker, got:\n{TOML_MERGE_PROVENANCE_HEADER}"
    );
}

/// Writing a second, unrelated merge target's provenance must not clobber a
/// previously recorded one -- this is the read-modify-write round trip
/// [`write_toml_merge_provenance`]'s doc promises ("other merge targets' records are
/// left untouched"), pinned so a future rewrite of the read-modify-write step cannot
/// silently drop it.
#[test]
fn toml_merge_provenance_write_extends_rather_than_replaces_other_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let mut poly_arrays = std::collections::BTreeMap::new();
    poly_arrays.insert("discovery.exclude".to_string(), vec!["target/**".to_string()]);
    write_toml_merge_provenance(base, Path::new("poly.toml"), &poly_arrays).expect("write poly.toml provenance");

    let mut other_arrays = std::collections::BTreeMap::new();
    other_arrays.insert("some.key".to_string(), vec!["value".to_string()]);
    write_toml_merge_provenance(base, Path::new("other.toml"), &other_arrays).expect("write other.toml provenance");

    assert_eq!(
        read_toml_merge_provenance(base, Path::new("poly.toml")),
        poly_arrays,
        "recording a second merge target's provenance must leave the first's untouched"
    );
    assert_eq!(read_toml_merge_provenance(base, Path::new("other.toml")), other_arrays);
}

/// The leading whitespace of every array-element line in a rendered record.
///
/// An element line is any line between one ending in `= [` and the `]` that closes it, which
/// is the only structure both records share -- deliberately derived from the rendered bytes
/// rather than from [`RECORD_ARRAY_INDENT`], so the comparison below cannot agree with itself
/// by construction. ~keep
fn array_element_indents(rendered: &str) -> Vec<String> {
    let mut indents = Vec::new();
    let mut inside_array = false;
    for line in rendered.lines() {
        let trimmed = line.trim();
        if inside_array {
            if trimmed == "]" {
                inside_array = false;
            } else {
                indents.push(line.chars().take_while(|character| character.is_whitespace()).collect());
            }
        } else if trimmed.ends_with("= [") {
            inside_array = true;
        }
    }
    indents
}

/// The two committed records sit side by side in a consumer's repo root and pass through the
/// same `poly fmt --check` gate, so how they indent an array element is one fact -- and it was
/// derived in two places that never compared notes: the ownership record hand-rendered two
/// spaces while the provenance record inherited `toml::to_string_pretty`'s four, which made
/// every regenerated tree unreleasable downstream (the gate says "would reformat", and
/// hand-formatting is overwritten by the next `alef generate`).
///
/// Comparing the two writers' actual output, rather than pinning the literal two spaces, is
/// what makes the next divergence fail here whichever side moves. ~keep
#[test]
fn both_committed_records_indent_array_elements_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    // Long enough that neither record collapses to one line -- there is no element
    // indentation to compare in the inline shape. ~keep
    let values = vec![
        "packages/generated-bindings/some-language/build/**".to_string(),
        "packages/generated-bindings/other-language/build/**".to_string(),
        "packages/generated-bindings/third-language/build/**".to_string(),
    ];
    let mut arrays = std::collections::BTreeMap::new();
    arrays.insert("discovery.exclude".to_string(), values.clone());

    write_toml_merge_provenance(base, Path::new("poly.toml"), &arrays).expect("write provenance");
    let provenance = std::fs::read_to_string(base.join(TOML_MERGE_PROVENANCE_MANIFEST)).expect("read provenance");
    let ownership = render_ownership_manifest(&values);

    let provenance_indents = array_element_indents(&provenance);
    let ownership_indents = array_element_indents(&ownership);
    assert_eq!(
        ownership_indents.len(),
        values.len(),
        "apparatus check: the ownership record must render one element line per value, got:\n{ownership}"
    );
    assert_eq!(
        provenance_indents.len(),
        values.len(),
        "apparatus check: the provenance record must render one element line per value, got:\n{provenance}"
    );
    assert_eq!(
        provenance_indents, ownership_indents,
        "the two committed records must indent array elements identically, got \
         {provenance_indents:?} for the provenance record and {ownership_indents:?} for the \
         ownership record"
    );
}

/// Both committed records are rewritten wholesale on every `alef generate`, so the shape they
/// emit has to be the shape `poly fmt` would leave alone. It was not: a short array was
/// written one element per line, the consumer's format gate collapsed it onto one line, and
/// the next `alef generate` expanded it again -- the file changed in every commit forever and
/// no one could stop it by hand. The boundary below is measured against the bundled formatter
/// (120 columns inline is collapsed, 121 is left expanded), not assumed. ~keep
#[test]
fn record_arrays_collapse_exactly_where_the_format_gate_collapses_them() {
    let short = render_record_assignment("values", &["one".to_string(), "two".to_string()]);
    assert_eq!(
        short, r#"values = ["one", "two"]"#,
        "an array the format gate would collapse must be written inline"
    );

    let empty = render_record_assignment("values", &[]);
    assert_eq!(empty, "values = []", "an empty array has nothing to spread over lines");

    // Sized so `values = [...]` is exactly RECORD_ARRAY_MAX_INLINE_WIDTH columns.
    let filler = "x".repeat(RECORD_ARRAY_MAX_INLINE_WIDTH - r#"values = [""]"#.len());
    let at_limit = render_record_assignment("values", std::slice::from_ref(&filler));
    assert_eq!(
        at_limit.chars().count(),
        RECORD_ARRAY_MAX_INLINE_WIDTH,
        "apparatus check: the fixture must land exactly on the limit, got:\n{at_limit}"
    );
    assert!(
        !at_limit.contains('\n'),
        "a line exactly at the limit is still collapsed by the gate, so it must stay inline"
    );

    let over_limit = render_record_assignment("values", &[format!("{filler}y")]);
    assert_eq!(
        over_limit,
        format!("values = [\n{RECORD_ARRAY_INDENT}\"{filler}y\",\n]"),
        "one column past the limit the gate leaves the array expanded, so alef must too"
    );
}

/// A value carrying a quote or a backslash must survive the hand-rolled provenance writer,
/// for the same reason [`ownership_record_escapes_paths_that_need_it`] pins it for the other
/// record: the record is written by hand rather than by a serializer, and an unparseable one
/// reads as "alef proposed nothing", so a bad escape silently disables pruning instead of
/// failing. ~keep
#[test]
fn toml_merge_provenance_escapes_values_that_need_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let awkward = vec!["we\"ird\\value/**".to_string(), "plain/**".to_string()];
    let mut arrays = std::collections::BTreeMap::new();
    arrays.insert("discovery.ex\"clude".to_string(), awkward);

    write_toml_merge_provenance(base, Path::new("poly.toml"), &arrays).expect("write provenance");

    assert_eq!(
        read_toml_merge_provenance(base, Path::new("poly.toml")),
        arrays,
        "an awkward key path and value must round-trip through the hand-rolled writer unchanged"
    );
}
