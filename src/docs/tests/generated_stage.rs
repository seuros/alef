use super::*;
use std::fs;
use std::path::Path;

#[test]
fn docs_config_merges_workspace_and_crate_layers() {
    let cfg = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs]
reference_output = "docs/reference"

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.skills]
template_dir = "templates/skills"
outputs = [".codex/skills"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.docs.llms]
output = "llms.txt"
adopt_existing = true
"#,
    );

    let docs = cfg.docs.expect("docs config resolved");
    assert_eq!(
        docs.reference_output.as_deref(),
        Some(std::path::Path::new("docs/reference"))
    );
    assert_eq!(
        docs.llms.as_ref().and_then(|llms| llms.template.as_deref()),
        Some(std::path::Path::new("templates/llms.txt.jinja"))
    );
    assert_eq!(
        docs.llms.as_ref().and_then(|llms| llms.output.as_deref()),
        Some(std::path::Path::new("llms.txt"))
    );
    assert!(docs.llms.unwrap().adopt_existing);
    assert_eq!(
        docs.skills.unwrap().outputs,
        vec![std::path::PathBuf::from(".codex/skills")]
    );
}

#[test]
fn generate_docs_stage_renders_reference_llms_and_skills_from_templates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates/skills/api")).unwrap();
    fs::create_dir_all(root.join("templates/skills/cli")).unwrap();
    fs::create_dir_all(root.join("templates/skills/mcp")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/python")).unwrap();

    fs::write(
        root.join("src/cli.rs"),
        r#"
        use clap::{Parser, Subcommand};
        #[derive(Parser)]
        #[command(name = "mylib", about = "My CLI")]
        struct Cli {
            #[command(subcommand)]
            command: Commands,
        }
        #[derive(Subcommand)]
        enum Commands {
            /// Run the thing.
            Run {
                /// Input file
                input: String,
                /// Output file
                #[arg(short, long, value_name = "FILE")]
                output: Option<String>,
            },
        }
        "#,
    )
    .unwrap();
    fs::write(
        root.join("src/mcp.rs"),
        r#"
        struct Server;
        #[tool_router]
        impl Server {
            #[tool(description = "Run MCP work", annotations(title = "Run Work", read_only_hint = true))]
            async fn run_work(&self, Parameters(params): Parameters<crate::Params>) {}
        }
        "#,
    )
    .unwrap();
    fs::write(
        root.join("templates/llms.txt.jinja"),
        "Project {{ krate.name }} has {{ cli.commands | length }} CLI root and {{ mcp.tools | length }} MCP tools.\n{{ \"example.md\" | include_snippet(\"python\") }}\n",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/api/SKILL.md.jinja"),
        "# API Skill\n{% for ref in api_references %}- {{ ref.path }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/cli/SKILL.md.jinja"),
        "# CLI Skill\n{% for command in cli.commands %}- {{ command.path }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("templates/skills/mcp/SKILL.md.jinja"),
        "# MCP Skill\n{% for tool in mcp.tools %}- {{ tool.name }}\n{% endfor %}",
    )
    .unwrap();
    fs::write(
        root.join("docs/snippets/python/example.md"),
        "```python\nprint('ok')\n```\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.cli]
sources = ["src/cli.rs"]

[workspace.docs.mcp]
sources = ["src/mcp.rs"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.skills]
template_dir = "templates/skills"
outputs = [".codex/skills"]

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    result.unwrap();
    let paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(paths.contains(&"docs/reference/cli.md".to_string()));
    assert!(paths.contains(&"docs/reference/mcp.md".to_string()));
    assert!(paths.contains(&"docs/llms.txt".to_string()));
    assert!(paths.contains(&".codex/skills/api/SKILL.md".to_string()));
    assert!(paths.contains(&".codex/skills/cli/SKILL.md".to_string()));
    assert!(paths.contains(&".codex/skills/mcp/SKILL.md".to_string()));

    for group in ["api", "cli", "mcp"] {
        let path = PathBuf::from(format!(".codex/skills/{group}/SKILL.md"));
        let skill = files.iter().find(|file| file.path == path).unwrap();
        assert!(skill.content.starts_with(&format!("---\nname: \"{group}\"\n")));
        assert!(skill.content.contains("description: \"Use mylib"));
        assert!(skill.content.contains("auto-generated by alef"));
    }

    let llms = files
        .iter()
        .find(|file| file.path == Path::new("docs/llms.txt"))
        .unwrap();
    assert!(llms.content.contains("Project mylib has 1 CLI root and 1 MCP tools"));
    assert!(llms.content.contains("print('ok')"));
    assert!(llms.content.contains("auto-generated by alef"));

    let cli_doc = files
        .iter()
        .find(|file| file.path == Path::new("docs/reference/cli.md"))
        .unwrap();
    assert!(cli_doc.content.starts_with("---\ntitle: \"CLI Reference\"\n---"));
    assert!(
        cli_doc
            .content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"))
    );
    assert!(cli_doc.content.contains("`mylib run`"));
    assert!(cli_doc.content.contains("`--output`"));

    let api_doc = files
        .iter()
        .find(|file| file.path == Path::new("docs/reference/api-python.md"))
        .unwrap();
    assert!(api_doc.content.starts_with("---\n"));
    assert!(
        api_doc
            .content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"))
    );
}

