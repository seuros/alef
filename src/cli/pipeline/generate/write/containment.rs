//! The disk-aware half of the generated-output write boundary.
//!
//! Split out of `write.rs` rather than added to it: that file sits close to this repository's
//! 1,000-line cap, and "does this path still land inside the project once the filesystem has had
//! its say" is a self-contained concern from "how bytes reach disk". ~keep

use std::path::{Component, Path};

/// Refuse an emitted path whose already-existing ancestor chain leaves `base_dir` through a
/// symlink.
///
/// [`super::contained_output_path`]'s lexical check is the first half of this boundary and stays
/// exactly as it is: it runs on the string, before anything exists on disk, and rejects
/// absoluteness, `..` and drive prefixes. What it cannot see is the disk. `packages/node/index.ts`
/// passes every component test there is and still lands outside the project the moment
/// `packages/node` is a symlink to somewhere else, because both writers that follow resolve
/// symlinks: `std::fs::create_dir_all` walks through a symlinked directory, and
/// `tempfile::NamedTempFile::new_in(parent)` creates its temporary inside whatever `parent`
/// really is. A repository can ship that symlink in its own tracked tree, so the escape needs no
/// hostile config value at all -- which is why the lexical pass alone is not the boundary.
///
/// Resolution stops at the deepest ancestor that **exists**. The leaf almost never does -- it is
/// what this run is about to create -- and `canonicalize` on a missing path returns an error
/// rather than an answer, so canonicalizing the full emitted path would fail on the ordinary
/// case. The remaining components are left to the lexical pass that already cleared them: a
/// component that does not exist cannot be a symlink, and one created later is created by us,
/// underneath a parent this walk has already resolved and contained.
///
/// Every comparison is canonical-to-canonical, because `base_dir` itself being reached through a
/// symlink is the common case, not the exotic one: on macOS `/tmp` is a symlink to `/private/tmp`
/// (so is every `tempfile::tempdir()` handed out under `/var/folders`, `/var` -> `private/var`),
/// and a checkout under a symlinked home or mounted volume behaves the same. Comparing a resolved
/// descendant against the *uncanonicalized* base would reject all of those legitimate writes,
/// which is the likeliest way a containment check breaks real usage. Canonicalizing the base once
/// and joining each component onto the previously-resolved parent keeps both sides in the same
/// namespace. ~keep
pub(super) fn ensure_no_symlink_escape(base_dir: &Path, emitted_path: &Path) -> Result<(), String> {
    let Ok(canonical_base) = base_dir.canonicalize() else {
        // A base that does not exist yet has nothing beneath it to follow; `create_dir_all`
        // materialises the whole chain itself, so every component is one we create. ~keep
        return Ok(());
    };
    let mut resolved = canonical_base.clone();
    for component in emitted_path.components() {
        let segment = match component {
            Component::Normal(segment) => segment,
            Component::CurDir => continue,
            _ => {
                return Err(format!(
                    "resolved output path `{}` is not a plain relative path",
                    emitted_path.display()
                ));
            }
        };
        let candidate = resolved.join(segment);
        // `symlink_metadata` rather than `exists`: a dangling symlink is a component that
        // exists for the purposes of this walk (the writers would follow it) while `exists`
        // reports it missing, which would end the walk one component early. ~keep
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "failed to inspect existing output path `{}`: {error}",
                    candidate.display()
                ));
            }
        }
        let real = candidate.canonicalize().map_err(|error| {
            format!(
                "failed to resolve existing output path `{}`: {error}",
                candidate.display()
            )
        })?;
        if !real.starts_with(&canonical_base) {
            return Err(format!(
                "existing path `{}` resolves to `{}`, which is outside `{}`",
                candidate.display(),
                real.display(),
                canonical_base.display()
            ));
        }
        resolved = real;
    }
    Ok(())
}
