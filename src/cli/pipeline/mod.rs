mod cleanup;
mod commands;
mod extract;
mod format;
pub(crate) mod generate;
mod helpers;
mod version;
mod version_core;
mod version_csharp;
mod version_python;
mod version_regen;
mod version_registry;
mod version_swift;
mod version_text;
mod version_workspace;
mod workspace_lints;

pub use cleanup::cleanup_orphaned_files;
pub use commands::{build, clean, fmt, fmt_post_generate, lint, run_post_build, setup, test, test_apps_run, update};
pub use extract::extract;
pub use format::{format_generated, warn_missing_formatters};
pub(crate) use format::{install_poly_hooks, poly_format, poly_format_strict};
pub use generate::{
    WriteReport, collect_alef_headered_paths, diff_files, finalize_hashes, finalize_hashes_sweeping, generate,
    generate_public_api, generate_service_api, generate_stubs, generate_sweep_roots, managed_generated_files,
    managed_output_paths, normalize_content, readme, reconcile_managed_scaffold_manifests, report_refused_writes,
    scaffold, sweep_manifest_orphans, sweep_orphans, targeted_e2e_sweep_roots, write_files, write_files_report,
    write_scaffold_files, write_scaffold_files_report, write_scaffold_files_with_overwrite,
};
pub(crate) use generate::{
    apply_shebang_chmod, atomic_write, check_ffi_header_freshness, ensure_ffi_header_freshness,
    ensure_generated_header, provenance_header_for_path, stamp_for_adoption,
};
pub use helpers::{init, run_optional};
pub use version::sync_versions;
pub use version_core::{set_version, verify_versions};
pub(crate) use version_registry::sync_registry_package_versions;
pub use workspace_lints::ensure_workspace_alef_meta_check_cfg;