#[test]
fn should_error_when_configured_docs_snippets_dir_does_not_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("templates/llms.txt.jinja"), "Project {{ krate.name }}\n").unwrap();
    // Note: `docs/snippets` is intentionally never created, mirroring the
    // shipped bug where `docs.snippets.dirs` pointed at a nonexistent path and
    // was silently filtered out instead of failing the build. ~keep

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.expect_err("a configured docs.snippets.dirs entry that does not exist must be a hard error");
    let message = err.to_string();
    assert!(
        message.contains("docs.snippets.dirs"),
        "error must name the config key, got: {message}"
    );
    assert!(
        message.contains("docs/snippets"),
        "error must name the offending configured path, got: {message}"
    );
    // The regression this whole change fixes: a snippet-stage failure must not discard the API
    // reference pages that were already fully rendered before snippet discovery ever ran. ~keep
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "already-rendered API reference pages must survive a later stage failure, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn should_error_with_actionable_message_when_snippet_reference_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/python")).unwrap();
    fs::write(
        root.join("templates/llms.txt.jinja"),
        "Project {{ krate.name }}\n{{ \"missing.py\" | include_snippet(\"python\") }}\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[workspace.docs.snippets]
dirs = ["docs/snippets"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.expect_err("an unresolvable snippet reference must fail, not silently omit content");
    let message = err.to_string();
    assert!(
        message.contains("python"),
        "error must name the language, got: {message}"
    );
    assert!(
        message.contains("missing.py"),
        "error must name the path, got: {message}"
    );
    assert!(
        message.contains("docs/snippets"),
        "error must name the root(s) that were searched, got: {message}"
    );
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "already-rendered API reference pages must survive a later llms-render failure, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

fn reference_output_config() -> ResolvedCrateConfig {
    config_from_toml(
        r#"
[workspace]
languages = ["python", "ffi"]

[workspace.docs]
reference_output = "docs-site/src/content/docs/reference"

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "literllm"
"#,
    )
}

/// A previously published reference page, carrying the `<!-- alef:hash: -->` line the docs
/// stage stamps into every page it emits, must be committed to disk. ~keep
fn stale_reference_page(version: &str) -> String {
    format!(
        "---\ntitle: \"C API Reference\"\n---\n\n\
         <!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
         <!-- alef:hash:0000000000000000000000000000000000000000000000000000000000000000 -->\n\n\
         ## C API Reference <span class=\"version-badge\">v{version}</span>\n"
    )
}

/// A full generation must land every API-reference page on disk, in the configured
/// `[docs] reference_output`.
///
/// The regression: `alef all --clean` regenerated the bindings and the 2821 committed
/// snippets but left all 15 `api-*.md` pages at the previous release's content, so the
/// published reference contradicted the published snippets on the same site. This drives the
/// real writer rather than only inspecting the returned `GeneratedFile`s, because the pages
/// went missing between emission and disk, not during rendering. ~keep
#[test]
fn full_generation_writes_api_reference_pages_into_configured_output() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let reference_output = root.join("docs-site/src/content/docs/reference");

    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python, Language::Ffi], None, root);
    result.unwrap();
    let written = crate::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();
    assert!(written > 0, "the docs stage must write files, wrote {written}");

    for page in ["api-python.md", "api-c.md", "configuration.md", "types.md", "errors.md"] {
        let path = reference_output.join(page);
        let content =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("{} was not written: {err}", path.display()));
        assert!(
            content.contains("auto-generated by alef"),
            "{page} must carry the generated-file header:\n{content}"
        );
    }
}

