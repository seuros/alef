//! Structural guard for the render-time `~keep` strip.
//!
//! `~keep` is a marker for `poly`'s uncomment pass and is meaningful only in a source tree
//! `poly` reads. Template authors write it inside `.jinja` sources so alef's own `poly fmt`
//! spares the rationale, and before the strip existed minijinja rendered it verbatim into
//! consumer trees. The unit tests in `core::keep_marker` pin the strip's behaviour; this file
//! pins that every built-in render path still calls it, because the failure mode that
//! produced the leak is a render path that never had the call, not a strip that regressed.
//!
//! A new backend is added by copying an existing `template_env.rs`, so the copy inherits the
//! call — but a hand-written one would not, and nothing else in the build would notice. ~keep

use std::fs;
use std::path::{Path, PathBuf};

const STRIP_CALL: &str = "strip_keep_markers";

/// Renders consumer-supplied extension templates rather than alef's own embedded ones. A
/// consumer writing `~keep` in their template is asking for it in their own output, and their
/// `poly` run is the one that reads it — so this path deliberately does not strip. ~keep
const EXTENSION_FACING_ENV: &str = "src/core/template_env.rs";

/// Below the number of `template_env.rs` modules that existed when this guard was written.
/// A walk that silently stops finding files would otherwise pass while examining nothing.
const MINIMUM_EXPECTED_ENVS: usize = 20;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_template_envs(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let entry = entry.expect("read source directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_template_envs(&path, found);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("template_env.rs") {
            found.push(path);
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("path under repository root")
        .to_str()
        .expect("path is valid UTF-8")
        .replace('\\', "/")
}

#[test]
fn every_builtin_template_env_applies_the_keep_marker_strip() {
    let root = repository_root();
    let mut envs = Vec::new();
    collect_template_envs(&root.join("src"), &mut envs);
    envs.sort();

    assert!(
        envs.len() >= MINIMUM_EXPECTED_ENVS,
        "expected at least {MINIMUM_EXPECTED_ENVS} template_env.rs modules, found {}: {envs:?} -- \
         the walk is examining nothing, not passing",
        envs.len()
    );

    let mut missing = Vec::new();
    let mut checked = 0usize;
    for path in &envs {
        let name = relative(&root, path);
        if name == EXTENSION_FACING_ENV {
            continue;
        }
        checked += 1;
        let source = fs::read_to_string(path).expect("read template_env source");
        if !source.contains(STRIP_CALL) {
            missing.push(name);
        }
    }

    assert!(checked > 0, "no built-in template_env module was checked");
    assert!(
        missing.is_empty(),
        "these built-in template render paths do not call `{STRIP_CALL}`, so `~keep` markers in \
         their templates reach generated consumer code: {missing:?}"
    );
}

#[test]
fn the_extension_facing_template_env_does_not_strip_consumer_markers() {
    let root = repository_root();
    let path = root.join(EXTENSION_FACING_ENV);
    let source = fs::read_to_string(&path).expect("read extension-facing template_env source");

    assert!(
        source.contains("pub fn render"),
        "{EXTENSION_FACING_ENV} no longer exposes a render entry point; this exemption needs revisiting"
    );
    assert!(
        !source.contains(STRIP_CALL),
        "{EXTENSION_FACING_ENV} renders consumer-supplied templates -- stripping `~keep` there would \
         delete a marker the consumer wrote for their own uncomment pass"
    );
}
