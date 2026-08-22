//! The stamping seam a whole-tree formatter pass must be paired with.
//!
//! Split out of `write.rs` rather than added to it: that file already sits at the line budget
//! this repository sets for codegen/CLI sources, and the invariant below is a self-contained
//! concern -- it adds no state and only narrows which of `write.rs`'s two stamping entry points
//! a caller is allowed to reach for. ~keep

use super::finalize_hashes_sweeping;
use std::path::Path;

/// The [`super::finalize_hashes`] variant a caller **must** use when the formatter pass it just ran
/// was whole-tree, i.e. `format_generated(.., None)` -- which delegates to
/// `converge_full_regen_formatting`, which runs `poly fmt --fix <tree_root>` and
/// `cargo fmt --all` over the entire repository, not over this run's output.
///
/// The invariant: **the stamp scope must cover the format scope.** `finalize_hashes` stamps
/// exactly the paths it is handed, and it is documented to run *after* every formatter so a
/// formatter pass is never mistaken for drift. That guarantee holds only while the formatter
/// stayed inside the handed path set. A caller that formatted the whole tree and then stamped
/// a narrow set (the stub files it generated, the manifests it regenerated) leaves every other
/// alef-marked file in the repository holding an `alef:hash:` line derived from its
/// *pre-format* bytes, while its on-disk bytes are now post-format. Nothing later repairs
/// that: `alef verify` rehashes the on-disk bytes and reports the file stale on every run, and
/// re-running generation does not help when the owning language's cache reports it unchanged.
/// Two files whose bodies differ only in line width but carry the *same* embedded hash is the
/// fingerprint of exactly this -- a content-inclusive hash cannot otherwise collide.
///
/// This is the same laundering trade-off [`finalize_hashes_sweeping`] documents at length,
/// widened from that function's language output roots to the root the formatter was actually
/// pointed at, and it is accepted here for the same reason: the formatter has *already*
/// rewritten those bytes by the time this runs, so declining to re-stamp does not preserve a
/// hand-edit signal, it only converts alef's own formatting into permanent, unfixable drift. ~keep
pub fn finalize_hashes_after_tree_format(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    tree_root: &Path,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
) -> anyhow::Result<usize> {
    finalize_hashes_sweeping(paths, &[tree_root.to_path_buf()], sources_hash, alef_toml_bytes)
}
