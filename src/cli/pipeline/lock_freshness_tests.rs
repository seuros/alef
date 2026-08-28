//! Coverage for the generated-manifest / committed-lock disagreement check.
//!
//! Every fixture reproduces the reported shape structurally: an alef-generated Rust e2e crate
//! that is its own workspace root and reaches its real registry requirements through a *path*
//! dependency it does not own. That indirection is the whole point — it is why the pre-existing
//! `relock_lockfiles_beside_changed_manifests` hook (keyed on "did alef rewrite this manifest")
//! cannot see the breakage, and why nothing observed lock freshness at all before this module.
//!
//! No cargo invocation anywhere: the fixtures are plain files and the check is pure.

use super::*;

const E2E_RELATIVE_DIR: &str = "e2e/rust";

/// Package name of the crate under test, and of the lock entry the fixtures move.
const REGISTRY_DEPENDENCY: &str = "sample-json";

/// The lock pins this; the manifests below require one minor above it in the stale fixtures.
const STALE_PIN: &str = "1.25.0";
const FRESH_PIN: &str = "1.26.0";
const REQUIREMENT: &str = "1.26";

/// Root workspace crate that the generated e2e crate depends on by path.
fn write_root_manifest(root: &Path, dependencies: &str) {
    std::fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{dependencies}"),
    )
    .expect("write root Cargo.toml");
}

/// The alef-generated e2e crate: its own workspace root, depending on the crate under test by
/// path exactly as `crate::e2e::codegen::rust::cargo_toml` emits it.
fn write_generated_e2e_manifest(root: &Path) -> PathBuf {
    let dir = root.join(E2E_RELATIVE_DIR);
    std::fs::create_dir_all(&dir).expect("create e2e dir");
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[workspace]\n\n[package]\nname = \"sample-core-e2e-rust\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\nsample_core = { package = \"sample-core\", path = \"../..\" }\n",
    )
    .expect("write generated e2e Cargo.toml");
    manifest
}

/// A committed lock beside the generated manifest, pinning the registry dependency at `pin`.
fn write_lock(root: &Path, pin: &str) {
    std::fs::write(
        root.join(E2E_RELATIVE_DIR).join("Cargo.lock"),
        format!(
            "version = 4\n\n\
             [[package]]\nname = \"sample-core-e2e-rust\"\nversion = \"0.1.0\"\n\n\
             [[package]]\nname = \"sample-core\"\nversion = \"0.1.0\"\n\n\
             [[package]]\nname = \"{REGISTRY_DEPENDENCY}\"\nversion = \"{pin}\"\n"
        ),
    )
    .expect("write Cargo.lock");
}

fn e2e_dir(root: &Path) -> PathBuf {
    root.join(E2E_RELATIVE_DIR)
}

/// The regression: the generated manifest is byte-identical to what alef would emit, nothing
/// alef owns changed, and the lock still cannot satisfy a requirement the path dependency
/// declares. `cargo metadata --locked` fails here; before this module alef exited 0.
#[test]
fn stale_lock_findings_reports_a_requirement_no_locked_version_satisfies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.dependency, REGISTRY_DEPENDENCY);
    assert_eq!(finding.requirement, REQUIREMENT);
    assert_eq!(finding.locked_versions, vec![STALE_PIN.to_string()]);
    assert_eq!(finding.lock, e2e_dir(root).join("Cargo.lock"));
    assert_eq!(
        finding.declared_in,
        root.join("Cargo.toml"),
        "the requirement is declared in the path dependency, not in the manifest alef generated"
    );
}

/// The control that stops "always fail" from satisfying this suite: the identical fixture with a
/// lock that does satisfy the requirement must produce nothing at all.
#[test]
fn stale_lock_findings_accepts_a_lock_that_satisfies_every_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, FRESH_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a lock that resolves must be reported clean: {findings:?}"
    );
}