/// Regenerating over the previously published page must replace it, not preserve it.
///
/// `.md` sits outside `marker_comment_style`'s extension allowlist, so a reference page can
/// only ever be proved alef-owned by the `<!-- alef:hash: -->` line `finalize_hashes` stamps
/// into its *content* -- which every published page carries. An ownership guard that ignores
/// that in-content evidence for unstampable extensions and consults only the machine-local
/// `.alef/` record (empty on a fresh clone and in cache-less CI) refuses every reference page
/// forever, which is exactly the staleness this suite exists to prevent. ~keep
#[test]
fn stale_api_reference_page_is_regenerated_not_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let reference_output = root.join("docs-site/src/content/docs/reference");
    fs::create_dir_all(&reference_output).unwrap();
    fs::write(reference_output.join("api-c.md"), stale_reference_page("1.16.0")).unwrap();

    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Ffi], None, root);
    result.unwrap();
    crate::cli::pipeline::write_scaffold_files_with_overwrite(&files, root, true).unwrap();

    let page = fs::read_to_string(reference_output.join("api-c.md")).unwrap();
    assert!(
        page.contains("<span class=\"version-badge\">v1.17.1</span>"),
        "the page must be regenerated at the current version:\n{page}"
    );
    assert!(
        !page.contains("v1.16.0"),
        "the previous release's content must not survive a full regeneration:\n{page}"
    );
}

/// Every emitted page must badge the resolved workspace version that produced it.
#[test]
fn generated_api_reference_badges_the_resolved_version() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = reference_output_config();
    let api = make_minimal_api("1.17.1");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python, Language::Ffi], None, root);
    result.unwrap();

    for slug in ["api-python.md", "api-c.md"] {
        let page = files
            .iter()
            .find(|file| file.path.file_name().is_some_and(|name| name.to_string_lossy() == slug))
            .unwrap_or_else(|| panic!("missing {slug}"));
        assert!(
            page.content.contains("<span class=\"version-badge\">v1.17.1</span>"),
            "{slug} must badge the resolved version:\n{}",
            page.content
        );
    }
}

/// The C page must spell symbols exactly as the generated header declares them.
///
/// `[ffi] prefix = "literllm"` makes the FFI backend set cbindgen's `[export] prefix` to
/// `LITERLLM`, so `liter_llm.h` declares `LITERLLMDefaultClient`. The reference page used to
/// publish `LiterllmDefaultClient`, a spelling that occurs zero times in the emitted header. ~keep
#[test]
fn generated_c_reference_uses_the_header_symbol_spelling() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = reference_output_config();
    let export_prefix = config.ffi_prefix().to_uppercase();

    let mut api = make_minimal_api("1.17.1");
    api.types = vec![empty_type("DefaultClient")];
    api.functions = vec![make_function(
        "create_client",
        vec![],
        TypeRef::Named("DefaultClient".to_string()),
        false,
        None,
    )];

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Ffi], None, root);
    result.unwrap();
    let page = files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == "api-c.md")
        })
        .expect("missing api-c.md");

    assert!(
        page.content.contains(&format!("{export_prefix}DefaultClient")),
        "C page must use the cbindgen export prefix `{export_prefix}`:\n{}",
        page.content
    );
    assert!(
        !page.content.contains("LiterllmDefaultClient"),
        "the PascalCase prefix appears nowhere in the generated header:\n{}",
        page.content
    );
}

#[test]
fn generate_docs_stage_rejects_unmanaged_llms_output_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("templates/llms.txt.jinja"), "Generated {{ krate.name }}\n").unwrap();
    fs::write(root.join("docs/llms.txt"), "human authored\n").unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.llms]
template = "templates/llms.txt.jinja"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not Alef-managed"));
    assert!(
        files
            .iter()
            .any(|file| file.path.to_string_lossy().replace('\\', "/") == "docs/reference/api-python.md"),
        "an unmanaged llms.txt must not discard the already-rendered API reference pages, got: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}
