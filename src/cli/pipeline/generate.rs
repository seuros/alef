mod diff;
mod generation;
mod header_freshness;
#[cfg(test)]
mod manifest_reconciliation_tests;
mod normalization;
mod orphans;
mod scaffold;
#[cfg(test)]
mod tests;
mod validation;
mod write;

pub use diff::diff_files;
pub use generation::{generate, generate_public_api, generate_service_api, generate_stubs};
pub(crate) use header_freshness::{check_ffi_header_freshness, ensure_ffi_header_freshness};
pub use normalization::normalize_content;
pub use orphans::{
    collect_alef_headered_paths, generate_sweep_roots, sweep_manifest_orphans, sweep_orphans, targeted_e2e_sweep_roots,
};
pub use scaffold::{
    readme, reconcile_managed_scaffold_manifests, scaffold, write_scaffold_files, write_scaffold_files_report,
    write_scaffold_files_with_overwrite,
};
pub use write::report_refused_writes;
pub(crate) use write::{
    apply_shebang_chmod, atomic_write, ensure_generated_header, marker_comment_style, provenance_header_for_path,
    stamp_for_adoption,
};
pub use write::{
    finalize_hashes, finalize_hashes_sweeping, managed_generated_files, managed_output_paths, write_files,
    write_files_report,
};

#[cfg(test)]
use normalization::{detect_crate_edition, parse_package_edition};
