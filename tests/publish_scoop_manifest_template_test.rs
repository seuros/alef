//! Non-vacuous regression proving `scripts/publish/alef.json.tmpl` renders into the exact
//! nested Scoop manifest shape `xberg-io/scoop-bucket` validates against -- specifically that
//! `architecture.64bit.url`, `.hash`, and `.extract_dir` are the fields that carry the
//! release's real URL/hash/extract directory, not flat top-level `url`/`hash`/`extract_dir`
//! fields.
//!
//! A prior, since-deleted ad-hoc updater (`scripts/publish/update-scoop-manifest.sh`) wrote
//! only top-level `url`/`hash`/`extract_dir` on an existing manifest and left
//! `architecture.64bit` untouched -- but Scoop reads the `architecture.64bit` override, so
//! that script would have bumped the top-level `version` while silently continuing to install
//! whatever binary the manifest's `architecture.64bit` block already pointed at. `check-scoop`
//! (src/cli/commands/check_registry.rs) only compares the top-level `version` field, so that
//! bug would have reported success. A test that only checks top-level fields would have passed
//! against that broken script, so `flat_top_level_url_hash_extract_dir_do_not_exist` below
//! asserts those keys are absent, not merely that the nested ones are present.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn template_path() -> PathBuf {
    repo_root().join("scripts/publish/alef.json.tmpl")
}

// ~keep Mirrors the exact substitution mechanism `xberg-io/actions/publish-scoop-manifests@v1`
// documents using (Python's `string.Template`, per scoop-bucket/MANIFESTS.md), so this test
// renders the same way the real action would. This is a verification harness for the
// template's own shape, not a reimplementation of the action's production logic (asset
// resolution, `gh release download`, hashing, git commit/push) -- those stay owned by the
// action, per the "do NOT duplicate its rendering logic locally" requirement.
const RENDER_SCRIPT: &str = r#"
import json
import string
import sys

with open(sys.argv[1]) as f:
    tmpl = string.Template(f.read())
rendered = tmpl.substitute(version=sys.argv[2], tag=sys.argv[3], win_x64_sha=sys.argv[4])
print(rendered)
"#;

const TEST_VERSION: &str = "9.9.9";
const TEST_TAG: &str = "v9.9.9";
const TEST_SHA: &str = "b4c1c1a2f1e5a6d7c8b9a0f1e2d3c4b5a6978869504132211009988776655aa";

fn render(version: &str, tag: &str, win_x64_sha: &str) -> serde_json::Value {
    let output = Command::new("python3")
        .arg("-c")
        .arg(RENDER_SCRIPT)
        .arg(template_path())
        .arg(version)
        .arg(tag)
        .arg(win_x64_sha)
        .output()
        .expect("python3 must run to render alef.json.tmpl");
    assert!(
        output.status.success(),
        "template render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("rendered template is not valid JSON ({error}):\n{stdout}"))
}

#[test]
fn hash_and_url_and_extract_dir_land_under_architecture_64bit() {
    let manifest = render(TEST_VERSION, TEST_TAG, TEST_SHA);
    let arch = &manifest["architecture"]["64bit"];

    assert_eq!(
        arch["url"],
        serde_json::json!(format!(
            "https://github.com/xberg-io/alef/releases/download/{TEST_TAG}/alef-x86_64-pc-windows-msvc.zip"
        )),
        "architecture.64bit.url must carry the release's real download URL"
    );
    assert_eq!(
        arch["hash"],
        serde_json::json!(TEST_SHA),
        "architecture.64bit.hash must carry the locally-computed SHA256, not a stale value"
    );
    assert_eq!(
        arch["extract_dir"],
        serde_json::json!("alef-x86_64-pc-windows-msvc"),
        "architecture.64bit.extract_dir must match build-cli's own archive directory name"
    );
}

#[test]
fn flat_top_level_url_hash_extract_dir_do_not_exist() {
    let manifest = render(TEST_VERSION, TEST_TAG, TEST_SHA);
    assert!(manifest.get("url").is_none(), "url must not exist at the top level");
    assert!(manifest.get("hash").is_none(), "hash must not exist at the top level");
    assert!(
        manifest.get("extract_dir").is_none(),
        "extract_dir must not exist at the top level"
    );
}

#[test]
fn version_is_top_level_and_matches_the_release() {
    let manifest = render(TEST_VERSION, TEST_TAG, TEST_SHA);
    assert_eq!(manifest["version"], serde_json::json!(TEST_VERSION));
}

#[test]
fn required_scoop_bucket_fields_are_present() {
    // xberg-io/scoop-bucket/MANIFESTS.md requires these exact fields on every manifest.
    let manifest = render(TEST_VERSION, TEST_TAG, TEST_SHA);
    assert_eq!(
        manifest["$schema"],
        serde_json::json!("https://raw.githubusercontent.com/ScoopInstaller/Scoop/master/schema.json")
    );
    assert_eq!(manifest["bin"], serde_json::json!("alef.exe"));
    assert_eq!(
        manifest["suggest"]["vcredist"],
        serde_json::json!("extras/vcredist2022")
    );
    assert_eq!(
        manifest["checkver"]["github"],
        serde_json::json!("https://github.com/xberg-io/alef")
    );
}

#[test]
fn autoupdate_url_keeps_the_literal_scoop_version_placeholder() {
    // The template writes `$$version` so the renderer collapses it to a literal `$version` --
    // Scoop's own autoupdate feature expands that placeholder itself, later, against the
    // published release tags. If this test ever sees the *substituted* version string instead
    // of the literal placeholder, the escaping was dropped and autoupdate is permanently
    // frozen on this one release.
    let manifest = render(TEST_VERSION, TEST_TAG, TEST_SHA);
    let autoupdate_url = manifest["autoupdate"]["architecture"]["64bit"]["url"]
        .as_str()
        .expect("autoupdate.architecture.64bit.url must be a string");

    assert!(
        autoupdate_url.contains("$version"),
        "autoupdate URL must retain the literal $version placeholder, got: {autoupdate_url}"
    );
    assert!(
        !autoupdate_url.contains(TEST_VERSION),
        "autoupdate URL must not contain the substituted version -- escaping was lost, got: {autoupdate_url}"
    );
}
