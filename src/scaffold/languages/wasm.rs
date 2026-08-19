use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::scaffold::scaffold_meta;
use anyhow::Context as _;
use regex::Regex;
use std::path::{Path, PathBuf};

pub(crate) fn scaffold_wasm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();

    let mut files = vec![];

    let wasm_pkg_name = config.wasm_package_name();

    let core_crate_file = core_crate_dir.replace('-', "_");
    let repository_block = meta
        .configured_repository
        .as_deref()
        .map(|repository| {
            format!(
                r#",
  "repository": {{
    "type": "git",
    "url": "{repository}",
    "directory": "crates/{core_crate_dir}-wasm"
  }}"#
            )
        })
        .unwrap_or_default();
    let license_block = meta
        .license
        .as_deref()
        .map(|license| format!(",\n  \"license\": \"{license}\""))
        .unwrap_or_default();

    // wasm-pack build targets that ship in this package. Each target embeds a ~keep
    // full copy of the wasm binary, so a single-target set keeps the published ~keep
    // package small. Derives `files`, entry points, and the build scripts. ~keep
    let targets = config.wasm_targets();
    const VALID_TARGETS: &[&str] = &["web", "bundler", "nodejs", "deno"];
    if targets.is_empty() {
        anyhow::bail!("[crates.wasm].targets must list at least one wasm-pack target (web, bundler, nodejs, deno)");
    }
    for target in &targets {
        if !VALID_TARGETS.contains(&target.as_str()) {
            anyhow::bail!("[crates.wasm].targets: unknown target '{target}' (valid: web, bundler, nodejs, deno)");
        }
    }
    let has = |t: &str| targets.iter().any(|x| x == t);
    // `main`/`types` prefer the CommonJS-friendly nodejs build; `module` prefers ~keep
    // the browser ES module. When a preferred target isn't built, fall back to ~keep
    // the first configured target. ~keep
    let node_target = if has("nodejs") { "nodejs" } else { targets[0].as_str() };
    let web_target = if has("web") { "web" } else { targets[0].as_str() };
    let exports_block = crate::scaffold::template_env::render(
        "wasm_package_exports.json.jinja",
        minijinja::context! {
            node_target => node_target,
            web_target => web_target,
            crate_file => core_crate_file,
        },
    );

    // A single-target package publishes just that target's dir; a multi-target ~keep
    // package keeps the broad glob for backward compatibility. ~keep
    let files_block = if targets.len() == 1 {
        format!("[\n    \"pkg/{}\",\n    \"README.md\"\n  ]", targets[0])
    } else {
        "[\n    \"pkg\",\n    \"*.wasm\",\n    \"*.d.ts\",\n    \"README.md\"\n  ]".to_string()
    };

    let per_target_scripts: String = targets
        .iter()
        .map(|t| format!("    \"build:wasm:{t}\": \"wasm-pack build --release --target {t} --out-dir pkg/{t}\",\n"))
        .collect();
    let build_all = targets
        .iter()
        .map(|t| format!("npm run build:wasm:{t}"))
        .collect::<Vec<_>>()
        .join(" && ");

    let pkg_json = format!(
        r#"{{
  "name": "{wasm_pkg_name}",
  "version": "{version}",
  "private": false,
  "description": "{description}"{license_block}{repository_block},
  "publishConfig": {{
    "access": "public"
  }},
  "type": "module",
  "files": {files_block},
  "main": "pkg/{node_target}/{core_crate_file}_wasm.js",
  "module": "pkg/{web_target}/{core_crate_file}_wasm.js",
  "types": "pkg/{node_target}/{core_crate_file}_wasm.d.ts",
  {exports_block}  "engines": {{
    "node": "{node_engine}"
  }},
  "scripts": {{
    "build": "wasm-pack build --target {node_target} --out-dir pkg/{node_target}",
    "build:ci": "wasm-pack build --release --target {node_target} --out-dir pkg/{node_target}",
{per_target_scripts}    "build:all": "{build_all} && find pkg -name .gitignore -delete",
    "test": "vitest run",
    "test:watch": "vitest watch",
    "test:coverage": "vitest run --coverage",
    "clean": "rm -rf pkg dist"
  }}
}}
"#,
        wasm_pkg_name = wasm_pkg_name,
        version = version,
        description = meta.description,
        license_block = license_block,
        repository_block = repository_block,
        files_block = files_block,
        node_target = node_target,
        web_target = web_target,
        core_crate_file = core_crate_file,
        exports_block = exports_block,
        node_engine = tv::npm::NODE_ENGINE,
        per_target_scripts = per_target_scripts,
        build_all = build_all,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}

