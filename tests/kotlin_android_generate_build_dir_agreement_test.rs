//! On a stock config with no `[crates.output]` table, alef wrote the Android Gradle project to
//! one directory and built it from another.
//!
//! `resolve_output_paths` fills `output_paths` for *every* targeted language from
//! `OutputTemplate`, so `output_for("kotlin_android")` is always `Some`, and the unconfigured
//! template default was `packages/{lang}` — spelled from the config key, giving
//! `packages/kotlin_android`. `KotlinAndroidBackend` writes `build.gradle.kts`, `gradlew`,
//! `AndroidManifest.xml` and the Kotlin sources there. Meanwhile `package_dir`, which is what
//! `build_command_config_for_language` interpolates into `cd {output_dir} && gradle …` (and what
//! the lint, test, clean, setup and `sync_versions` paths use), answered
//! `packages/kotlin-android`. Every gradle invocation therefore targeted a directory the
//! generator had never written to.
//!
//! This test runs the *real* backend and the *real* build-command resolution and asserts they
//! land on the same directory. Neither side is a hard-coded path literal, so reintroducing the
//! divergence fails this test regardless of which side moves.

use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};
use alef::core::{Backend, GeneratedFile};
use std::path::{Path, PathBuf};

/// The file the Gradle build needs to find in its working directory. If `cd {dir} && gradle`
/// runs somewhere this file is not, the build cannot succeed.
const GRADLE_PROJECT_MARKER: &str = "build.gradle.kts";

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

/// A stock single-crate config targeting `kotlin_android` with no `[crates.output]` table at all
/// — the shape that reproduced the split.
fn unconfigured_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.toolkit"
"#,
    )
    .expect("fixture config parses");
    cfg.resolve().expect("fixture config resolves").remove(0)
}

/// The directory the backend actually wrote the Gradle project into, taken from its emitted file
/// list rather than from any path formula.
fn generated_gradle_project_dir(files: &[GeneratedFile]) -> PathBuf {
    let marker = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == GRADLE_PROJECT_MARKER))
        .unwrap_or_else(|| {
            panic!(
                "the kotlin_android backend emitted no `{GRADLE_PROJECT_MARKER}` among: {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        });
    marker
        .path
        .parent()
        .expect("an emitted build.gradle.kts always has a parent directory")
        .to_path_buf()
}

/// Undo POSIX single-quote grouping so a token lifted out of a shell command can be compared as
/// a path.
///
/// The build command is a *shell* string (`cd 'packages/kotlin-android' && gradle …`) — the
/// quoting is what stops a configured `[crates.output]` path from being executed. Comparing the
/// raw token against a `PathBuf` would compare quoting spelling rather than the directory, and
/// would fail whenever the escaping policy changes without the target directory moving, which is
/// the opposite of what this test exists to detect. ~keep
fn unquote_shell_word(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars();
    let mut in_single = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => in_single = !in_single,
            '\\' if !in_single => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

/// The directory the resolved build command changes into, parsed back out of the command a
/// consumer's `alef build` would run.
fn build_command_target_dir(config: &ResolvedCrateConfig) -> PathBuf {
    let command = config
        .build_command_config_for_language(Language::KotlinAndroid)
        .build
        .expect("kotlin_android has a default build command")
        .commands()
        .join(" && ");
    let after_cd = command
        .strip_prefix("cd ")
        .unwrap_or_else(|| panic!("expected the kotlin_android build command to start with `cd `, got `{command}`"));
    let dir = after_cd
        .split_once(" &&")
        .map(|(dir, _)| dir)
        .unwrap_or_else(|| panic!("expected `cd <dir> && …` in the kotlin_android build command, got `{command}`"));
    PathBuf::from(unquote_shell_word(dir))
}

#[test]
fn unconfigured_kotlin_android_generates_into_the_directory_the_build_command_targets() {
    let config = unconfigured_config();
    let files = alef::backends::kotlin_android::KotlinAndroidBackend
        .generate_bindings(&api(), &config)
        .expect("kotlin_android bindings generate");

    let generated = generated_gradle_project_dir(&files);
    let built = build_command_target_dir(&config);

    assert_eq!(
        generated,
        built,
        "`alef generate` wrote the Gradle project to `{}` while `alef build` runs gradle in `{}`; \
         the build cannot see the project alef just generated",
        generated.display(),
        built.display()
    );
}

/// The same agreement stated against the accessor the rest of the pipeline reads — lint, test,
/// clean, setup, scaffolding and `sync_versions` all go through `package_dir` rather than
/// re-parsing the build command, so pinning only the build command would leave them free to
/// drift back.
#[test]
fn unconfigured_kotlin_android_package_dir_matches_the_generated_project_root() {
    let config = unconfigured_config();
    let files = alef::backends::kotlin_android::KotlinAndroidBackend
        .generate_bindings(&api(), &config)
        .expect("kotlin_android bindings generate");

    assert_eq!(
        generated_gradle_project_dir(&files),
        Path::new(&config.package_dir(Language::KotlinAndroid)),
        "package_dir must name the directory the backend writes the Gradle project into"
    );
}

/// Both halves of the split derive from the same constant now, but a reader should still be able
/// to see which directory that is: the hyphenated Gradle project root every consumer has on disk,
/// and the one the scaffolded `.gitattributes`, the lint/clean/setup defaults and
/// `sync_versions`' manifest path already named before this fix.
#[test]
fn the_agreed_kotlin_android_directory_is_the_hyphenated_gradle_project_root() {
    let config = unconfigured_config();
    assert_eq!(
        config.output_for("kotlin_android"),
        Some(Path::new("packages/kotlin-android")),
        "the unconfigured generator output must be the hyphenated Gradle project root"
    );
}
