//! `package_dir` for `java` and `kotlin` (JVM, non-Android) used to ignore
//! `[crates.output]` entirely: it always answered the hard-coded literal `packages/java` /
//! `packages/kotlin`, while `JavaBackend`/`KotlinBackend`'s own `generate_bindings` honored a
//! configured `[crates.output].<lang>` and wrote sources wherever that pointed.
//!
//! `build_command_config_for_language` interpolates `package_dir` into `mvn -f
//! {output_dir}/pom.xml …` (Java) and `cd {output_dir} && gradle …` (Kotlin), so a consumer who
//! moves the tree with `[crates.output]` -- as `tslp` does for Java, spelling out the full Maven
//! `src/main/java/` source directory -- got a build command that targeted a directory
//! `alef generate` never wrote a single file into. Two consumer repos (`xberg`, `tslp`) stayed
//! accidentally unaffected only because both configured trees still happen to sit under
//! `packages/java`; a consumer configuring an unrelated tree such as `sdk/java/…` hit the split
//! immediately.
//!
//! This test runs the *real* backends and the *real* build-command resolution and asserts the
//! build command's target directory is an ancestor of (or equal to) the directory the generator
//! actually wrote sources into. Unlike `kotlin_android`, `java`/`kotlin` generation never emits
//! the project manifest (`pom.xml` / `build.gradle.kts` are scaffold-only, written once and
//! user-owned after that), so the two directories are not expected to be identical -- only for
//! the build target to be the source directory's Maven/Gradle project root. Neither side is a
//! hard-coded path literal, so reintroducing the divergence fails this test regardless of which
//! side moves.

use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};
use alef::core::{Backend, GeneratedFile};
use std::path::PathBuf;

/// The first file `JavaBackend::generate_bindings` unconditionally emits, regardless of API
/// shape -- see `backends::java::gen_bindings::mod::generate_bindings`.
const JAVA_MARKER: &str = "package-info.java";

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "toolkit".to_string(),
        functions: vec![FunctionDef {
            name: "summarize".to_string(),
            rust_path: "toolkit::summarize".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn java_config(output: Option<&str>) -> ResolvedCrateConfig {
    let output_toml = output
        .map(|path| format!("\n[crates.output]\njava = \"{path}\"\n"))
        .unwrap_or_default();
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.toolkit"
{output_toml}"#
    ))
    .expect("fixture config parses");
    cfg.resolve().expect("fixture config resolves").remove(0)
}

fn kotlin_config(output: Option<&str>) -> ResolvedCrateConfig {
    let output_toml = output
        .map(|path| format!("\n[crates.output]\nkotlin = \"{path}\"\n"))
        .unwrap_or_default();
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.toolkit"
target = "jvm"
{output_toml}"#
    ))
    .expect("fixture config parses");
    cfg.resolve().expect("fixture config resolves").remove(0)
}

