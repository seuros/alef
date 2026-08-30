use super::*;

/// The exact shape `scaffold_wasm` emitted before the fix that added the `exports` map --
/// a single `nodejs` target, `main`/`module`/`types` all pointing at the same crate file.
fn pre_fix_package_json() -> String {
    "{\n  \
     \"name\": \"@scope/example-wasm\",\n  \
     \"version\": \"1.0.0\",\n  \
     \"private\": false,\n  \
     \"description\": \"An example crate\",\n  \
     \"publishConfig\": {\n    \"access\": \"public\"\n  },\n  \
     \"type\": \"module\",\n  \
     \"files\": [\n    \"pkg/nodejs\",\n    \"README.md\"\n  ],\n  \
     \"main\": \"pkg/nodejs/example_wasm.js\",\n  \
     \"module\": \"pkg/nodejs/example_wasm.js\",\n  \
     \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n  \
     \"engines\": {\n    \"node\": \">=18\"\n  },\n  \
     \"scripts\": {\n    \"build\": \"wasm-pack build\"\n  }\n\
     }\n"
    .to_string()
}

#[test]
fn should_insert_exports_map_when_missing_from_alef_authored_package_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    std::fs::write(pkg_dir.join("package.json"), pre_fix_package_json()).expect("write pre-fix package.json");

    let relative_path = Path::new("crates/example-wasm/package.json");
    let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
    assert!(changed, "a package.json missing exports must be reported as changed");

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
    assert_eq!(
        parsed["exports"]["."]["types"], "./pkg/nodejs/example_wasm.d.ts",
        "exports map must reference the same target/crate_file as the existing main/module/types fields"
    );
    assert_eq!(parsed["exports"]["."]["require"], "./pkg/nodejs/example_wasm.js");
    assert_eq!(
        parsed["name"], "@scope/example-wasm",
        "fields outside exports must survive untouched"
    );
    assert_eq!(
        parsed["scripts"]["build"], "wasm-pack build",
        "user-visible fields must survive untouched"
    );

    let changed_again =
        migrate_wasm_package_json_exports(dir.path(), relative_path).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

#[test]
fn should_not_touch_a_package_json_that_already_has_exports() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let hand_written = "{\n  \"name\": \"@scope/example-wasm\",\n  \"exports\": \"./custom.js\"\n}\n";
    std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write hand-edited package.json");

    let relative_path = Path::new("crates/example-wasm/package.json");
    let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
    assert!(
        !changed,
        "a package.json that already declares exports must never be touched"
    );

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(
        on_disk, hand_written,
        "a custom exports field must survive byte-for-byte"
    );
}

#[test]
fn should_not_touch_a_foreign_package_json_without_the_alef_wasm_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let hand_written = concat!(
        "{\n",
        "  \"name\": \"example\",\n",
        "  \"main\": \"index.js\",\n",
        "  \"engines\": {\n",
        "    \"node\": \">=18\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write foreign package.json");

    let relative_path = Path::new("crates/example-wasm/package.json");
    let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("migration must not error");
    assert!(
        !changed,
        "a package.json without alef's main/module/types shape must never be touched"
    );

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(
        on_disk, hand_written,
        "a foreign package.json must survive byte-for-byte"
    );
}

#[test]
fn migrate_wasm_package_json_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative_path = Path::new("crates/example-wasm/package.json");
    let changed = migrate_wasm_package_json_exports(dir.path(), relative_path).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join(relative_path).exists());
}

/// The shape `scaffold_wasm` emitted before the fix that declared `vitest` (and its coverage
/// provider) as devDependencies: the alef main/module/types fingerprint and every test script
/// are present, but `devDependencies` does not exist at all -- the true pre-fix state of every
/// consumer crate scaffolded before that fix shipped. `scripts` is deliberately the last
/// top-level key, matching the exact `  }\n}` tail [`insert_new_dev_dependencies_block`]
/// anchors on.
fn pre_vitest_fix_package_json() -> String {
    concat!(
        "{\n",
        "  \"name\": \"@scope/example-wasm\",\n",
        "  \"version\": \"1.0.0\",\n",
        "  \"private\": false,\n",
        "  \"main\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"module\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n",
        "  \"engines\": {\n",
        "    \"node\": \">= 22\"\n",
        "  },\n",
        "  \"scripts\": {\n",
        "    \"build\": \"wasm-pack build --target nodejs --out-dir pkg/nodejs\",\n",
        "    \"test\": \"vitest run\",\n",
        "    \"test:watch\": \"vitest watch\",\n",
        "    \"test:coverage\": \"vitest run --coverage\",\n",
        "    \"clean\": \"rm -rf pkg dist\"\n",
        "  }\n",
        "}\n",
    )
    .to_string()
}

