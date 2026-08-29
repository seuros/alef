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

/// Coverage for [`check_generated_node_lock_freshness`] / [`stale_node_lock_findings`], the pnpm
/// sibling of the checks above. Unlike the Rust fixtures, there is no path-dependency indirection
/// to reproduce: the specifiers being compared live in the one `package.json` alef generated, so
/// the fixtures below only need that file and a `pnpm-lock.yaml` beside it.
mod node {
    use super::*;

    const NODE_DIR_RELATIVE: &str = "e2e/typescript";
    const NODE_DEPENDENCY: &str = "sample-pkg";
    const NODE_STALE_SPEC: &str = "1.3.0";
    const NODE_FRESH_SPEC: &str = "1.2.3";

    fn node_dir(root: &Path) -> PathBuf {
        root.join(NODE_DIR_RELATIVE)
    }

    /// The alef-generated e2e `package.json`, matching the shape
    /// `crate::e2e::codegen::typescript::config::render_package_json` emits: the dependency under
    /// test sits in `devDependencies`.
    fn write_package_json(root: &Path, specifier: &str) -> PathBuf {
        let dir = node_dir(root);
        std::fs::create_dir_all(&dir).expect("create node dir");
        let manifest = dir.join("package.json");
        std::fs::write(
            &manifest,
            format!(
                "{{\n  \"name\": \"sample-pkg-e2e-typescript\",\n  \"version\": \"0.1.0\",\n  \"private\": \
                 true,\n  \"devDependencies\": {{\n    \"{NODE_DEPENDENCY}\": \"{specifier}\"\n  }}\n}}\n"
            ),
        )
        .expect("write package.json");
        manifest
    }

    /// `lockfileVersion` 9's workspace-aware shape: dependency tables nest under `importers.".".*`.
    fn write_pnpm_lock_v9(root: &Path, locked_specifier: &str) {
        std::fs::write(
            node_dir(root).join("pnpm-lock.yaml"),
            format!(
                "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
                 {NODE_DEPENDENCY}:\n        specifier: {locked_specifier}\n        version: \
                 {locked_specifier}\n"
            ),
        )
        .expect("write pnpm-lock.yaml");
    }

    /// `lockfileVersion` 6's flat, non-workspace shape: dependency tables sit at the document root
    /// with no `importers` wrapper at all -- the fallback `locked_node_specifiers` must also read.
    fn write_pnpm_lock_v6(root: &Path, locked_specifier: &str) {
        std::fs::write(
            node_dir(root).join("pnpm-lock.yaml"),
            format!(
                "lockfileVersion: '6.0'\n\ndevDependencies:\n  {NODE_DEPENDENCY}:\n    specifier: \
                 {locked_specifier}\n    version: {locked_specifier}\n"
            ),
        )
        .expect("write pnpm-lock.yaml");
    }

    /// The regression: `package.json` was regenerated with a specifier the committed
    /// `pnpm-lock.yaml` does not record, exactly the shape that fails `pnpm install` under the
    /// default frozen lockfile in CI. Before this module alef reported nothing and exited 0.
    #[test]
    fn stale_node_lock_findings_reports_a_specifier_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert_eq!(findings.len(), 1, "expected exactly one finding, got: {findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.dependency, NODE_DEPENDENCY);
        assert_eq!(finding.bucket, "devDependencies");
        assert_eq!(finding.requirement, NODE_STALE_SPEC);
        assert_eq!(finding.locked_requirement, NODE_FRESH_SPEC);
        assert_eq!(finding.lock, node_dir(root).join("pnpm-lock.yaml"));
        assert_eq!(finding.declared_in, node_dir(root).join("package.json"));
    }