/// Repair a pre-existing `crates/<crate>-wasm/package.json` that predates the `exports` map
/// [`scaffold_wasm`] now emits (added in the fix that also introduced
/// `wasm_package_exports.json.jinja`).
///
/// `crates/*-wasm/package.json` is `generated_header: false` (create-only: see
/// `write_scaffold_files_report`'s ownership guard in `cli::pipeline::generate::scaffold`), so a
/// repo scaffolded before the `exports` map existed keeps shipping a `package.json` with no
/// `exports` key forever — `require()`/`import` resolution under Node's package-exports
/// enforcement then falls back to legacy `main`/`module` resolution, which still works for the
/// package root but leaves any consumer relying on subpath/conditional exports (`browser`,
/// dual CJS/ESM `require`+`import`) unresolvable. A full regenerate-and-overwrite is not
/// attempted: `package.json` is exactly the kind of file consumers hand-edit (extra
/// `devDependencies`, custom `scripts`), so this only ever *inserts* the missing block, never
/// touches anything else on the line, and refuses outright rather than guess when the file
/// doesn't unambiguously carry alef's own `main`/`module`/`types` shape. See
/// [`repair_missing_wasm_exports`] for the exact detection and insertion. ~keep
pub(crate) fn migrate_wasm_package_json_exports(base_dir: &Path, relative_path: &Path) -> anyhow::Result<bool> {
    let path = base_dir.join(relative_path);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Some(migrated) = repair_missing_wasm_exports(&existing) else {
        return Ok(false);
    };
    if migrated == existing {
        return Ok(false);
    }

    let parent = path
        .parent()
        .context("wasm package.json path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing crates/*-wasm/package.json: inserted the missing \"exports\" map"
    );
    Ok(true)
}

