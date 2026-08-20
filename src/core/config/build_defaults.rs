use super::extras::Language;
use super::output::{BuildCommandConfig, StringOrVec};
use super::tools::{LangContext, require_tool, wrap_command as wrap};
use crate::core::template_versions as tv;

/// `maturin develop`'s own environment resolution, minus its parent-directory walk: it needs
/// `VIRTUAL_ENV`, `CONDA_PREFIX`, or a `.venv` directory, and without one it exits in tens of
/// milliseconds — long before any compilation could have started. A build failure that fast is
/// never a defect in generated code, so the check that would have caught it belongs here rather
/// than in a reader's head. ~keep
const PYTHON_ENVIRONMENT_CHECK: &str = r#"[ -n "$VIRTUAL_ENV" ] || [ -n "$CONDA_PREFIX" ] || [ -d .venv ]"#;

/// The one command that creates the interpreter environment `maturin develop` installs into,
/// phrased for whichever package manager `[tools] python_package_manager` selected.
fn python_environment_remediation(package_manager: &str) -> String {
    match package_manager {
        "poetry" => "poetry install".to_string(),
        "uv" => "uv venv".to_string(),
        _ => "python3 -m venv .venv".to_string(),
    }
}

/// `mix compile` refuses to run against unfetched dependencies ("the dependency is not available,
/// run `mix deps.get`"), and `deps/` is what `mix deps.get` creates — untracked, so absent on
/// every fresh checkout.
///
/// Gating on `mix.lock` instead was considered and rejected: alef does not scaffold a lockfile,
/// so a check that only fires when one exists would pass on exactly the pristine checkout that
/// motivated it and examine nothing. The residual cost is a dependency-free mix project, which
/// would be skipped forever — but the mix.exs alef scaffolds always declares `rustler`,
/// `rustler_precompiled`, `credo`, and `ex_doc` (see `scaffold::languages::elixir`), so that
/// project cannot be one alef generated. A user who hand-writes one overrides
/// `dependency_precondition`. ~keep
fn mix_dependency_check(output_dir: &str) -> String {
    format!("[ -d {output_dir}/deps ]")
}

