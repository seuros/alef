//! Small, self-contained `test_file.rs` unit tests split into a sibling file (matching the
//! `import_lines.rs`/`lint_clean_python_tests.rs` split) to keep `test_file.rs` itself under
//! material headroom below its baselined file-size ceiling.

use super::*;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::FixtureGroup;

fn test_filenames(groups: &[FixtureGroup]) -> Vec<String> {
    groups
        .iter()
        .map(|g| format!("test_{}.py", sanitize_filename(&g.category)))
        .collect()
}

#[test]
fn test_filenames_produces_snake_case_names() {
    let groups = vec![
        FixtureGroup {
            category: "MyCategory".to_string(),
            fixtures: Vec::new(),
        },
        FixtureGroup {
            category: "another-thing".to_string(),
            fixtures: Vec::new(),
        },
    ];
    let names = test_filenames(&groups);
    assert_eq!(names[0], "test_mycategory.py");
    assert_eq!(names[1], "test_another_thing.py");
}

#[test]
fn per_call_native_types_are_excluded_from_public_imports() {
    let import_names = vec!["create_client".to_string(), "WidgetRequest".to_string()];
    let native_imports = [("my_lib._internal_bindings".to_string(), "WidgetRequest".to_string())]
        .into_iter()
        .collect();

    assert_eq!(
        public_import_names(&import_names, &native_imports),
        vec!["create_client"]
    );
}