/// The one-sided rule: a requirement whose package is not in the lock at all is never reported.
/// Absence is ambiguous (trimmed dev-dependencies, `[patch]`, platform gating) and reporting it
/// would turn healthy trees red; only a contradiction cargo itself would reject is a finding.
#[test]
fn stale_lock_findings_ignores_a_dependency_absent_from_the_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(root, "[dependencies]\nsample-absent = \"2\"\n");
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a package missing from the lock is not evidence of staleness: {findings:?}"
    );
}

/// The common real-world spelling: the path dependency inherits its requirement from the
/// workspace root it is itself the root of. Resolving inheritance is what keeps this check from
/// being blind on most consumer repos.
#[test]
fn stale_lock_findings_resolves_a_workspace_inherited_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[workspace]\nmembers = []\n\n[workspace.dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n\n\
             [dependencies]\n{REGISTRY_DEPENDENCY} = {{ workspace = true }}\n"
        ),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert_eq!(
        findings.len(),
        1,
        "expected the inherited requirement, got: {findings:?}"
    );
    assert_eq!(findings[0].dependency, REGISTRY_DEPENDENCY);
    assert_eq!(findings[0].requirement, REQUIREMENT);
}

/// A git dependency is locked by revision; the `version` field beside it is not a registry
/// requirement the lock's pinned version has to satisfy.
#[test]
fn stale_lock_findings_ignores_a_git_dependency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!(
            "[dependencies]\n{REGISTRY_DEPENDENCY} = {{ git = \"https://example.invalid/sample.git\", version = \
             \"{REQUIREMENT}\" }}\n"
        ),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a git dependency carries no registry pin: {findings:?}"
    );
}

/// Alef never authors a lockfile. A generated crate without one is a consumer choice, not a
/// defect, and must not fail the run.
#[test]
fn stale_lock_findings_skips_a_directory_with_no_committed_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);

    let findings = stale_lock_findings(&e2e_dir(root));

    assert!(
        findings.is_empty(),
        "a directory with no lock has nothing to check: {findings:?}"
    );
}

/// The run-level entry point: it must select manifests out of the generated path set, and the
/// error it returns must name the dependency, the lock, and the command that fixes it.
#[test]
fn check_generated_lock_freshness_names_the_dependency_and_the_remedy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    let manifest = write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [manifest, root.join(E2E_RELATIVE_DIR).join("tests/basic_test.rs")]
        .into_iter()
        .collect();

    let error = check_generated_lock_freshness(&generated).expect("a stale lock must fail the run");
    let message = format!("{error:#}");

    assert!(
        message.contains(REGISTRY_DEPENDENCY),
        "message must name the dependency: {message}"
    );
    assert!(
        message.contains(STALE_PIN),
        "message must name the stale pin: {message}"
    );
    assert!(
        message.contains(REQUIREMENT),
        "message must name the requirement: {message}"
    );
    assert!(
        message.contains("cargo update"),
        "message must name the remedy: {message}"
    );
    assert!(
        message.contains(&e2e_dir(root).join("Cargo.lock").display().to_string()),
        "message must name the lock: {message}"
    );
}

/// Control for the entry point, matching the `stale_lock_findings` control above: a resolvable
/// lock must return `None` so the run keeps its zero exit.
#[test]
fn check_generated_lock_freshness_passes_a_resolvable_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    let manifest = write_generated_e2e_manifest(root);
    write_lock(root, FRESH_PIN);
    let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

    assert!(
        check_generated_lock_freshness(&generated).is_none(),
        "a resolvable lock must not fail the run"
    );
}

/// A generated path set containing no Rust manifest at all must not walk anything.
#[test]
fn check_generated_lock_freshness_ignores_non_manifest_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_root_manifest(
        root,
        &format!("[dependencies]\n{REGISTRY_DEPENDENCY} = \"{REQUIREMENT}\"\n"),
    );
    write_generated_e2e_manifest(root);
    write_lock(root, STALE_PIN);
    let generated: HashSet<PathBuf> = [root.join(E2E_RELATIVE_DIR).join("tests/basic_test.rs")]
        .into_iter()
        .collect();

    assert!(check_generated_lock_freshness(&generated).is_none());
}
