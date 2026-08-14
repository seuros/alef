use super::*;

#[cfg(test)]
mod write_scaffold_normalize_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use std::path::PathBuf;

    fn make_file(name: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(name),
            content: content.to_owned(),
            generated_header: false,
        }
    }

    /// `write_scaffold_files_with_overwrite` must strip trailing whitespace and
    /// ensure a single trailing newline — matching what prek's
    /// `end-of-file-fixer` and `trailing-whitespace` hooks would do.
    #[test]
    fn test_scaffold_write_normalizes_trailing_whitespace_and_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content = "line one   \nline two\n\n";
        let files = vec![make_file("out.py", content)];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.py")).expect("read ok");
        assert_eq!(
            written, "line one\nline two\n",
            "trailing whitespace must be stripped and single newline ensured"
        );
    }

    #[test]
    fn test_scaffold_write_adds_missing_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.gleam", "pub fn main() {}")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.gleam")).expect("read ok");
        assert!(
            written.ends_with('\n'),
            "file must end with newline, got: {:?}",
            written
        );
    }

    #[test]
    fn test_scaffold_write_does_not_add_double_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.zig", "const x = 1;\n")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.zig")).expect("read ok");
        assert!(!written.ends_with("\n\n"), "must not have double trailing newline");
        assert!(written.ends_with('\n'));
    }

    #[test]
    fn poly_scaffold_merges_generated_defaults_without_deleting_user_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing = r#"# Project policy. ~keep
[lint.python.ruff]
ignore = ["PT027", "S105"]

[per-file-ignores]
"tools/**" = ["S105"]

[hooks.builtin]
file_safety = { exclude = ["bindings/manual.rs"] }

[project-policy]
owner = "maintainers"
"#;
        std::fs::write(base.join("poly.toml"), existing).expect("write existing config");
        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: r#"[lint.python.ruff]
ignore = ["F401"]

[per-file-ignores]
"tests/**" = ["S101"]

[hooks.builtin]
file_safety = { exclude = ["target/**"] }
"#
            .into(),
            generated_header: true,
        };

        write_scaffold_files_with_overwrite(&[generated], base, true).expect("merge poly config");
        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        let parsed = merged.parse::<toml_edit::DocumentMut>().expect("merged TOML parses");

        assert!(merged.contains("# Project policy. ~keep"), "{merged}");
        assert_eq!(parsed["project-policy"]["owner"].as_str(), Some("maintainers"));
        assert!(parsed["per-file-ignores"]["tools/**"].is_value());
        assert!(parsed["per-file-ignores"]["tests/**"].is_value());
        let ruff_ignores = parsed["lint"]["python"]["ruff"]["ignore"]
            .as_array()
            .expect("ruff ignore array");
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("PT027")));
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("S105")));
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("F401")));
        let file_safety = parsed["hooks"]["builtin"]["file_safety"]["exclude"]
            .as_array()
            .expect("file safety excludes");
        assert!(
            file_safety
                .iter()
                .any(|value| value.as_str() == Some("bindings/manual.rs"))
        );
        assert!(file_safety.iter().any(|value| value.as_str() == Some("target/**")));

        let generated_again = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: r#"[lint.python.ruff]
ignore = ["F401"]

[per-file-ignores]
"tests/**" = ["S101"]