/// Return the default build configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). The `crate_name` is the name of the core crate
/// (e.g. `my-lib`). Both are substituted into command templates. `ctx`
/// provides tool selection and run_wrapper.
pub(crate) fn default_build_config(
    lang: Language,
    output_dir: &str,
    crate_name: &str,
    ctx: &LangContext,
) -> BuildCommandConfig {
    match lang {
        Language::Rust => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single("cargo build --workspace".to_string())),
            build_release: Some(StringOrVec::Single("cargo build --release --workspace".to_string())),
        },
        Language::Python => BuildCommandConfig {
            precondition: Some(require_tool("maturin")),
            dependency_precondition: Some(PYTHON_ENVIRONMENT_CHECK.to_string()),
            dependency_remediation: Some(python_environment_remediation(ctx.tools.python_pm())),
            before: None,
            build: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml --release"
            ))),
        },
        Language::Node => BuildCommandConfig {
            precondition: Some(require_tool("npm")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!(
                "npx --yes -p @napi-rs/cli@3.7.3 napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --dts {}",
                tv::npm::NAPI_AUTO_DTS_FILENAME
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "npx --yes -p @napi-rs/cli@3.7.3 napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --dts {} --release",
                tv::npm::NAPI_AUTO_DTS_FILENAME
            ))),
        },
        Language::Wasm => BuildCommandConfig {
            precondition: Some(require_tool("wasm-pack")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --dev"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --release"
            ))),
        },
        Language::Go => {
            let cmd = format!("cd {output_dir} && go build ./...");
            BuildCommandConfig {
                precondition: Some(require_tool("go")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(cmd.clone(), ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(cmd, ctx.run_wrapper))),
            }
        }
        Language::Ruby => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-rb"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-rb"))),
        },
        Language::Php => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-php"))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-php"
            ))),
        },
        Language::Ffi => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-ffi"))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-ffi"
            ))),
        },
        Language::Java => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("mvn -f {proj} package -DskipTests --batch-mode --no-transfer-progress"),
                    format!("mvn -f {proj} package -DskipTests --batch-mode --no-transfer-progress"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests --batch-mode --no-transfer-progress"),
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests --batch-mode --no-transfer-progress"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("mvn")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
            }
        }
        Language::Csharp => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("dotnet build {proj} --configuration Debug -q"),
                    format!("dotnet build {proj} --configuration Release -q"),
                )
            } else {
                (
                    format!("dotnet build {output_dir} --configuration Debug -q"),
                    format!("dotnet build {output_dir} --configuration Release -q"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("dotnet")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
            }
        }
        Language::Elixir => BuildCommandConfig {
            precondition: Some(require_tool("mix")),
            dependency_precondition: Some(mix_dependency_check(output_dir)),
            dependency_remediation: Some(format!("cd {output_dir} && mix deps.get")),
            before: None,
            build: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
            build_release: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
        },
        Language::R => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-r"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-r"))),
        },
        Language::Kotlin => BuildCommandConfig {
            precondition: Some(require_tool("gradle")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle build -Prelease"),
                ctx.run_wrapper,
            ))),
        },
        Language::KotlinAndroid => BuildCommandConfig {
            precondition: Some(require_tool("gradle")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle assembleDebug"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle assembleRelease"),
                ctx.run_wrapper,
            ))),
        },
        Language::Swift => BuildCommandConfig {
            precondition: Some(require_tool("swift")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir}"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir} --configuration release"),
                ctx.run_wrapper,
            ))),
        },
        Language::Dart => BuildCommandConfig {
            precondition: Some(require_tool("dart")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
        },
        Language::Zig => BuildCommandConfig {
            precondition: Some(require_tool("zig")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build --release=fast"),
                ctx.run_wrapper,
            ))),
        },
        Language::Gleam => BuildCommandConfig {
            precondition: Some(require_tool("gleam")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
        },
        Language::C => BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: None,
            build_release: None,
        },
        Language::Jni => BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: None,
            build_release: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::ToolsConfig;
    use super::*;

    fn all_languages() -> Vec<Language> {
        vec![
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
            Language::Ffi,
            Language::Rust,
            Language::Kotlin,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
        ]
    }

    fn cfg(lang: Language, dir: &str, crate_name: &str) -> BuildCommandConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_build_config(lang, dir, crate_name, &ctx)
    }

    #[test]
    fn every_language_has_build_and_build_release() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            assert!(c.build.is_some(), "{lang} should have a default build command");
            assert!(
                c.build_release.is_some(),
                "{lang} should have a default build_release command"
            );
        }
    }

    #[test]
    fn every_language_has_default_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    /// Every dependency check must arrive with the command that satisfies it — the whole reason
    /// this outcome beats a bare failure is that it can tell the reader what to run. Enforced for
    /// user config in `validation::preconditions`; enforced for alef's own defaults here. ~keep
    #[test]
    fn every_dependency_precondition_ships_with_its_remediation() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            assert_eq!(
                c.dependency_precondition.is_some(),
                c.dependency_remediation.is_some(),
                "{lang} must declare a dependency check and its remediation together"
            );
        }
    }

    /// The deliberate short list, pinned so it stays deliberate. Every language left out builds
    /// through a tool that resolves its own dependencies as part of the build (cargo, gradle,
    /// maven, dotnet, go, swiftpm, zig, gleam, pub) — giving those a dependency precondition
    /// would skip builds that work today, which is a worse defect than the one being fixed. ~keep
    #[test]
    fn only_tools_that_refuse_to_fetch_their_own_dependencies_declare_a_dependency_precondition() {
        let gated: Vec<Language> = all_languages()
            .into_iter()
            .filter(|lang| cfg(*lang, "packages/test", "my-lib").dependency_precondition.is_some())
            .collect();

        assert_eq!(gated, vec![Language::Python, Language::Elixir]);
    }

    #[test]
    fn python_dependency_precondition_matches_maturin_environment_resolution() {
        let c = cfg(Language::Python, "packages/python", "my-lib");
        let check = c.dependency_precondition.expect("python declares a dependency check");

        assert!(check.contains("VIRTUAL_ENV"), "{check}");
        assert!(check.contains("CONDA_PREFIX"), "{check}");
        assert!(check.contains(".venv"), "{check}");
        assert_eq!(c.dependency_remediation.as_deref(), Some("uv venv"));
    }

    #[test]
    fn python_remediation_follows_the_configured_package_manager() {
        assert_eq!(python_environment_remediation("uv"), "uv venv");
        assert_eq!(python_environment_remediation("poetry"), "poetry install");
        assert_eq!(python_environment_remediation("pip"), "python3 -m venv .venv");
    }

    #[test]
    fn elixir_dependency_precondition_points_at_mix_deps_get() {
        let c = cfg(Language::Elixir, "packages/elixir", "my-lib");

        assert_eq!(
            c.dependency_precondition.as_deref(),
            Some("[ -d packages/elixir/deps ]")
        );
        assert_eq!(
            c.dependency_remediation.as_deref(),
            Some("cd packages/elixir && mix deps.get")
        );
    }

    /// Runs the emitted shell string against a real directory tree rather than asserting on its
    /// text: the check is a command, and a command that reads correctly but exits wrong would
    /// either skip every elixir build forever or examine nothing at all. The first case here is
    /// the pristine checkout that motivated the change — a scaffolded mix project with a mix.exs
    /// and no fetched dependencies — and it must fail. ~keep
    #[test]
    fn mix_dependency_check_fails_on_a_pristine_checkout_and_passes_once_deps_are_fetched() {
        let root = tempfile::tempdir().expect("tempdir");
        // The check is emitted with the *relative* `output_dir` a real config carries
        // (`packages/elixir`), and is run from the workspace root. Handing `sh` an absolute path
        // instead would feed it `C:\Users\...` on Windows, where every backslash is an `sh`
        // escape -- the test then answers "no deps" whatever is on disk, and its first assertion
        // passes for the wrong reason. ~keep
        const PACKAGE: &str = "packages/elixir";
        let package = root.path().join(PACKAGE);
        std::fs::create_dir_all(&package).expect("create package dir");
        let passes = || {
            std::process::Command::new("sh")
                .args(["-c", &mix_dependency_check(PACKAGE)])
                .current_dir(root.path())
                .status()
                .expect("check runs")
                .success()
        };
        std::fs::write(package.join("mix.exs"), "defmodule Sample.MixProject do\nend\n").expect("write mix.exs");

        assert!(
            !passes(),
            "a checked-out mix project with no deps/ has not run `mix deps.get`"
        );

        std::fs::create_dir(package.join("deps")).expect("create deps");
        assert!(passes(), "fetched deps must let the build through");
    }

    #[test]
    fn rust_uses_cargo_build_workspace() {
        let c = cfg(Language::Rust, "packages/rust", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build --workspace"));
        assert!(release.contains("cargo build --release --workspace"));
    }

    #[test]
    fn python_uses_maturin_develop() {
        let c = cfg(Language::Python, "packages/python", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("maturin develop"));
        assert!(build.contains("my-lib-py"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn node_uses_napi_build() {
        let c = cfg(Language::Node, "packages/node", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("npx --yes -p @napi-rs/cli@3.7.3 napi"));
        assert!(build.contains("build --manifest-path"));
        assert!(build.contains("my-lib-node"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn wasm_uses_wasm_pack() {
        let c = cfg(Language::Wasm, "packages/wasm", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("wasm-pack build"));
        assert!(build.contains("my-lib-wasm"));
        assert!(build.contains("--dev"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn ffi_uses_cargo_build_p() {
        let c = cfg(Language::Ffi, "packages/ffi", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-ffi"));
        assert!(release.contains("cargo build --release -p my-lib-ffi"));
    }

    #[test]
    fn ruby_uses_cargo_build_rb() {
        let c = cfg(Language::Ruby, "packages/ruby", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-rb"));
    }

    #[test]
    fn php_uses_cargo_build_php() {
        let c = cfg(Language::Php, "packages/php", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-php"));
    }

    #[test]
    fn r_uses_cargo_build_r() {
        let c = cfg(Language::R, "packages/r", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-r"));
    }

    #[test]
    fn java_uses_maven_package() {
        let c = cfg(Language::Java, "packages/java", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mvn"));
        assert!(build.contains("package"));
        assert!(build.contains("-DskipTests"));
    }

    #[test]
    fn csharp_uses_dotnet_build_configurations() {
        let c = cfg(Language::Csharp, "packages/csharp", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("dotnet build"));
        assert!(build.contains("--configuration Debug"));
        assert!(release.contains("--configuration Release"));
    }

    #[test]
    fn elixir_uses_mix_compile() {
        let c = cfg(Language::Elixir, "packages/elixir", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mix compile"));
    }

    #[test]
    fn crate_name_substituted_in_commands() {
        let c = cfg(Language::Python, "packages/python", "custom-crate");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("custom-crate-py"));
    }

    #[test]
    fn output_dir_substituted_in_go_commands() {
        let c = cfg(Language::Go, "my/custom/path", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("my/custom/path"));
    }

    #[test]
    fn kotlin_uses_gradle_build() {
        let c = cfg(Language::Kotlin, "packages/kotlin", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("gradle build"),
            "Kotlin build should use gradle build, got: {build}"
        );
        assert!(
            release.contains("gradle build"),
            "Kotlin release should use gradle build, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    /// `KotlinAndroid`'s default here is a second, independent derivation of "the
    /// kotlin_android build command" from `build_command_for`'s `"gradle"` arm in
    /// `src/cli/pipeline/commands/build.rs` (`cd <settings.gradle-root, found by walking
    /// up from the source output dir> && gradle build`). This one is reached only through
    /// `ResolvedCrateConfig::build_command_config_for_language`
    /// (`src/core/config/resolved/lookups.rs`) when an alef.toml declares ANY
    /// `[crates.build_commands.kotlin_android]` overlay — even a partial one that leaves
    /// `build`/`build_release` unset, since `BuildCommandConfig::merge_overlay` keeps this
    /// default for whatever fields the overlay omits. It also never walks up: `output_dir`
    /// comes from `package_dir(KotlinAndroid)`, which deliberately ignores an explicit
    /// `[crates.output] kotlin_android` override (see
    /// `package_dir_kotlin_ignores_source_output_override`-style tests in
    /// `resolved/lookups.rs`) and always resolves to the fixed `"packages/kotlin-android"`.
    ///
    /// `assembleDebug`/`assembleRelease` and `gradle build` are both defensible commands for
    /// an Android library module — `assemble*` is the narrower, variant-scoped AGP task,
    /// while `build` is the umbrella task every other backend arm here defaults to — so this
    /// pins the current, accepted divergence instead of letting the two silently drift
    /// further apart. Changing either command, or giving this arm the same walk-up as the
    /// `build.rs` one, is a deliberate decision that must update this test too. ~keep
    #[test]
    fn kotlin_android_default_diverges_intentionally_from_build_command_for_gradle_arm() {
        let c = cfg(Language::KotlinAndroid, "packages/kotlin-android", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert_eq!(build, "cd packages/kotlin-android && gradle assembleDebug");
        assert_eq!(release, "cd packages/kotlin-android && gradle assembleRelease");
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_build_with_package_path() {
        let c = cfg(Language::Swift, "packages/swift", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("swift build"),
            "Swift build should use swift build, got: {build}"
        );
        assert!(
            build.contains("--package-path packages/swift"),
            "Swift build should include package path, got: {build}"
        );
        assert!(
            release.contains("--configuration release"),
            "Swift release should use --configuration release, got: {release}"
        );
    }

    #[test]
    fn dart_uses_dart_pub_get() {
        let c = cfg(Language::Dart, "packages/dart", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(
            build.contains("dart pub get"),
            "Dart build should use dart pub get, got: {build}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_build() {
        let c = cfg(Language::Gleam, "packages/gleam", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("gleam build"),
            "Gleam build should use gleam build, got: {build}"
        );
        assert!(
            release.contains("gleam build"),
            "Gleam release should use gleam build, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build() {
        let c = cfg(Language::Zig, "packages/zig", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("zig build"),
            "Zig build should use zig build, got: {build}"
        );
        assert!(
            release.contains("--release=fast"),
            "Zig release should use --release=fast, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