/// Pure text transform behind [`migrate_wasm_package_json_exports`]. Returns `None` when
/// `content` is not a safe migration candidate at all: it already has an `"exports"` key
/// (nothing missing — whether that's this fix's own output or a consumer's hand-added one, either
/// way there is nothing to insert without risking a duplicate or clobbering a custom map), or it
/// does not carry the exact `"main"`/`"module"`/`"types"` triple `scaffold_wasm` emits (so this
/// isn't provably alef's own package.json shape at all), or the file has no `"engines": {` line to
/// anchor the insertion before (the one point in the template `scaffold_wasm` always emits the
/// block directly ahead of).
///
/// `node_target`/`web_target`/`crate_file` for the newly rendered block are extracted from the
/// file's own `main`/`module`/`types` fields, not recomputed from live config — the values already
/// on disk are definitionally consistent with the rest of the file, so reusing them (rather than
/// asking the current `ResolvedCrateConfig` what it thinks the targets are today) can never
/// disagree with the paths those existing `main`/`module`/`types` lines already point at.
fn repair_missing_wasm_exports(content: &str) -> Option<String> {
    if content.contains("\"exports\":") {
        return None;
    }

    let main_pattern = Regex::new(r#""main":\s*"pkg/([^/"]+)/([^"]+)_wasm\.js""#).expect("valid regex");
    let module_pattern = Regex::new(r#""module":\s*"pkg/([^/"]+)/([^"]+)_wasm\.js""#).expect("valid regex");
    let types_pattern = Regex::new(r#""types":\s*"pkg/([^/"]+)/([^"]+)_wasm\.d\.ts""#).expect("valid regex");

    let main_captures = main_pattern.captures(content)?;
    let module_captures = module_pattern.captures(content)?;
    let types_captures = types_pattern.captures(content)?;

    let node_target = main_captures.get(1)?.as_str();
    let crate_file = main_captures.get(2)?.as_str();
    let web_target = module_captures.get(1)?.as_str();
    if module_captures.get(2)?.as_str() != crate_file {
        return None;
    }
    if types_captures.get(1)?.as_str() != node_target || types_captures.get(2)?.as_str() != crate_file {
        return None;
    }

    let engines_line_index = content.lines().position(|line| line.trim() == "\"engines\": {")?;

    let exports_block = crate::scaffold::template_env::render(
        "wasm_package_exports.json.jinja",
        minijinja::context! {
            node_target => node_target,
            web_target => web_target,
            crate_file => crate_file,
        },
    );
    // The template's first line carries no leading indent of its own -- `scaffold_wasm` supplies
    // it via the two literal spaces ahead of `{exports_block}` in its format string -- so that
    // indent has to be added back here for the block to line up once spliced in as whole lines.
    // Every later line already bakes in its own absolute indent (matching that same 2-space
    // base), so only the first line needs it. ~keep
    let mut export_lines: Vec<String> = exports_block.lines().map(str::to_string).collect();
    if let Some(first) = export_lines.first_mut() {
        *first = format!("  {first}");
    }

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    lines.splice(engines_line_index..engines_line_index, export_lines);
    let mut joined = lines.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    /// The exact shape `scaffold_wasm` emitted before the fix that added the `exports` map --
    /// a single `nodejs` target, `main`/`module`/`types` all pointing at the same crate file.
    fn pre_fix_package_json() -> String {
        "{\n  \
         \"name\": \"@scope/example-wasm\",\n  \
         \"version\": \"1.0.0\",\n  \
         \"private\": false,\n  \
         \"description\": \"An example crate\",\n  \
         \"publishConfig\": {\n    \"access\": \"public\"\n  },\n  \
         \"type\": \"module\",\n  \
         \"files\": [\n    \"pkg/nodejs\",\n    \"README.md\"\n  ],\n  \
         \"main\": \"pkg/nodejs/example_wasm.js\",\n  \
         \"module\": \"pkg/nodejs/example_wasm.js\",\n  \
         \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n  \
         \"engines\": {\n    \"node\": \">=18\"\n  },\n  \
         \"scripts\": {\n    \"build\": \"wasm-pack build\"\n  }\n\
         }\n"
        .to_string()
    }

    #[test]
    fn should_insert_exports_map_when_missing_from_alef_authored_package_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-wasm");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
        std::fs::write(pkg_dir.join("package.json"), pre_fix_package_json()).expect("write pre-fix package.json");

        let relative_path = Path::new("crates/example-wasm/package.json");
        let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
        assert!(changed, "a package.json missing exports must be reported as changed");

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
        let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
        assert_eq!(
            parsed["exports"]["."]["types"], "./pkg/nodejs/example_wasm.d.ts",
            "exports map must reference the same target/crate_file as the existing main/module/types fields"
        );
        assert_eq!(parsed["exports"]["."]["require"], "./pkg/nodejs/example_wasm.js");
        assert_eq!(
            parsed["name"], "@scope/example-wasm",
            "fields outside exports must survive untouched"
        );
        assert_eq!(
            parsed["scripts"]["build"], "wasm-pack build",
            "user-visible fields must survive untouched"
        );

        let changed_again =
            migrate_wasm_package_json_exports(dir.path(), relative_path).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_not_touch_a_package_json_that_already_has_exports() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-wasm");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
        let hand_written = "{\n  \"name\": \"@scope/example-wasm\",\n  \"exports\": \"./custom.js\"\n}\n";
        std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write hand-edited package.json");

        let relative_path = Path::new("crates/example-wasm/package.json");
        let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
        assert!(
            !changed,
            "a package.json that already declares exports must never be touched"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
        assert_eq!(
            on_disk, hand_written,
            "a custom exports field must survive byte-for-byte"
        );
    }

    #[test]
    fn should_not_touch_a_foreign_package_json_without_the_alef_wasm_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("crates/example-wasm");
        std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
        let hand_written = "{\n  \"name\": \"example\",\n  \"main\": \"index.js\",\n  \"engines\": {\n    \"node\": \">=18\"\n  }\n}\n";
        std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write foreign package.json");

        let relative_path = Path::new("crates/example-wasm/package.json");
        let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
        assert!(
            !changed,
            "a package.json without alef's main/module/types shape must never be touched"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
        assert_eq!(
            on_disk, hand_written,
            "a foreign package.json must survive byte-for-byte"
        );
    }

    #[test]
    fn migrate_wasm_package_json_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative_path = Path::new("crates/example-wasm/package.json");
        let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join(relative_path).exists());
    }
}