[hooks.builtin]
file_safety = { exclude = ["target/**"] }
"#
            .into(),
            generated_header: true,
        };
        let count = write_scaffold_files_with_overwrite(&[generated_again], base, true).expect("repeat merge");
        assert_eq!(count, 0, "merged poly config must converge on a second scaffold pass");
    }

    /// `normalize_content` must strip trailing whitespace from `.rs` files even
    /// when rustfmt rejects them — e.g. cextendr `lib.rs` files use the
    /// `name: T = "default"` parameter-default syntax that rustfmt cannot
    /// parse, so it falls back to the raw codegen output. Without a final
    /// whitespace pass, the raw output's trailing-whitespace blank lines
    /// (e.g. `    \n` between `#[must_use]` and `pub fn …`) survive into the
    /// finalised `alef:hash`, and prek's `trailing-whitespace` hook then
    /// rewrites the file post-hash, breaking `alef verify`.
    #[test]
    fn test_normalize_content_strips_trailing_whitespace_when_rustfmt_fails() {
        let path = PathBuf::from("packages/r/src/rust/src/lib.rs");
        let content = "extendr_module! {\n    fn convert(\n    \n        title: String = \"\",\n    );\n}\n";
        let normalized = normalize_content(&path, content);
        for (i, line) in normalized.lines().enumerate() {
            assert_eq!(
                line.trim_end(),
                line,
                "line {i} has trailing whitespace after normalize: {line:?}"
            );
        }
        assert!(normalized.ends_with('\n'), "must end with newline");
    }

    /// `sweep_orphans` must delete alef-marked files that aren't in the keep set,
    /// preserve user-owned files (no marker), and preserve files that are in the
    /// keep set even if they have the marker.
    #[test]
    fn test_sweep_orphans_removes_only_alef_marked_files_outside_keep_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let nested = base.join("e2e/elixir/test");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc\n";
        let kept = nested.join("keep_test.exs");
        let orphan = nested.join("orphan_test.exs");
        let user_owned = nested.join("user_helper.exs");

        std::fs::write(&kept, format!("{alef_marker}defmodule Keep do\nend\n")).unwrap();
        std::fs::write(&orphan, format!("{alef_marker}defmodule Orphan do\nend\n")).unwrap();
        std::fs::write(&user_owned, "defmodule UserHelper do\nend\n").unwrap();

        let mut keep = std::collections::HashSet::new();
        keep.insert(kept.clone());

        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 1, "should remove exactly one orphan");
        assert!(kept.exists(), "kept alef-marked file must remain");
        assert!(!orphan.exists(), "orphan alef-marked file must be removed");
        assert!(user_owned.exists(), "user-owned (no marker) file must remain");
    }

    /// `sweep_orphans` must skip dependency / build directories (target, node_modules,
    /// _build, deps, vendor, build, dist, .git, .venv) so it never deletes anything
    /// inside a vendored or compiled tree.
    #[test]
    fn test_sweep_orphans_skips_dependency_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let alef_marker = "// auto-generated by alef\n// alef:hash:def\n";
        for skip_dir in ["target", "node_modules", "_build", "vendor"] {
            let nested = base.join(skip_dir).join("nested");
            std::fs::create_dir_all(&nested).expect("mkdir");
            std::fs::write(nested.join("orphan.rs"), alef_marker).unwrap();
        }
        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 0, "must not descend into dependency directories");
    }

    #[test]
    fn targeted_e2e_sweep_ignores_snippet_outputs_nested_under_e2e_root() {
        let base = PathBuf::from("/workspace");
        let e2e_root = base.join("e2e");
        let snippet_root = e2e_root.join("ruby");
        let outputs = vec![snippet_root.join(".alef-snippet-coverage.json")];

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));

        assert!(roots.is_empty(), "snippet-only output must not authorize an e2e sweep");
    }

    #[test]
    fn targeted_e2e_sweep_includes_only_languages_with_current_e2e_outputs() {
        let base = PathBuf::from("/workspace");
        let e2e_root = base.join("e2e");
        let snippet_root = base.join("docs/snippets-generated");
        let outputs = vec![
            e2e_root.join("ruby/spec/example_spec.rb"),
            snippet_root.join("ruby/api/example.md"),
        ];

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));

        assert_eq!(roots, [e2e_root.join("ruby/spec")]);
    }

    #[test]
    fn targeted_e2e_sweep_preserves_specs_when_only_scaffold_and_snippets_are_generated() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let e2e_root = directory.path().join("e2e");
        let snippet_root = directory.path().join("docs/snippets-generated");
        let spec_directory = e2e_root.join("ruby/spec");
        std::fs::create_dir_all(&spec_directory).expect("create spec directory");
        let existing_spec = spec_directory.join("existing_spec.rb");
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(
            &existing_spec,
            format!("# alef:hash:{HASH}\n# This file is auto-generated by alef — DO NOT EDIT.\n"),
        )
        .expect("write existing spec");
        let outputs = vec![
            e2e_root.join("ruby/Gemfile"),
            snippet_root.join("ruby/http/create_item.md"),
            snippet_root.join(".alef-snippet-coverage.json"),
        ];
        let keep = outputs.iter().cloned().collect();

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));
        let removed = sweep_orphans(&roots, &keep).expect("sweep targeted outputs");

        assert!(
            roots.is_empty(),
            "top-level scaffolding and snippets own no E2E test subtree"
        );
        assert_eq!(removed, 0);
        assert!(
            existing_spec.exists(),
            "an ungenerated E2E subtree must remain untouched"
        );
    }

    #[test]
    fn managed_toml_scaffold_preserves_unknown_tables_and_records_ownership() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let manifest = directory.path().join("crates/sample-py/Cargo.toml");
        std::fs::create_dir_all(manifest.parent().expect("manifest parent")).expect("create manifest directory");
        std::fs::write(
            &manifest,
            "[package]\nname = \"old-name\"\n\n[lints.clippy]\nunwrap_used = \"deny\"\n",
        )
        .expect("write existing manifest");
        let files = vec![crate::core::backend::GeneratedFile {
            path: PathBuf::from("crates/sample-py/Cargo.toml"),
            content: "[package]\nname = \"sample-py\"\nversion = \"1.0.0\"\n".into(),
            generated_header: true,
        }];

        let written = write_scaffold_files(&files, directory.path()).expect("refresh managed manifest");
        let refreshed = std::fs::read_to_string(&manifest).expect("read refreshed manifest");

        assert_eq!(written, 1);
        assert!(refreshed.contains("auto-generated by alef"), "{refreshed}");
        assert!(refreshed.contains("name = \"sample-py\""), "{refreshed}");
        assert!(refreshed.contains("[lints.clippy]"), "{refreshed}");
        assert!(refreshed.contains("unwrap_used = \"deny\""), "{refreshed}");
        let grouped = vec![(crate::core::config::Language::Rust, files)];
        assert!(
            diff_files(&grouped, directory.path())
                .expect("diff managed manifest")
                .is_empty()
        );
    }

    /// Regression: a file that contains loose "auto-generated" or "DO NOT EDIT"
    /// markers but lacks the `alef:hash:` line must NOT be deleted by
    /// `sweep_orphans`. This protects consumer-vendored files such as cgo headers.
    #[test]
    fn sweep_orphans_preserves_loose_marker_file_without_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let include_dir = base.join("packages/go/include");
        std::fs::create_dir_all(&include_dir).expect("mkdir");

        let vendored = include_dir.join("sample_crawler.h");
        std::fs::write(
            &vendored,
            "// DO NOT EDIT — vendored cgo header\n#ifndef FOO_H\n#define FOO_H\n\ntypedef void CrawlEngine;\n\n#endif\n",
        )
        .unwrap();

        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 0, "vendored file without alef:hash must not be deleted");
        assert!(vendored.exists(), "vendored cgo header must survive sweep_orphans");
    }

    /// Positive path: a file that contains the `alef:hash:` line IS alef-owned
    /// and must be deleted by `sweep_orphans` when not in the keep set.
    #[test]
    fn sweep_orphans_removes_file_with_alef_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out_dir = base.join("e2e/rust/src");
        std::fs::create_dir_all(&out_dir).expect("mkdir");

        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let alef_file = out_dir.join("lib.rs");
        std::fs::write(
            &alef_file,
            format!(
                "// alef:hash:{HASH}\n// This file is auto-generated by alef — DO NOT EDIT.\npub fn hello() {{}}\n"
            ),
        )
        .unwrap();

        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 1, "alef-owned file not in keep set must be deleted");
        assert!(!alef_file.exists(), "alef:hash file must be removed by sweep_orphans");
    }

    /// `collect_alef_headered_paths` must return all alef-headered files under
    /// the given root and skip user-owned (no marker) files.
    #[test]
    fn test_collect_alef_headered_paths_finds_headered_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let lang_dir = base.join("python");
        std::fs::create_dir_all(&lang_dir).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc123\nprint('hello')\n";
        let user_file = "print('user code')\n";

        let headered = lang_dir.join("test_chat.py");
        let plain = lang_dir.join("conftest.py");
        std::fs::write(&headered, alef_marker).unwrap();
        std::fs::write(&plain, user_file).unwrap();

        let collected = collect_alef_headered_paths(base);
        assert!(collected.contains(&headered), "alef-headered file must be collected");
        assert!(!collected.contains(&plain), "user-owned file must not be collected");
    }

    /// `collect_alef_headered_paths` on a non-existent root must return an
    /// empty set without panicking.
    #[test]
    fn test_collect_alef_headered_paths_missing_root_returns_empty() {
        let paths = collect_alef_headered_paths(std::path::Path::new("/nonexistent/test_apps"));
        assert!(paths.is_empty(), "missing root must yield empty set");
    }

    /// Regression for #524: `collect_alef_headered_paths` must find an
    /// alef-headered file that is missing its `alef:hash:` line. Matching on
    /// `extract_hash(..).is_some()` instead of the marker (the pre-fix
    /// behaviour) makes an already-unstamped file permanently invisible to
    /// this scan, which is exactly how a stripped file stays stripped
    /// forever.
    #[test]
    fn test_collect_alef_headered_paths_finds_files_missing_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let lang_dir = base.join("java");
        std::fs::create_dir_all(&lang_dir).expect("mkdir");

        let headered_no_hash = "// This file is auto-generated by alef — DO NOT EDIT.\npublic class Foo {}\n";
        let headered_path = lang_dir.join("Foo.java");
        std::fs::write(&headered_path, headered_no_hash).unwrap();

        let collected = collect_alef_headered_paths(base);
        assert!(
            collected.contains(&headered_path),
            "a headered file missing its alef:hash: line must still be collected, got: {collected:?}"
        );
    }

    /// Regression for #524: `finalize_hashes_sweeping` must re-stamp an
    /// alef-headered, hash-less file found on disk under `roots` even when
    /// that file's path is absent from the caller-supplied `paths` set. This
    /// is the exact scenario a language dropped from the per-language cache
    /// (`generation::generate`) produces: its output is never added to
    /// `current_gen_paths`, so plain `finalize_hashes` would leave it
    /// unstamped forever.
    #[test]
    fn test_finalize_hashes_sweeping_restamps_file_absent_from_explicit_path_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("java");
        std::fs::create_dir_all(&root).expect("mkdir");

        let stripped_content = "// This file is auto-generated by alef — DO NOT EDIT.\npublic class Foo {}\n";
        let cached_language_file = root.join("Foo.java");
        std::fs::write(&cached_language_file, stripped_content).unwrap();

        // Simulate a run whose in-memory tracking never saw this file at all
        // (e.g. its language hit the per-language cache and was dropped from
        // `bindings` before `current_gen_paths` was built).
        let explicit_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let roots = vec![dir.path().to_path_buf()];
        let sources_hash = "sources";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"java\"]\n";

        let updated =
            finalize_hashes_sweeping(&explicit_paths, &roots, sources_hash, alef_toml_bytes).expect("sweep ok");
        assert_eq!(updated, 1, "the swept, previously-unstamped file must be finalized");

        let after = std::fs::read_to_string(&cached_language_file).expect("read after sweep");
        assert!(
            crate::core::hash::extract_hash(&after).is_some(),
            "file discovered only via the roots sweep must carry an alef:hash: line, got:\n{after}"
        );
    }

    /// `finalize_hashes_sweeping` must not double-count a file present in
    /// both the explicit `paths` set and the `roots` sweep -- the union is a
    /// `HashSet`, so it is stamped exactly once either way.
    #[test]
    fn test_finalize_hashes_sweeping_does_not_duplicate_explicitly_tracked_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("python");
        std::fs::create_dir_all(&root).expect("mkdir");

        let content = "# This file is auto-generated by alef — DO NOT EDIT.\nprint('hi')\n";
        let file_path = root.join("mod.py");
        std::fs::write(&file_path, content).unwrap();

        let explicit_paths: std::collections::HashSet<std::path::PathBuf> =
            std::iter::once(file_path.clone()).collect();
        let roots = vec![dir.path().to_path_buf()];

        let updated = finalize_hashes_sweeping(&explicit_paths, &roots, "sources", b"[workspace]\n").expect("sweep ok");
        assert_eq!(updated, 1, "one physical file must produce exactly one update");
    }

    /// End-to-end regression for #524's documented two-pass design: a file
    /// written by `write_files_report` (headered, no hash yet by design) must
    /// carry an `alef:hash:` line once `finalize_hashes` runs over its path,
    /// matching what every real pipeline caller does.
    #[test]
    fn test_write_then_finalize_stamps_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![(
            crate::core::config::Language::Rust,
            vec![GeneratedFile {
                path: PathBuf::from("lib.rs"),
                content: "pub fn hello() {}\n".to_string(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, base).expect("write ok");
        let written_path = base.join("lib.rs");
        assert!(report.changed_paths.contains(&written_path));

        let after_write = std::fs::read_to_string(&written_path).expect("read after write");
        assert!(
            crate::core::hash::extract_hash(&after_write).is_none(),
            "write_files_report must not embed a hash before finalize_hashes runs"
        );

        let mut paths = std::collections::HashSet::new();
        paths.insert(written_path.clone());
        finalize_hashes(&paths, "sources", b"[workspace]\nlanguages = [\"rust\"]\n").expect("finalize ok");

        let after_finalize = std::fs::read_to_string(&written_path).expect("read after finalize");
        assert!(
            crate::core::hash::extract_hash(&after_finalize).is_some(),
            "a written file must carry an alef:hash: line after finalize_hashes, got:\n{after_finalize}"
        );
    }

    /// Invariant: after `write` + simulated format-pass + `finalize_hashes`, the
    /// embedded `alef:hash:` must cover both generation inputs and the finalized file body.
    #[test]
    fn test_finalize_hashes_embeds_per_file_content_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content_before_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n";
        let file_path = base.join("lib.rs");
        std::fs::write(&file_path, content_before_format).expect("write pre-format content");

        let content_after_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n\n";
        std::fs::write(&file_path, content_after_format).expect("write post-format content");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"rust\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");
        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let inputs_hash = crate::core::hash::compute_inputs_hash(sources_hash, alef_toml_bytes);
        let expected = crate::core::hash::compute_file_hash(&inputs_hash, content_after_format);
        assert_eq!(embedded, expected, "embedded hash must cover the finalized file body");

        let reformatted = format!("{content_after_format}\n// formatter added this line\n");
        std::fs::write(&file_path, &reformatted).expect("simulate post-finalize formatter rewrite");
        let after_reformat = std::fs::read_to_string(&file_path).expect("read after reformat");
        let _still_embedded = crate::core::hash::extract_hash(&after_reformat);
        assert_ne!(
            crate::core::hash::compute_file_hash(&inputs_hash, &after_reformat),
            expected,
            "editing generated output must invalidate its embedded hash"
        );
    }

    /// Regression: `finalize_hashes` must be idempotent when run twice on the
    /// same file — the second pass must detect the existing hash is already
    /// correct and skip the write.
    #[test]
    fn test_finalize_hashes_is_idempotent_with_inputs_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n";
        let file_path = base.join("lib.rs");
        std::fs::write(&file_path, content).expect("write initial content");

        let sources_hash = "sources";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"rust\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());

        let n1 = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("first finalize");
        assert_eq!(n1, 1, "first finalize must write the hash line");

        let n2 = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("second finalize");
        assert_eq!(n2, 0, "second finalize must be a no-op (same inputs hash)");
    }

    /// `finalize_hashes` must skip files without the alef header marker, even
    /// when a non-Rust file has content that would otherwise match. Go files
    /// (gofmt emitting blank lines) are preserved unchanged.
    #[test]
    fn test_finalize_hashes_non_rust_file_gets_inputs_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let gofmt_output = concat!(
            "// This file is auto-generated by alef — DO NOT EDIT.\n",
            "package foo\n",
            "\n",
            "\n",
            "func Hello() {}\n",
        );
        let file_path = base.join("binding.go");
        std::fs::write(&file_path, gofmt_output).expect("write gofmt output");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"go\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");

        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let inputs_hash = crate::core::hash::compute_inputs_hash(sources_hash, alef_toml_bytes);
        let expected = crate::core::hash::compute_file_hash(&inputs_hash, gofmt_output);
        assert_eq!(embedded, expected, "embedded hash must cover Go file content");

        let stripped = crate::core::hash::strip_hash_line(&finalised);
        assert!(
            stripped.contains("\n\n\n"),
            "two consecutive blank lines must survive finalize_hashes: got:\n{stripped:?}"
        );
    }

    /// Regression: `finalize_hashes` must recognize both "auto-generated by alef"
    /// (standard header) and "Generated by alef" (custom headers in Swift, Kotlin,
    /// Dart, Gleam, Zig, JNI). Without this, renamed files like SwiftPluginHelpers.swift
    /// would not get the `alef:hash:` marker, preventing the cleanup system from
    /// identifying them as alef-owned and deleting stale renamed files.
    #[test]
    fn test_finalize_hashes_recognizes_generated_by_alef_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let swift_content =
            "// Generated by alef. Do not edit by hand.\n// swift-format-ignore-file\n\nimport Foundation\n";
        let file_path = base.join("Helpers.swift");
        std::fs::write(&file_path, swift_content).expect("write swift content");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"swift\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        let updated = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        assert_eq!(
            updated, 1,
            "finalize_hashes must process files with 'Generated by alef' header"
        );

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");

        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let inputs_hash = crate::core::hash::compute_inputs_hash(sources_hash, alef_toml_bytes);
        let expected = crate::core::hash::compute_file_hash(&inputs_hash, swift_content);
        assert_eq!(embedded, expected, "embedded hash must cover Swift file content");
    }

    /// Regression: `write_scaffold_files_with_overwrite(overwrite=false)` must
    /// skip files that already exist on disk, leaving the existing content
    /// unchanged.  This is the invariant relied on by scaffold-once files
    /// (Cargo.toml, package.json, gemspec) — user customisations are preserved.
    ///
    /// README files are NOT scaffold-once: they are always regenerated from
    /// templates.  Using `overwrite=false` for READMEs means a file modified by
    /// an external tool (e.g. `rumdl-fmt` padding table columns) is silently
    /// preserved, while `alef readme` (which always uses `overwrite=true`) writes
    /// fresh compact content.  The two commands then produce different bytes for
    /// the same README — the root cause of the `alef generate`/`alef readme`
    /// divergence surfaced during downstream regeneration.
    #[test]
    fn readme_overwrite_false_preserves_existing_content_producing_divergence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let padded_content = "# My README\n\n| Document            | Size  |\n| ------------------- | ----- |\n| Lists (Timeline)    | 129KB |\n";
        std::fs::write(base.join("README.md"), padded_content).expect("write padded README");

        let compact_content = "# My README\n\n| Document | Size |\n|----------|------|\n| Lists (Timeline) | 129KB |\n";
        let files = vec![make_file("README.md", compact_content)];

        write_scaffold_files_with_overwrite(&files, base, false).expect("write ok (overwrite=false)");
        let after_false = std::fs::read_to_string(base.join("README.md")).expect("read");
        assert_eq!(
            after_false, padded_content,
            "overwrite=false must not touch an existing README — padded content preserved (bug state)"
        );

        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok (overwrite=true)");
        let after_true = std::fs::read_to_string(base.join("README.md")).expect("read");
        assert!(
            after_true.contains("|----------|"),
            "overwrite=true must write compact-separator content, got:\n{after_true}"
        );
        assert!(
            !after_true.contains("| ------------------- |"),
            "overwrite=true must NOT preserve rumdl-fmt-padded separators, got:\n{after_true}"
        );

        assert_eq!(
            after_true,
            normalize_content(&std::path::PathBuf::from("README.md"), compact_content),
            "alef readme and alef all must produce identical on-disk bytes for README files"
        );
    }

    /// A `.gitattributes` (or any seed file with `generated_header: false`) written
    /// by `write_scaffold_files(overwrite=false)` must not be overwritten when the
    /// file already exists on disk. This preserves hand-added entries such as
    /// `* text=auto eol=lf` that the user may have added alongside alef's entries.
    #[test]
    fn seed_file_with_generated_header_false_is_preserved_on_overwrite_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let original = "# hand-crafted\n* text=auto eol=lf\n";
        std::fs::write(base.join(".gitattributes"), original).expect("write original");

        let generated = GeneratedFile {
            path: std::path::PathBuf::from(".gitattributes"),
            content: "# Generated by alef scaffold.\ne2e/** linguist-generated=true\n".to_owned(),
            generated_header: false,
        };

        let count = write_scaffold_files_with_overwrite(&[generated], base, false).expect("write ok");
        assert_eq!(
            count, 0,
            "overwrite=false must not write any file when seed already exists"
        );

        let after = std::fs::read_to_string(base.join(".gitattributes")).expect("read");
        assert_eq!(
            after, original,
            "overwrite=false must not touch an existing seed file (generated_header: false)"
        );
    }

    /// `detect_crate_edition` must return the edition declared in the nearest
    /// `Cargo.toml` when one is present, and fall back to `"2024"` when absent.
    #[test]
    fn test_detect_crate_edition_reads_from_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let cargo_toml = "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        std::fs::write(base.join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");

        let src = base.join("src").join("lib.rs");
        std::fs::create_dir_all(src.parent().unwrap()).expect("mkdir src");

        let edition = detect_crate_edition(&src);
        assert_eq!(edition, "2021", "should detect edition 2021 from Cargo.toml");
    }

    #[test]
    fn test_detect_crate_edition_defaults_to_2024_when_no_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let orphan = dir.path().join("orphan.rs");

        let edition = detect_crate_edition(&orphan);
        assert_eq!(edition, "2024", "should default to 2024 when no Cargo.toml found");
    }

    #[test]
    fn test_detect_crate_edition_defaults_to_2024_when_edition_absent_from_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        std::fs::write(
            base.join("Cargo.toml"),
            "[package]\nname = \"no-edition-crate\"\nversion = \"0.1.0\"\n",
        )
        .expect("write Cargo.toml");

        let src = base.join("lib.rs");
        let edition = detect_crate_edition(&src);
        assert_eq!(edition, "2024", "should default to 2024 when edition field absent");
    }

    #[test]
    fn test_parse_package_edition_extracts_value() {
        let toml = "[package]\nname = \"x\"\nedition = \"2021\"\n";
        assert_eq!(parse_package_edition(toml).as_deref(), Some("2021"));
    }

    #[test]
    fn test_parse_package_edition_ignores_other_sections() {
        let toml = "[workspace]\nedition = \"2021\"\n[package]\nname = \"x\"\n";
        assert_eq!(parse_package_edition(toml), None);
    }

    /// `write_scaffold_files_with_overwrite` must set the executable bit on files
    /// whose content begins with a shebang line, matching the behaviour of
    /// `write_files`. Previously the scaffold writer lacked the chmod call, so
    /// generated shell scripts (e.g. `download_ffi.sh`, `run_tests.sh`) landed
    /// as `-rw-r--r--` and consumers could not execute them.
    #[cfg(unix)]
    #[test]
    fn test_scaffold_write_sets_executable_bit_for_shebang_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let shebang_content = "#!/usr/bin/env bash\nset -euo pipefail\necho hello\n";
        let file = GeneratedFile {
            path: std::path::PathBuf::from("run_tests.sh"),
            content: shebang_content.to_owned(),
            generated_header: false,
        };

        write_scaffold_files_with_overwrite(&[file], base, true).expect("write ok");

        let path = base.join("run_tests.sh");
        let metadata = std::fs::metadata(&path).expect("metadata");
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o100 != 0,
            "shebang file must have owner-executable bit set, got mode {mode:#o}"
        );
    }

    /// Non-shebang files must NOT receive the executable bit.
    #[cfg(unix)]
    #[test]
    fn test_scaffold_write_does_not_set_executable_bit_for_non_shebang_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let plain_content = "# not a shebang\nsome content\n";
        let file = GeneratedFile {
            path: std::path::PathBuf::from("plain.sh"),
            content: plain_content.to_owned(),
            generated_header: false,
        };

        write_scaffold_files_with_overwrite(&[file], base, true).expect("write ok");

        let path = base.join("plain.sh");
        let metadata = std::fs::metadata(&path).expect("metadata");
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 == 0,
            "non-shebang file must not have any executable bit set, got mode {mode:#o}"
        );
    }
}

