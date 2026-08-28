//! Removal of self-imports from generated Swift sources.
//!
//! Several emitters in this backend write into more than one SwiftPM target: the trait-bridge
//! protocol/adapter files and the plugin `Box` shims land in `Sources/RustBridge/`, while the
//! facade, service wrappers and registration overloads land in `Sources/<Module>/`. Both groups
//! declare `import RustBridge`, which is correct for the second group and a self-import for the
//! first -- Swift answers it with `file ... is part of module 'RustBridge'; ignoring import`,
//! which fails any warnings-as-errors build.
//!
//! The decision therefore cannot live in the emitter (it does not know its destination) or in the
//! template (it is shared by both groups). It is made here, from the destination path, so a file
//! moved between targets keeps the imports it needs and loses only the one it cannot use.

use crate::core::backend::GeneratedFile;
use std::path::Path;

/// SwiftPM derives a target's module name from its directory under `Sources/`, so
/// `Sources/<Module>/x.swift` is the only shape that identifies a file's own module. ~keep
const SWIFT_TARGET_PARENT_DIR: &str = "Sources";

const SWIFT_EXTENSION: &str = "swift";

/// The SwiftPM module a generated file belongs to, or `None` when its path is not
/// `.../Sources/<Module>/<file>` and the owning module is therefore unknown.
fn owning_module(path: &Path) -> Option<&str> {
    if path.extension()?.to_str()? != SWIFT_EXTENSION {
        return None;
    }
    let target_dir = path.parent()?;
    let module = target_dir.file_name()?.to_str()?;
    let sources_dir = target_dir.parent()?.file_name()?.to_str()?;
    (sources_dir == SWIFT_TARGET_PARENT_DIR).then_some(module)
}

/// Drop every `import <Module>` line from a source that is itself part of `<Module>`.
///
/// Only a line whose entire content is the import is dropped, so prose mentioning the import in a
/// comment or doc block survives.
fn without_self_import(content: &str, module: &str) -> Option<String> {
    let self_import = format!("import {module}");
    if !content.lines().any(|line| line.trim() == self_import) {
        return None;
    }
    let kept: Vec<&str> = content.lines().filter(|line| line.trim() != self_import).collect();
    let mut stripped = kept.join("\n");
    if content.ends_with('\n') {
        stripped.push('\n');
    }
    Some(stripped)
}

/// Strip self-imports from every Swift file in `files`, leaving imports of other modules alone.
pub(super) fn strip_self_module_imports(mut files: Vec<GeneratedFile>) -> Vec<GeneratedFile> {
    for file in &mut files {
        let Some(module) = owning_module(&file.path).map(str::to_owned) else {
            continue;
        };
        if let Some(stripped) = without_self_import(&file.content, &module) {
            file.content = stripped;
        }
    }
    files
}

#[cfg(test)]
mod tests;
