//! Regression coverage for one decision every e2e backend that emits a build-tool-level (or
//! process-level) working-directory setting must make the same way: guard it on the
//! test_documents directory's existence.
//!
//! Unguarded, a forked test worker (Gradle) or Surefire's forked JVM (Maven) fails to fork
//! at all when the directory does not exist -- Gradle reports a misleading "Gradle Test
//! Executor N ... not in started or detached state" with the real fork `IOException` masked
//! and no assertion text at all. A fresh checkout of a consumer repo whose `test_documents/`
//! fixture directory has zero tracked files hits this on every run.
//!
//! Five backends implement this decision independently: Kotlin and Kotlin Android both emit
//! Gradle Kotlin DSL `workingDir =`; Java emits a Maven Surefire `<workingDirectory>`; C#
//! emits a runtime `Directory.SetCurrentDirectory` chdir inside the generated test binary's
//! module initializer instead of a build-file setting; Zig's `build.zig` calls
//! `RunStep.setCwd` at build-configure time. Gradle DSL, Maven XML, "chdir at runtime in the
//! emitted binary", and "RunStep.setCwd in build.zig" are different enough mechanisms that a
//! single shared Rust/template helper spanning all of them does not fit -- see CLAUDE.md's
//! `avoid-duplication` rule (shared code must have one reason to change; these do not share
//! one). Kotlin and Kotlin Android *do* share a mechanism (both are Gradle Kotlin DSL) and
//! share the `gradle/guarded_working_dir.kt.jinja` template for it.
//!
//! What still needs a guardrail is the *decision*, independent of syntax: this test pins it
//! for every backend that emits one, through each backend's own public
//! [`super::E2eCodegen::generate`] entry point (rather than each backend's own
//! privately-scoped renderer, which a peer module cannot reach across a private `mod`
//! boundary) -- so a regression in an existing backend, or a new backend that reintroduces an
//! unguarded working-directory setting, is caught here even before it is added to this file's
//! own suite.

use super::E2eCodegen;
use super::csharp::CSharpCodegen;
use super::java::JavaCodegen;
use super::kotlin::KotlinE2eCodegen;
use super::kotlin_android::KotlinAndroidE2eCodegen;
use super::zig::ZigE2eCodegen;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

fn generated_file_ending_in<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a GeneratedFile {
    files.iter().find(|f| f.path.ends_with(suffix)).unwrap_or_else(|| {
        panic!(
            "expected a generated file ending in `{suffix}`, got: {:?}",
            files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
        )
    })
}

#[test]
fn kotlin_build_gradle_guards_working_dir_on_existence() {
    let files = KotlinE2eCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("kotlin e2e generation succeeds on an empty fixture set");
    let build_gradle = generated_file_ending_in(&files, "build.gradle.kts");
    assert!(
        build_gradle.content.contains(".isDirectory"),
        "plain-Kotlin build.gradle.kts must guard workingDir on directory existence, got:\n{}",
        build_gradle.content
    );
}

#[test]
fn kotlin_android_build_gradle_guards_working_dir_on_existence() {
    let files = KotlinAndroidE2eCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("kotlin_android e2e generation succeeds on an empty fixture set");
    let build_gradle = generated_file_ending_in(&files, "build.gradle.kts");
    assert!(
        build_gradle.content.contains(".isDirectory"),
        "kotlin_android build.gradle.kts must guard workingDir on directory existence, got:\n{}",
        build_gradle.content
    );
}

#[test]
fn java_pom_xml_guards_working_directory_on_existence() {
    let files = JavaCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("java e2e generation succeeds on an empty fixture set");
    let pom_xml = generated_file_ending_in(&files, "pom.xml");
    let (unconditional, guarded) = pom_xml.content.split_once("<profiles>").unwrap_or(("", ""));
    assert!(
        !unconditional.contains("<workingDirectory>"),
        "the unconditionally-active surefire plugin config must not set workingDirectory \
         unguarded, got:\n{}",
        pom_xml.content
    );
    assert!(
        guarded.contains("<activation>") && guarded.contains("<exists>"),
        "workingDirectory must be set only inside a file-existence-activated profile, got:\n{}",
        pom_xml.content
    );
}

#[test]
fn zig_build_zig_guards_set_cwd_on_working_directory_existence() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "detect_mime_type_from_bytes".to_string(),
            args: vec![ArgMapping {
                name: "content".to_string(),
                field: "input.data".to_string(),
                arg_type: "file_path".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let fixture = Fixture {
        id: "mime_detect_from_path".to_string(),
        description: "Detect MIME type from a file path".to_string(),
        input: serde_json::json!({"data": "pdf/fake_memo.pdf"}),
        ..Fixture::default()
    };
    let groups = [FixtureGroup {
        category: "mime".to_string(),
        fixtures: vec![fixture],
    }];

    let files = ZigE2eCodegen
        .generate(
            &groups,
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("zig e2e generation succeeds for a file-fixture-bearing fixture set");
    let build_zig = generated_file_ending_in(&files, "build.zig");
    assert!(
        build_zig.content.contains("openDir("),
        "build.zig must guard RunStep.setCwd on the test_documents directory's existence, got:\n{}",
        build_zig.content
    );
}

#[test]
fn csharp_test_setup_guards_working_directory_change_on_existence() {
    let files = CSharpCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("csharp e2e generation succeeds on an empty fixture set");
    let test_setup = generated_file_ending_in(&files, "TestSetup.cs");
    assert!(
        test_setup.content.contains("Directory.Exists"),
        "TestSetup.cs must guard Directory.SetCurrentDirectory on Directory.Exists, got:\n{}",
        test_setup.content
    );
}