/// Regression coverage for the crawlberg incident: a plain `alef all --clean` run
/// silently claimed and stamped hand-written `e2e/go/helpers_test.go` /
/// `e2e/go/main_test.go` (added an `alef:hash:` header they never had), and
/// separately clobbered `e2e/elixir/test/test_helper.exs` (deleted a hand-written
/// FFI environment-propagation workaround), because `write_scaffold_files_report`
/// wrote every `generated_header: true` file unconditionally, without ever
/// checking whether the pre-existing content on disk had ever been alef's.
#[cfg(test)]
mod scaffold_ownership_guard_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use crate::core::hash::{CommentStyle, content_has_alef_marker, extract_hash, header, inject_hash_line};
    use std::path::PathBuf;

    fn marked_content(body: &str) -> String {
        let with_header = format!("{}{body}", header(CommentStyle::DoubleSlash));
        inject_hash_line(&with_header, &"0".repeat(64))
    }

    /// THE core regression test: a pre-existing, unmarked file at a path alef has
    /// never emitted before must survive a `generated_header: true` write
    /// byte-for-byte, and must NOT gain an `alef:hash:` marker. Without the
    /// ownership guard in `write_scaffold_files_report`, this fails — the write
    /// path stamps the alef header onto the hand-written body unconditionally
    /// (reproducing the Go half of the crawlberg incident exactly: alef's
    /// generated content for `helpers_test.go` happened to be byte-identical to
    /// the hand-written file, so the only visible change was the new header).
    #[test]
    fn pre_existing_unmarked_file_at_unrecorded_path_survives_untouched_and_unstamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("e2e/go/helpers_test.go");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let hand_written =
            "package e2e_test\n\n// jsonString is hand-rolled here.\nfunc jsonString(v any) string { return \"\" }\n";
        std::fs::write(&target, hand_written).expect("seed hand-written file");

        let generated = GeneratedFile {
            path: target_relative,
            content: hand_written.to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert_eq!(
            after, hand_written,
            "a path alef has never recorded as its own output must be left byte-for-byte untouched"
        );
        assert!(
            extract_hash(&after).is_none(),
            "a file alef never authored must never gain an alef:hash: marker, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
    }

    /// The ownership guard must not fire on a path alef is physically unable to stamp.
    ///
    /// `ensure_generated_header` only knows a comment syntax for a fixed extension set;
    /// everything else (`.md` above all) is returned unchanged, so an alef-authored
    /// README never carries a marker no matter how many times it is regenerated. Keying
    /// the guard on the marker alone would therefore read every generated README as
    /// foreign content and freeze it permanently on the first run after this guard ships.
    #[test]
    fn unstampable_generated_file_is_still_regenerated_despite_carrying_no_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("README.md");
        let target = base.join(&target_relative);
        std::fs::write(&target, "# Stale generated README\n").expect("seed previous output");

        let regenerated = "# Regenerated README\n";
        let generated = GeneratedFile {
            path: target_relative,
            content: regenerated.to_owned(),
            generated_header: true,
        };

        write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            regenerated,
            "a markdown file alef cannot stamp must still be regenerated -- a missing marker \
             there is not evidence the file is foreign"
        );
    }

    /// Happy-path counterpart: a file alef legitimately authored on a prior run
    /// (and therefore already carries the marker on disk) is still updated
    /// normally when its content changes. This proves the ownership guard does
    /// not degrade into a blanket "never touch an existing file" rule.
    #[test]
    fn file_alef_already_owns_is_updated_normally_on_the_next_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("e2e/go/main_test.go");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let prior_run_content = marked_content("package e2e_test\n\nfunc TestMain(m *testing.M) { m.Run() }\n");
        std::fs::write(&target, &prior_run_content).expect("seed prior alef output");
        assert!(
            content_has_alef_marker(&prior_run_content),
            "sanity: seed must carry the marker"
        );

        let generated = GeneratedFile {
            path: target_relative,
            content: "package e2e_test\n\nfunc TestMain(m *testing.M) { os.Exit(m.Run()) }\n".to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert!(
            after.contains("os.Exit(m.Run())"),
            "a file alef already owns must be updated with this run's content, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 1);
    }

    /// A brand-new path (nothing on disk yet) is written and stamped normally —
    /// the guard only ever engages when there is pre-existing content to protect.
    #[test]
    fn brand_new_managed_file_is_written_and_stamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let generated = GeneratedFile {
            path: PathBuf::from("e2e/go/robots_test.go"),
            content: "package e2e_test\n\nfunc TestRobots(t *testing.T) {}\n".to_owned(),
            generated_header: true,
        };

        write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(base.join("e2e/go/robots_test.go")).expect("read after");
        assert!(
            after.starts_with("// This file is auto-generated by alef"),
            "a genuinely new path must be stamped normally, got:\n{after}"
        );
    }

    /// The specific shape that broke in the incident: a generator target
    /// directory containing a mix of alef-authored and hand-written files. Only
    /// the alef-owned file may be touched; the hand-written sibling in the same
    /// directory must survive completely untouched.
    #[test]
    fn mixed_directory_of_owned_and_hand_written_files_touches_only_the_owned_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let e2e_go_dir = base.join("e2e/go");
        std::fs::create_dir_all(&e2e_go_dir).expect("mkdir");

        let hand_written_path = e2e_go_dir.join("helpers_test.go");
        let hand_written =
            "package e2e_test\n\n// hand rolled, never alef's\nfunc jsonString(v any) string { return \"\" }\n";
        std::fs::write(&hand_written_path, hand_written).expect("seed hand-written sibling");

        let owned_path = e2e_go_dir.join("main_test.go");
        let owned_prior = marked_content("package e2e_test\n\nfunc TestMain(m *testing.M) { m.Run() }\n");
        std::fs::write(&owned_path, &owned_prior).expect("seed owned sibling");

        let files = vec![
            GeneratedFile {
                path: PathBuf::from("e2e/go/helpers_test.go"),
                content: hand_written.to_owned(),
                generated_header: true,
            },
            GeneratedFile {
                path: PathBuf::from("e2e/go/main_test.go"),
                content: "package e2e_test\n\nfunc TestMain(m *testing.M) { os.Exit(m.Run()) }\n".to_owned(),
                generated_header: true,
            },
        ];

        let report = write_scaffold_files_report(&files, base, true).expect("write ok");

        let hand_written_after = std::fs::read_to_string(&hand_written_path).expect("read hand-written after");
        assert_eq!(
            hand_written_after, hand_written,
            "the hand-written sibling in the same directory must survive untouched"
        );
        assert!(extract_hash(&hand_written_after).is_none(), "must not gain a marker");

        let owned_after = std::fs::read_to_string(&owned_path).expect("read owned after");
        assert!(
            owned_after.contains("os.Exit(m.Run())"),
            "the alef-owned sibling must still update, got:\n{owned_after}"
        );
        assert_eq!(
            report.changed_count(),
            1,
            "only the genuinely-owned sibling counts as changed"
        );
    }
}

