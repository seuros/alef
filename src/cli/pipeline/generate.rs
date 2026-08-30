mod binary;
mod diff;
mod generation;
mod header_freshness;
#[cfg(test)]
mod manifest_reconciliation_tests;
mod normalization;
mod orphans;
mod scaffold;
mod scaffold_drift;
#[cfg(test)]
mod scaffold_lockfile_relock_tests;
#[cfg(test)]
mod scaffold_write_finalize_idempotency_tests;
#[cfg(test)]
mod tests;
mod user_owned;
#[cfg(test)]
mod user_owned_disposition_tests;
mod validation;
pub(crate) mod write;

use crate::core::backend::GeneratedFile;
use std::path::Path;

pub(crate) use binary::{decode_base64_binary, is_base64_binary_output};
pub use diff::diff_files;
pub use generation::{generate, generate_public_api, generate_service_api, generate_stubs};
pub(crate) use header_freshness::{check_ffi_header_freshness, ensure_ffi_header_freshness};
pub use normalization::normalize_content;
pub use orphans::{
    collect_alef_headered_paths, generate_sweep_roots, sweep_manifest_orphans, sweep_orphans, targeted_e2e_sweep_roots,
};
pub use scaffold::{readme, reconcile_managed_scaffold_manifests, scaffold};
pub use scaffold_drift::find_create_once_template_drift;
pub(crate) use user_owned::declared_user_owned;
pub use write::{WriteReport, report_refused_writes, report_user_owned_skips};
pub(crate) use write::{
    apply_shebang_chmod, atomic_write, ensure_generated_header, is_markable_path, is_owned_by_ownership_record,
    marker_comment_style, matches_alef_output, provenance_header_for_path, stamp_for_adoption,
};
pub use write::{
    finalize_hashes, finalize_hashes_after_tree_format, finalize_hashes_sweeping, managed_generated_files,
    managed_output_paths, stampable_output_paths, write_files, write_files_report,
};

#[cfg(test)]
use normalization::{detect_crate_edition, parse_package_edition};

/// Like [`scaffold::write_scaffold_files_report`], but also relocks any `Cargo.lock` sitting
/// beside an alef-generated `Cargo.toml` this call actually changed.
///
/// Every caller of `write_scaffold_files*` funnels through this one entry point (the other two
/// functions below both delegate here), so wrapping it covers `alef build`, `alef generate`,
/// `alef scaffold`, and version-sync's own scaffold regen alike -- see
/// [`super::version_lockfiles::relock_lockfiles_beside_changed_manifests`] for why this hook
/// exists and why it is scoped to only the manifests this call rewrote.
pub fn write_scaffold_files_report(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<WriteReport> {
    let report = scaffold::write_scaffold_files_report(files, base_dir, overwrite)?;
    super::version_lockfiles::relock_lockfiles_beside_changed_manifests(&report.changed_paths);
    super::version_lockfiles::relock_dart_lockfiles_beside_generated_manifests(files, base_dir, &report.changed_paths);
    Ok(report)
}

/// Like [`write_scaffold_files_report`] above but returns a bare changed-file count.
pub fn write_scaffold_files_with_overwrite(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<usize> {
    Ok(write_scaffold_files_report(files, base_dir, overwrite)?.changed_count())
}

/// Like [`write_scaffold_files_with_overwrite`] above with `overwrite: false` -- scaffold files
/// stay create-only, so an existing file a human may have grown past its placeholder is left
/// alone.
pub fn write_scaffold_files(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<usize> {
    write_scaffold_files_with_overwrite(files, base_dir, false)
}