/// EXISTING Alef-authored population, no `devDependencies` key at all: migration must insert
/// a whole new block declaring both `vitest` and its coverage provider, and must not disturb
/// any other field. Also proves idempotency at the file level: a second run makes no further
/// change, and the bytes on disk after the second run are identical to the bytes after the
/// first.
#[test]
fn should_insert_full_dev_dependencies_block_for_pre_vitest_fix_package_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    std::fs::write(pkg_dir.join("package.json"), pre_vitest_fix_package_json())
        .expect("write pre-fix package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(changed, "a package.json missing devDependencies must be reported as changed");

    let after_first = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&after_first).expect("migrated file must be valid JSON");
    assert_eq!(
        parsed["devDependencies"]["vitest"],
        tv::npm::VITEST,
        "must pin vitest to the central registry version, got:\n{parsed:#}"
    );
    assert_eq!(
        parsed["devDependencies"]["@vitest/coverage-v8"],
        tv::npm::VITEST_COVERAGE_V8,
        "must pin the coverage provider to the central registry version, got:\n{parsed:#}"
    );
    assert_eq!(
        parsed["name"], "@scope/example-wasm",
        "fields outside devDependencies must survive untouched"
    );
    assert_eq!(
        parsed["scripts"]["clean"], "rm -rf pkg dist",
        "scripts must survive untouched"
    );

    let changed_again = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("second pass must not error");
    assert!(!changed_again, "second pass over an already-migrated file must be a no-op");
    let after_second = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file after second pass");
    assert_eq!(
        after_first, after_second,
        "a second migration pass must produce byte-identical output to the first"
    );
}

/// EXISTING Alef-authored population, `devDependencies` already present with `vitest` pinned
/// to a value a consumer chose themselves (not the central registry's current value):
/// migration must add only the missing coverage provider and must never overwrite the
/// consumer's own `vitest` pin.
#[test]
fn should_add_only_the_missing_coverage_provider_and_never_overwrite_an_existing_vitest_pin() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let fixture = concat!(
        "{\n",
        "  \"name\": \"@scope/example-wasm\",\n",
        "  \"main\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"module\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n",
        "  \"engines\": {\n",
        "    \"node\": \">= 22\"\n",
        "  },\n",
        "  \"scripts\": {\n",
        "    \"test\": \"vitest run\",\n",
        "    \"test:watch\": \"vitest watch\",\n",
        "    \"test:coverage\": \"vitest run --coverage\"\n",
        "  },\n",
        "  \"devDependencies\": {\n",
        "    \"vitest\": \"1.2.3\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), fixture).expect("write fixture package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(changed, "a package.json missing only the coverage provider must be reported as changed");

    let after_first = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&after_first).expect("migrated file must be valid JSON");
    assert_eq!(
        parsed["devDependencies"]["vitest"], "1.2.3",
        "an existing consumer-chosen vitest pin must never be overwritten, got:\n{parsed:#}"
    );
    assert_eq!(
        parsed["devDependencies"]["@vitest/coverage-v8"],
        tv::npm::VITEST_COVERAGE_V8,
        "must add the coverage provider pinned to the central registry version, got:\n{parsed:#}"
    );

    let changed_again = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("second pass must not error");
    assert!(!changed_again, "second pass over an already-migrated file must be a no-op");
    let after_second = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file after second pass");
    assert_eq!(
        after_first, after_second,
        "a second migration pass must produce byte-identical output to the first"
    );
}

/// EXISTING Alef-authored population with a consumer's own unrelated dev dependency already
/// present: migration must add the missing `vitest`/coverage entries without disturbing the
/// consumer's own entry in any way.
#[test]
fn should_preserve_a_consumer_added_dev_dependency_while_inserting_the_missing_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let fixture = concat!(
        "{\n",
        "  \"name\": \"@scope/example-wasm\",\n",
        "  \"main\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"module\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n",
        "  \"engines\": {\n",
        "    \"node\": \">= 22\"\n",
        "  },\n",
        "  \"scripts\": {\n",
        "    \"test\": \"vitest run\",\n",
        "    \"test:watch\": \"vitest watch\",\n",
        "    \"test:coverage\": \"vitest run --coverage\"\n",
        "  },\n",
        "  \"devDependencies\": {\n",
        "    \"typescript\": \"^7.0.0\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), fixture).expect("write fixture package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(changed);

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
    assert_eq!(
        parsed["devDependencies"]["typescript"], "^7.0.0",
        "a consumer-added, unrelated dev dependency must survive untouched, got:\n{parsed:#}"
    );
    assert_eq!(parsed["devDependencies"]["vitest"], tv::npm::VITEST);
    assert_eq!(parsed["devDependencies"]["@vitest/coverage-v8"], tv::npm::VITEST_COVERAGE_V8);
}

/// FOREIGN/unrecognized population, case 1: no alef main/module/types fingerprint at all,
/// even though the file happens to run vitest -- must never be touched. This is the safety
/// case: a false positive here would silently rewrite a user's hand-maintained package.json.
#[test]
fn should_not_touch_a_foreign_vitest_package_json_without_the_alef_wasm_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let hand_written = concat!(
        "{\n",
        "  \"name\": \"example\",\n",
        "  \"main\": \"index.js\",\n",
        "  \"scripts\": {\n",
        "    \"test\": \"vitest run\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write foreign package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(
        !changed,
        "a package.json without alef's main/module/types shape must never be touched"
    );
    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(on_disk, hand_written, "a foreign package.json must survive byte-for-byte");
}