    /// The control that stops "always fail" from satisfying this suite: the identical fixture
    /// with a lock that already records the same specifier must produce nothing at all. This is
    /// the one that would NOT fail if `stale_node_lock_findings` were reverted to always compare
    /// unconditionally correctly -- it instead catches a reversion that made the comparison
    /// unconditionally report (e.g. dropping the equality check).
    #[test]
    fn stale_node_lock_findings_accepts_a_lock_that_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_FRESH_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "a lock matching package.json must be reported clean: {findings:?}"
        );
    }

    /// `lockfileVersion` 6's flat shape (no `importers` wrapper) must be read too, not only 9's --
    /// this is the one that would fail if the `importers.".".*` fallback in
    /// `locked_node_specifiers` were the only shape read.
    #[test]
    fn stale_node_lock_findings_reports_a_mismatch_in_the_flat_lockfile_v6_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v6(root, NODE_FRESH_SPEC);

        let findings = stale_node_lock_findings(&node_dir(root));

        assert_eq!(
            findings.len(),
            1,
            "expected the flat lockfileVersion 6 shape to be read too, got: {findings:?}"
        );
        assert_eq!(findings[0].locked_requirement, NODE_FRESH_SPEC);
    }

    /// The one-sided rule, matching the cargo check's absence rule: a dependency package.json
    /// declares but the lock's own bucket never mentions is not reported. The lock here is
    /// non-empty (it pins an unrelated package) so this exercises the per-name lookup missing,
    /// not merely an empty bucket short-circuiting earlier.
    #[test]
    fn stale_node_lock_findings_ignores_a_dependency_absent_from_the_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        let dir = node_dir(root);
        std::fs::write(
            dir.join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      other-pkg:\n        \
             specifier: 2.0.0\n        version: 2.0.0\n",
        )
        .expect("write pnpm-lock.yaml");

        let findings = stale_node_lock_findings(&dir);

        assert!(
            findings.is_empty(),
            "a package missing from the lock's bucket is not evidence of drift: {findings:?}"
        );
    }

    /// `workspace:` specifiers are resolved through a workspace root this check never reads, so a
    /// text mismatch against the lock's recorded specifier must not be reported.
    #[test]
    fn stale_node_lock_findings_ignores_a_workspace_specifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, "workspace:*");
        write_pnpm_lock_v9(root, "workspace:^1.0.0");

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "workspace: specifiers are not directly comparable: {findings:?}"
        );
    }

    /// `file:` specifiers (the wasm e2e app's local dependency mode) are excluded for the same
    /// reason `fingerprint.rs` excludes `node_modules`/`vendor` from its own hash: a locally
    /// linked dependency's content, and potentially the text pnpm records for it, can move for
    /// reasons a text diff here cannot verify.
    #[test]
    fn stale_node_lock_findings_ignores_a_file_specifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, "file:../../..");
        write_pnpm_lock_v9(root, "file:../../../dist");

        let findings = stale_node_lock_findings(&node_dir(root));

        assert!(
            findings.is_empty(),
            "file: specifiers are not directly comparable: {findings:?}"
        );
    }

    /// The run-level entry point: it must select `package.json` out of the generated path set,
    /// and the error it returns must name the dependency, both specifiers, the lock, and the
    /// remedy.
    #[test]
    fn check_generated_node_lock_freshness_names_the_dependency_and_the_remedy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        let error = check_generated_node_lock_freshness(&generated).expect("a stale lock must fail the run");
        let message = format!("{error:#}");

        assert!(
            message.contains(NODE_DEPENDENCY),
            "message must name the dependency: {message}"
        );
        assert!(
            message.contains(NODE_STALE_SPEC),
            "message must name the package.json specifier: {message}"
        );
        assert!(
            message.contains(NODE_FRESH_SPEC),
            "message must name the locked specifier: {message}"
        );
        assert!(
            message.contains("pnpm install"),
            "message must name the remedy: {message}"
        );
        assert!(
            message.contains(&node_dir(root).join("pnpm-lock.yaml").display().to_string()),
            "message must name the lock: {message}"
        );
    }

    /// Control for the entry point, matching the pattern above: a lock whose specifier already
    /// matches must return `None` so the run keeps its zero exit. This is the assertion that
    /// would catch a regression turning this check into an unconditional failure.
    #[test]
    fn check_generated_node_lock_freshness_passes_a_matching_lock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let manifest = write_package_json(root, NODE_FRESH_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [manifest].into_iter().collect();

        assert!(
            check_generated_node_lock_freshness(&generated).is_none(),
            "a matching lock must not fail the run"
        );
    }

    /// A generated path set containing no `package.json` at all must not walk anything.
    #[test]
    fn check_generated_node_lock_freshness_ignores_non_manifest_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        write_package_json(root, NODE_STALE_SPEC);
        write_pnpm_lock_v9(root, NODE_FRESH_SPEC);
        let generated: HashSet<PathBuf> = [node_dir(root).join("src/index.ts")].into_iter().collect();

        assert!(check_generated_node_lock_freshness(&generated).is_none());
    }
}
