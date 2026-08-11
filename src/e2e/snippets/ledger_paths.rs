use std::path::{Component, Path, PathBuf};

use crate::snippets::error::{Error, Result};

pub(crate) fn resolve_tracked_path(output_root: &Path, relative: &Path) -> Result<PathBuf> {
    let escapes_root = relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if relative.as_os_str().is_empty() || relative.is_absolute() || escapes_root {
        return Err(invalid_path(relative));
    }
    let root = output_root.canonicalize().map_err(Error::Io)?;
    let candidate = output_root.join(relative);
    let resolved = match candidate.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
        Err(error) => return Err(Error::Io(error)),
    };
    if !resolved.starts_with(&root) {
        return Err(invalid_path(relative));
    }
    Ok(candidate)
}

fn invalid_path(path: &Path) -> Error {
    Error::Other(format!(
        "fixture snippet ledger path must stay beneath its output root: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_regular_tracked_file_beneath_root() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("python/example.md");
        std::fs::create_dir_all(path.parent().expect("parent directory")).expect("create directory");
        std::fs::write(&path, "example").expect("write tracked file");

        assert_eq!(
            resolve_tracked_path(directory.path(), Path::new("python/example.md")).expect("safe path"),
            path
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_tracked_file_outside_root() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        let link = directory.path().join("escaped.md");
        std::os::unix::fs::symlink(outside.path(), &link).expect("create symlink");

        let error = resolve_tracked_path(directory.path(), Path::new("escaped.md")).expect_err("escaped path");
        assert!(error.to_string().contains("must stay beneath"), "{error}");
    }
}
