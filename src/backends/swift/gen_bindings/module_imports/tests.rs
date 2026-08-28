use super::strip_self_module_imports;
use crate::core::backend::GeneratedFile;
use std::path::PathBuf;

fn swift_file(path: &str, content: &str) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        content: content.to_string(),
        generated_header: false,
    }
}

fn only_content(files: Vec<GeneratedFile>) -> String {
    assert_eq!(files.len(), 1, "fixture builds exactly one file");
    files.into_iter().next().expect("one file").content
}

#[test]
fn should_drop_import_of_the_module_the_file_belongs_to() {
    let files = vec![swift_file(
        "packages/swift/Sources/RustBridge/SwiftSampleTraitBridge.swift",
        "import Foundation\nimport RustBridge\n\npublic protocol SwiftSampleTraitBridge {}\n",
    )];

    assert_eq!(
        only_content(strip_self_module_imports(files)),
        "import Foundation\n\npublic protocol SwiftSampleTraitBridge {}\n"
    );
}

#[test]
fn should_keep_import_of_a_different_module() {
    let source = "import Foundation\nimport RustBridge\n\npublic extension SampleValue {}\n";
    let files = vec![swift_file(
        "packages/swift/Sources/SampleModule/SampleModule.swift",
        source,
    )];

    assert_eq!(
        only_content(strip_self_module_imports(files)),
        source,
        "a file in Sources/SampleModule must keep `import RustBridge` -- RustBridge is a different \
         SwiftPM target and the file cannot name its symbols without it"
    );
}

#[test]
fn should_keep_prose_mentioning_the_self_import() {
    let source = "import Foundation\n// Re-exported so callers need only `import RustBridge`.\n";
    let files = vec![swift_file("packages/swift/Sources/RustBridge/Notes.swift", source)];

    assert_eq!(only_content(strip_self_module_imports(files)), source);
}

#[test]
fn should_ignore_files_outside_a_sources_target_directory() {
    let source = "import RustBridge\n";
    let files = vec![swift_file("packages/swift/RustBridge/Loose.swift", source)];

    assert_eq!(only_content(strip_self_module_imports(files)), source);
}

#[test]
fn should_ignore_non_swift_files() {
    let source = "import RustBridge\n";
    let files = vec![swift_file("packages/swift/Sources/RustBridge/notes.md", source)];

    assert_eq!(only_content(strip_self_module_imports(files)), source);
}
