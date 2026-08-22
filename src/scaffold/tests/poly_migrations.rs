//! Regression coverage for [`migrate_poly_toml_drop_snippet_hook`]: `poly.toml`'s managed merge
//! unions and prunes array values but never retracts a whole table alef stops emitting, so an
//! already-scaffolded consumer keeps re-merging the retracted `alef-snippets` pre-commit hook
//! forever. See the migration's own doc for the full defect.

use super::*;

const STALE_POLY_TOML: &str = "[discovery]\nexclude = []\n\n[hooks.pre-commit.commands.alef-snippets]\n\
     run = \"alef snippets check --strict --cache off\"\n\
     root = \".\"\n\
     workspace = true\n\
     files = \"{alef.toml,fixtures/**/*.json,docs/snippets/**}\"\n";

#[test]
fn should_remove_the_retracted_alef_snippets_pre_commit_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("poly.toml"), STALE_POLY_TOML).expect("write stale poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(
        changed,
        "the known-stale alef-snippets hook must be reported as changed"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read migrated file");
    assert!(
        !on_disk.contains("alef-snippets"),
        "the retracted hook table must be gone: {on_disk}"
    );
    assert!(
        !on_disk.contains("alef snippets check"),
        "the retracted hook command must be gone: {on_disk}"
    );
    // The rest of the file -- untouched tables -- must survive.
    assert!(on_disk.contains("[discovery]"));
    toml::from_str::<toml::Value>(&on_disk).expect("migrated poly.toml must still parse");

    let changed_again = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

#[test]
fn should_not_touch_a_consumer_added_pre_commit_command_of_a_different_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.rubocop]\n\
         run = \"bundle exec rubocop\"\n\
         root = \"packages/ruby\"\n\
         workspace = true\n\
         files = \"packages/ruby/**/*.rb\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(!changed, "no alef-snippets table present -- must be a no-op");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(
        on_disk, poly_toml,
        "an unrelated pre-commit command must survive byte-for-byte"
    );
}

#[test]
fn should_not_touch_a_same_named_hook_the_consumer_repurposed_with_a_different_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.alef-snippets]\n\
         run = \"echo custom hook the consumer wrote themselves\"\n\
         root = \".\"\n\
         workspace = true\n\
         files = \"docs/**\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(
        !changed,
        "a same-named table running a different command was never alef's own -- must be left alone"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(
        on_disk, poly_toml,
        "a consumer-repurposed alef-snippets table must survive byte-for-byte"
    );
}

#[test]
fn migrate_poly_toml_drop_snippet_hook_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join("poly.toml").exists());
}