/// FOREIGN/unrecognized population, case 2: the alef main/module/types fingerprint is
/// present, but the exact `"test": "vitest run"` script this migration anchors on is not --
/// proving the second, independent anchor also guards against a false positive on a file that
/// merely looks alef-shaped at the build-output level. Must never be touched.
#[test]
fn should_not_touch_an_alef_shaped_package_json_missing_the_vitest_test_script() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let hand_written = concat!(
        "{\n",
        "  \"name\": \"@scope/example-wasm\",\n",
        "  \"main\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"module\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n",
        "  \"scripts\": {\n",
        "    \"build\": \"wasm-pack build\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write foreign package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(
        !changed,
        "a package.json without the exact vitest test script anchor must never be touched"
    );
    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(on_disk, hand_written, "a foreign package.json must survive byte-for-byte");
}

/// FOREIGN/unrecognized population, case 3: both `vitest` and the coverage provider are
/// already declared (whatever their values), so there is nothing to add. Also the terminal
/// idempotency state every other test's second pass converges on.
#[test]
fn should_not_touch_a_package_json_that_already_declares_both_dependencies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    let hand_written = concat!(
        "{\n",
        "  \"name\": \"@scope/example-wasm\",\n",
        "  \"main\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"module\": \"pkg/nodejs/example_wasm.js\",\n",
        "  \"types\": \"pkg/nodejs/example_wasm.d.ts\",\n",
        "  \"scripts\": {\n",
        "    \"test\": \"vitest run\",\n",
        "    \"test:coverage\": \"vitest run --coverage\"\n",
        "  },\n",
        "  \"devDependencies\": {\n",
        "    \"vitest\": \"9.9.9\",\n",
        "    \"@vitest/coverage-v8\": \"9.9.9\"\n",
        "  }\n",
        "}\n",
    );
    std::fs::write(pkg_dir.join("package.json"), hand_written).expect("write already-complete package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path)
        .expect("migration must not error");
    assert!(!changed, "a package.json declaring both dependencies must never be touched");
    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(
        on_disk, hand_written,
        "an already-complete package.json must survive byte-for-byte"
    );
}

/// Idempotency of the pure transform itself, independent of the file-system round trip above:
/// applying it once to the pre-fix fixture produces a migration; applying it again to that
/// migrated string finds nothing left to add.
#[test]
fn repair_wasm_vitest_dev_dependencies_is_idempotent_at_the_string_level() {
    let fixture = pre_vitest_fix_package_json();
    let once = repair_missing_wasm_vitest_dev_dependencies(&fixture).expect("first pass must migrate");
    let twice = repair_missing_wasm_vitest_dev_dependencies(&once);
    assert!(
        twice.is_none(),
        "a second pass over already-migrated content must find nothing left to add, got:\n{twice:?}"
    );
}

#[test]
fn migrate_wasm_package_json_vitest_dev_dependencies_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative_path = Path::new("crates/example-wasm/package.json");
    let changed =
        migrate_wasm_package_json_vitest_dev_dependencies(dir.path(), relative_path).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join(relative_path).exists());
}

/// `migrate_wasm_package_json` is the one call site `cli::pipeline::generate::scaffold` invokes;
/// a file missing both the `exports` map and the vitest devDependencies (the true state of a
/// crate scaffolded before either fix shipped) must come out of one call with both repairs
/// applied, and a second call must be a byte-identical no-op.
#[test]
fn migrate_wasm_package_json_applies_both_repairs_through_the_one_entry_point() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-wasm");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-wasm");
    std::fs::write(pkg_dir.join("package.json"), pre_vitest_fix_package_json())
        .expect("write pre-fix package.json");
    let relative_path = Path::new("crates/example-wasm/package.json");

    let changed = migrate_wasm_package_json(dir.path(), relative_path).expect("migration must not error");
    assert!(changed, "a file missing both fixes must be reported as changed");

    let after_first = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&after_first).expect("migrated file must be valid JSON");
    assert_eq!(
        parsed["exports"]["."]["types"], "./pkg/nodejs/example_wasm.d.ts",
        "the exports repair must have run, got:\n{parsed:#}"
    );
    assert_eq!(
        parsed["devDependencies"]["vitest"],
        tv::npm::VITEST,
        "the vitest devDependency repair must have run, got:\n{parsed:#}"
    );
    assert_eq!(
        parsed["devDependencies"]["@vitest/coverage-v8"],
        tv::npm::VITEST_COVERAGE_V8,
        "the coverage provider repair must have run, got:\n{parsed:#}"
    );

    let changed_again = migrate_wasm_package_json(dir.path(), relative_path).expect("second pass must not error");
    assert!(!changed_again, "second pass over an already-migrated file must be a no-op");
    let after_second = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file after second pass");
    assert_eq!(
        after_first, after_second,
        "a second migration pass must produce byte-identical output to the first"
    );
}