#[cfg(test)]
mod generated_header_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;

    #[test]
    fn write_files_adds_missing_headers_to_managed_java_and_csharp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            (
                crate::core::config::Language::Java,
                vec![GeneratedFile {
                    path: "packages/java/Example.java".into(),
                    content: "package example;\n\npublic final class Example {}\n".into(),
                    generated_header: true,
                }],
            ),
            (
                crate::core::config::Language::Csharp,
                vec![GeneratedFile {
                    path: "packages/csharp/Example.cs".into(),
                    content: "namespace Example;\n\npublic sealed class Example {}\n".into(),
                    generated_header: true,
                }],
            ),
        ];

        write_files(&files, dir.path()).expect("write managed files");

        for path in ["packages/java/Example.java", "packages/csharp/Example.cs"] {
            let output = std::fs::read_to_string(dir.path().join(path)).expect("read managed file");
            assert!(
                output.starts_with("// This file is auto-generated by alef"),
                "{path}: {output}"
            );
        }
    }

    #[test]
    fn write_files_places_rust_header_before_multiline_inner_attribute() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Ffi,
            vec![GeneratedFile {
                path: "src/service.rs".into(),
                content: "#![allow(\n    unused_variables,\n)]\n\nfn generated() {}\n".into(),
                generated_header: true,
            }],
        )];

        write_files(&files, dir.path()).expect("write Rust file");

        let output = std::fs::read_to_string(dir.path().join("src/service.rs")).expect("read Rust file");
        assert!(output.starts_with("// This file is auto-generated by alef"));
        let header_end = output.find("#![allow(").expect("inner attribute");
        assert!(output[..header_end].contains("To verify freshness: alef verify"));
        let allowed_lint = output.find("unused_variables").expect("allowed lint");
        assert!(header_end < allowed_lint);
        assert!(!output[header_end..allowed_lint].contains("auto-generated by alef"));
    }

    #[test]
    fn write_files_places_php_header_after_opening_tag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Php,
            vec![GeneratedFile {
                path: "src/Service.php".into(),
                content: "<?php\ndeclare(strict_types=1);\n\nfinal class Service {}\n".into(),
                generated_header: true,
            }],
        )];

        write_files(&files, dir.path()).expect("write PHP file");

        let output = std::fs::read_to_string(dir.path().join("src/Service.php")).expect("read PHP file");
        assert!(output.starts_with("<?php\n// This file is auto-generated by alef"));
        assert!(output.contains("To verify freshness: alef verify\n\ndeclare(strict_types=1);"));
    }

    #[test]
    fn write_files_preserves_explicit_header_and_unmanaged_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let explicit = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
        let files = vec![(
            crate::core::config::Language::Java,
            vec![
                GeneratedFile {
                    path: "Managed.java".into(),
                    content: format!("{explicit}\npublic final class Managed {{}}\n"),
                    generated_header: true,
                },
                GeneratedFile {
                    path: "Unmanaged.java".into(),
                    content: "public final class Unmanaged {}\n".into(),
                    generated_header: false,
                },
            ],
        )];

        write_files(&files, dir.path()).expect("write files");

        let managed = std::fs::read_to_string(dir.path().join("Managed.java")).expect("read managed file");
        assert_eq!(managed.matches("auto-generated by alef").count(), 1);
        let unmanaged = std::fs::read_to_string(dir.path().join("Unmanaged.java")).expect("read unmanaged file");
        assert!(!unmanaged.contains("auto-generated by alef"));
    }

    #[test]
    fn unchanged_write_preserves_mtime_and_reports_no_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "Example.java".into(),
                content: "public final class Example {}\n".into(),
                generated_header: true,
            }],
        )];
        let first = write_files_report(&files, dir.path()).expect("initial write");
        let path = dir.path().join("Example.java");
        let initial_mtime = std::fs::metadata(&path).expect("metadata").modified().expect("mtime");

        let second = write_files_report(&files, dir.path()).expect("repeat write");
        let repeated_mtime = std::fs::metadata(&path).expect("metadata").modified().expect("mtime");

        assert_eq!(first.changed_count(), 1);
        assert_eq!(second.changed_count(), 0);
        assert_eq!(first.expected_count(), 1);
        assert_eq!(second.expected_count(), 1);
        assert_eq!(initial_mtime, repeated_mtime);
    }

    #[test]
    fn duplicate_outputs_are_deduplicated_or_rejected_before_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = GeneratedFile {
            path: "Shared.java".into(),
            content: "final class Shared {}\n".into(),
            generated_header: true,
        };
        let identical = vec![
            (crate::core::config::Language::Java, vec![file.clone()]),
            (crate::core::config::Language::Kotlin, vec![file.clone()]),
        ];
        assert_eq!(
            write_files_report(&identical, dir.path())
                .expect("deduplicated")
                .changed_count(),
            1
        );

        let mut conflicting = file;
        conflicting.content = "final class Different {}\n".into();
        let conflict = vec![
            (crate::core::config::Language::Java, identical[0].1.clone()),
            (crate::core::config::Language::Kotlin, vec![conflicting]),
        ];
        let error = write_files_report(&conflict, dir.path()).expect_err("conflict must fail");
        assert!(
            error
                .to_string()
                .contains("multiple generators emitted different content")
        );
    }

    #[test]
    fn duplicate_conflict_does_not_create_output_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            (
                crate::core::config::Language::Java,
                vec![GeneratedFile {
                    path: "new/Shared.java".into(),
                    content: "first\n".into(),
                    generated_header: false,
                }],
            ),
            (
                crate::core::config::Language::Kotlin,
                vec![GeneratedFile {
                    path: "new/Shared.java".into(),
                    content: "second\n".into(),
                    generated_header: false,
                }],
            ),
        ];

        write_files_report(&files, dir.path()).expect_err("conflict must fail");

        assert!(!dir.path().join("new").exists());
    }

    #[cfg(unix)]
    #[test]
    fn changed_atomic_replacement_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tool.txt");
        std::fs::write(&path, "old\n").expect("seed file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o744)).expect("seed mode");
        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "tool.txt".into(),
                content: "new\n".into(),
                generated_header: false,
            }],
        )];

        write_files_report(&files, dir.path()).expect("replacement");

        assert_eq!(
            std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o744
        );
    }

    #[test]
    fn managed_output_paths_exclude_handwritten_emissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            GeneratedFile {
                path: "managed.rs".into(),
                content: "fn managed() {}\n".into(),
                generated_header: true,
            },
            GeneratedFile {
                path: "handwritten.rs".into(),
                content: "fn handwritten() {}\n".into(),
                generated_header: false,
            },
        ];

        let managed = managed_output_paths(&files, dir.path());
        let formatter_inputs = managed_generated_files(&files);

        assert_eq!(managed, [dir.path().join("managed.rs")].into_iter().collect());
        assert!(!managed.contains(&dir.path().join("handwritten.rs")));
        assert_eq!(formatter_inputs.len(), 1);
        assert_eq!(formatter_inputs[0].path, std::path::Path::new("managed.rs"));
    }

    #[test]
    fn managed_manifest_is_hashed_while_handwritten_manifest_stays_unowned() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            GeneratedFile {
                path: "managed/Cargo.toml".into(),
                content: "[package]\nname = \"managed\"\n".into(),
                generated_header: true,
            },
            GeneratedFile {
                path: "handwritten/Cargo.toml".into(),
                content: "[package]\nname = \"handwritten\"\n".into(),
                generated_header: false,
            },
        ];
        write_scaffold_files_with_overwrite(&files, dir.path(), true).expect("write manifests");
        let managed_paths = managed_output_paths(&files, dir.path());
        finalize_hashes(&managed_paths, "sources", b"config").expect("finalize managed manifest");

        let managed = std::fs::read_to_string(dir.path().join("managed/Cargo.toml")).expect("managed manifest");
        let handwritten =
            std::fs::read_to_string(dir.path().join("handwritten/Cargo.toml")).expect("handwritten manifest");
        assert!(crate::core::hash::extract_hash(&managed).is_some());
        assert!(crate::core::hash::extract_hash(&handwritten).is_none());
        assert!(!handwritten.contains("Generated by alef"));
    }
}
