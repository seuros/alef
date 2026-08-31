//! The wiring test for coordinate validation: a hostile coordinate must be rejected by the real
//! production entry point, not merely by a validator someone remembered to call.
//!
//! An earlier version of this work shipped a fully correct grammar with zero production callers,
//! so an invalid coordinate flowed straight through resolution into generated manifests. These
//! tests exist to make that failure mode impossible to reintroduce silently: they drive the
//! actual `alef` binary over an actual `alef.toml`, and they assert on the artifacts on disk.
//!
//! Against the shipped 0.79.2 binary every hostile case below exited 0 and generated files:
//! `<groupId>dev"; System.exit(1); //</groupId>` landed verbatim in `pom.xml`, and a namespace of
//! `My.$(Evil)` produced a `.csproj` containing `<RootNamespace>My.$(Evil)</RootNamespace>`,
//! where `$(Evil)` is live MSBuild property expansion.

use std::path::Path;
use std::process::Command;

use alef::core::config::NewAlefConfig;

fn write_project(dir: &Path, java_package: &str, csharp_namespace: &str) {
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(dir.join("src/lib.rs"), "pub fn add(a: i64, b: i64) -> i64 { a + b }\n").expect("write lib.rs");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(dir.join("alef.toml"), config_toml(java_package, csharp_namespace)).expect("write alef.toml");
}

fn config_toml(java_package: &str, csharp_namespace: &str) -> String {
    format!(
        r#"[workspace]
languages = ["java", "csharp"]

[workspace.package_metadata]
repository = "https://example.com/sample-core"
authors = ["Sample Author <sample@example.com>"]
license = "MIT"
description = "Sample core library"

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.java]
package = {java_package:?}

[crates.csharp]
namespace = {csharp_namespace:?}
"#
    )
}

/// `(name, java package, csharp namespace)` — each is rejected by javac or dotnet, and each was
/// accepted end-to-end by alef before coordinate validation was wired into resolution.
const HOSTILE: &[(&str, &str, &str)] = &[
    ("java keyword segment", "dev.class", "Dev.Sample"),
    ("java path traversal", "../../etc", "Dev.Sample"),
    ("java source injection", "dev\"; System.exit(1); //", "Dev.Sample"),
    ("java empty segment", "dev..sample", "Dev.Sample"),
    ("csharp msbuild injection", "dev.sample", "My.$(Evil)"),
    ("csharp keyword segment", "dev.sample", "My.class"),
    ("csharp xml break-out", "dev.sample", "My\"><Evil/>"),
    ("csharp digit start", "dev.sample", "My.1Lib"),
];

const VALID: (&str, &str) = ("dev.example.samplecore", "Dev.Example.SampleCore");

#[test]
fn hostile_coordinates_are_rejected_by_the_real_cli_before_anything_is_written() {
    for &(name, java_package, csharp_namespace) in HOSTILE {
        let dir = tempfile::tempdir().expect("create temp workspace");
        write_project(dir.path(), java_package, csharp_namespace);

        let output = Command::new(env!("CARGO_BIN_EXE_alef"))
            .arg("--config")
            .arg(dir.path().join("alef.toml"))
            .arg("scaffold")
            .current_dir(dir.path())
            .output()
            .expect("run the alef binary");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "`{name}` must fail `alef scaffold`; it exited {:?}\nstderr:\n{stderr}",
            output.status.code()
        );
        assert!(
            stderr.contains("not a valid coordinate"),
            "`{name}` must fail with the coordinate diagnostic, not an unrelated error\nstderr:\n{stderr}"
        );
        assert!(
            !dir.path().join("packages").exists(),
            "`{name}` must be rejected before any package file is written"
        );
    }
}

#[test]
fn a_valid_coordinate_still_scaffolds_through_the_same_cli_path() {
    // The opposite control. Without it, a validator that rejected everything would pass the test
    // above, and "nothing is generated any more" would read identically to "hostile input is
    // blocked".
    let dir = tempfile::tempdir().expect("create temp workspace");
    write_project(dir.path(), VALID.0, VALID.1);

    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .arg("--config")
        .arg(dir.path().join("alef.toml"))
        .arg("scaffold")
        .current_dir(dir.path())
        .output()
        .expect("run the alef binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "the valid fixture must still scaffold\nstderr:\n{stderr}");
    assert!(
        dir.path().join("packages/java/pom.xml").exists(),
        "the valid fixture must still produce pom.xml\nstderr:\n{stderr}"
    );
    let pom = std::fs::read_to_string(dir.path().join("packages/java/pom.xml")).expect("read pom.xml");
    assert!(pom.contains("<groupId>dev.example.samplecore</groupId>"), "pom.xml:\n{pom}");
}

#[test]
fn resolution_itself_rejects_hostile_coordinates() {
    // Same gate one layer down, at `NewAlefConfig::resolve` — the function `load_config` calls
    // for every alef subcommand. Asserting here as well as through the binary means a refactor
    // that moves the call out of resolution cannot pass by relocating it into one CLI command.
    for &(name, java_package, csharp_namespace) in HOSTILE {
        let config: NewAlefConfig =
            toml::from_str(&config_toml(java_package, csharp_namespace)).expect("fixture parses");
        let error = config
            .resolve()
            .expect_err(&format!("`{name}` must not resolve"))
            .to_string();
        assert!(error.contains("not a valid coordinate"), "`{name}`: {error}");
    }
}

#[test]
fn resolution_accepts_the_valid_coordinate() {
    let config: NewAlefConfig = toml::from_str(&config_toml(VALID.0, VALID.1)).expect("fixture parses");
    let resolved = config.resolve().expect("the valid fixture must resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].java_package(), VALID.0);
    assert_eq!(resolved[0].csharp_namespace(), VALID.1);
}

#[test]
fn coordinate_validation_is_reachable_from_resolution_for_every_wired_language() {
    // Guards the scope of the wiring rather than one language's grammar: if a language's
    // coordinate check is ever dropped from `validate_package_coordinates`, this fails.
    for (language, table, bad) in [
        ("java", "[crates.java]\npackage = \"dev.class\"", "[crates.java].package"),
        ("kotlin", "[crates.kotlin]\npackage = \"dev.fun\"", "[crates.kotlin].package"),
        ("csharp", "[crates.csharp]\nnamespace = \"My.class\"", "[crates.csharp].namespace"),
        ("swift", "[crates.swift]\nmodule_name = \"Sample.Core\"", "[crates.swift].module_name"),
        ("dart", "[crates.dart]\npubspec_name = \"Sample-Core\"", "[crates.dart].pubspec_name"),
    ] {
        let toml = format!(
            "[workspace]\nlanguages = [{language:?}]\n\n\
             [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n{table}\n"
        );
        let config: NewAlefConfig = toml::from_str(&toml).expect("fixture parses");
        let error = config
            .resolve()
            .expect_err(&format!("{language} coordinate must be validated during resolution"))
            .to_string();
        assert!(error.contains(bad), "{language}: expected `{bad}` in: {error}");
    }
}
