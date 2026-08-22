//! The base64-encoded-binary output rail.
//!
//! A handful of alef's generated outputs are not text. `GeneratedFile::content` is a
//! `String`, so those emitters carry the artifact as base64 (see
//! `backends::kotlin_android::gradle_wrapper::get_gradle_wrapper_jar_base64`) and every
//! consumer of that content has to decode it before it means anything.
//!
//! Three call sites did that decode with their own inline `extension == "jar"` test —
//! [`super::write::write_files_report`], [`super::scaffold::write_scaffold_files_report`]
//! and, by omission, [`super::diff::diff_files`], which had no test at all and so compared
//! the *base64 text* against `read_to_string(...).unwrap_or_default()` — an empty string,
//! because a jar is not UTF-8. The comparison could therefore never be equal, and
//! `alef diff` reported every binary output as drifted in every repo on every run,
//! whatever its actual bytes. The predicate is named and shared here so a fourth consumer
//! cannot silently be a fourth answer. ~keep

use anyhow::{Context, Result};
use base64::Engine;
use std::path::Path;

/// File extensions whose `GeneratedFile::content` is base64 rather than the literal bytes.
///
/// Deliberately an extension allowlist rather than a content sniff: the question being
/// asked is "how did alef *encode* this output", which is a property of the emitter, and
/// a sniff would answer "do these bytes look like base64" — true of plenty of real text
/// output. ~keep
const BASE64_BINARY_EXTENSIONS: &[&str] = &["jar"];

/// Whether alef emits `path`'s content base64-encoded and writes it as decoded bytes.
pub fn is_base64_binary_output(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| BASE64_BINARY_EXTENSIONS.contains(&extension))
}

/// Decode a [`is_base64_binary_output`] path's generated content into the bytes a write
/// would place on disk.
pub fn decode_base64_binary(path: &Path, content: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(content)
        .with_context(|| format!("failed to decode base64 for {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_output_is_recognised_and_other_extensions_are_not() {
        assert!(is_base64_binary_output(Path::new(
            "packages/kotlin-android/gradle/wrapper/gradle-wrapper.jar"
        )));
        assert!(!is_base64_binary_output(Path::new("packages/node/package.json")));
        assert!(!is_base64_binary_output(Path::new("gradlew")));
    }

    #[test]
    fn decoding_names_the_path_when_the_content_is_not_base64() {
        let error = decode_base64_binary(Path::new("packages/demo/wrapper.jar"), "not base64!")
            .expect_err("malformed base64 must not decode");
        assert!(
            format!("{error:#}").contains("packages/demo/wrapper.jar"),
            "the decode failure must name the offending path, got: {error:#}"
        );
    }
}