/// The directory the Java backend actually wrote sources into, taken from its emitted file
/// list rather than from any path formula.
fn generated_java_source_dir(files: &[GeneratedFile]) -> PathBuf {
    let marker = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == JAVA_MARKER))
        .unwrap_or_else(|| {
            panic!(
                "the java backend emitted no `{JAVA_MARKER}` among: {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        });
    marker
        .path
        .parent()
        .expect("an emitted package-info.java always has a parent directory")
        .to_path_buf()
}

/// The directory the Kotlin (JVM) backend actually wrote its module source file into.
///
/// `KotlinBackend::generate_bindings` always emits `<PascalCaseCrate>.kt` for the default JVM
/// target when the crate has at least one visible function -- see
/// `backends::kotlin::gen_bindings::generate_jvm`.
fn generated_kotlin_source_dir(files: &[GeneratedFile]) -> PathBuf {
    let marker = files
        .iter()
        .find(|file| file.path.extension().is_some_and(|ext| ext == "kt"))
        .unwrap_or_else(|| {
            panic!(
                "the kotlin backend emitted no `.kt` file among: {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        });
    marker
        .path
        .parent()
        .expect("an emitted .kt file always has a parent directory")
        .to_path_buf()
}

/// The project root `mvn -f {root}/pom.xml …` targets, parsed back out of the command a
/// consumer's `alef build` would actually run.
fn build_command_target_dir_java(config: &ResolvedCrateConfig) -> PathBuf {
    let command = config
        .build_command_config_for_language(Language::Java)
        .build
        .expect("java has a default build command")
        .commands()
        .join(" && ");
    let after_flag = command
        .split_once("-f ")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("expected `mvn -f <path>/pom.xml …` in the java build command, got `{command}`"));
    let pom_path = after_flag.split_once(' ').map_or(after_flag, |(path, _)| path);
    PathBuf::from(pom_path)
        .parent()
        .expect("a resolved pom.xml path always has a parent directory")
        .to_path_buf()
}

/// The project root `cd {root} && gradle …` targets, parsed back out of the command a
/// consumer's `alef build` would actually run.
fn build_command_target_dir_kotlin(config: &ResolvedCrateConfig) -> PathBuf {
    let command = config
        .build_command_config_for_language(Language::Kotlin)
        .build
        .expect("kotlin has a default build command")
        .commands()
        .join(" && ");
    let after_cd = command
        .strip_prefix("cd ")
        .unwrap_or_else(|| panic!("expected the kotlin build command to start with `cd `, got `{command}`"));
    let dir = after_cd.split_once(" &&").map_or(after_cd, |(dir, _)| dir);
    PathBuf::from(dir)
}

#[test]
fn unconfigured_java_build_command_targets_an_ancestor_of_the_generated_sources() {
    let config = java_config(None);
    let files = alef::backends::java::JavaBackend
        .generate_bindings(&api(), &config)
        .expect("java bindings generate");

    let generated = generated_java_source_dir(&files);
    let built = build_command_target_dir_java(&config);

    assert!(
        generated.starts_with(&built),
        "`alef generate` wrote java sources to `{}` but `alef build` runs mvn against `{}`, which is not \
         an ancestor of the generated sources",
        generated.display(),
        built.display()
    );
}

/// The case the bug description calls out by name: a configured `[crates.output].java` that
/// moves the tree to somewhere unrelated to `packages/java`, spelled out in the full Maven
/// source-set shape `tslp` actually configures.
#[test]
fn configured_java_build_command_targets_an_ancestor_of_the_generated_sources() {
    let config = java_config(Some("sdk/java/src/main/java/"));
    let files = alef::backends::java::JavaBackend
        .generate_bindings(&api(), &config)
        .expect("java bindings generate");

    let generated = generated_java_source_dir(&files);
    let built = build_command_target_dir_java(&config);

    assert!(
        generated.starts_with(&built),
        "`alef generate` wrote java sources to `{}` but `alef build` runs mvn against `{}`, which is not \
         an ancestor of the generated sources",
        generated.display(),
        built.display()
    );
    assert_eq!(
        built,
        PathBuf::from("sdk/java"),
        "the moved tree's project root must follow the configured output path, not the packages/java default"
    );
}

#[test]
fn unconfigured_kotlin_build_command_targets_an_ancestor_of_the_generated_sources() {
    let config = kotlin_config(None);
    let files = alef::backends::kotlin::KotlinBackend
        .generate_bindings(&api(), &config)
        .expect("kotlin bindings generate");

    let generated = generated_kotlin_source_dir(&files);
    let built = build_command_target_dir_kotlin(&config);

    assert!(
        generated.starts_with(&built),
        "`alef generate` wrote kotlin sources to `{}` but `alef build` runs gradle in `{}`, which is not \
         an ancestor of the generated sources",
        generated.display(),
        built.display()
    );
}

/// The configured case that moves the tree to somewhere unrelated to `packages/kotlin`.
#[test]
fn configured_kotlin_build_command_targets_an_ancestor_of_the_generated_sources() {
    let config = kotlin_config(Some("sdk/kotlin/src/main/kotlin/dev/toolkit/"));
    let files = alef::backends::kotlin::KotlinBackend
        .generate_bindings(&api(), &config)
        .expect("kotlin bindings generate");

    let generated = generated_kotlin_source_dir(&files);
    let built = build_command_target_dir_kotlin(&config);

    assert!(
        generated.starts_with(&built),
        "`alef generate` wrote kotlin sources to `{}` but `alef build` runs gradle in `{}`, which is not \
         an ancestor of the generated sources",
        generated.display(),
        built.display()
    );
    assert_eq!(
        built,
        PathBuf::from("sdk/kotlin"),
        "the moved tree's project root must follow the configured output path, not the packages/kotlin default"
    );
}
