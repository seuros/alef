use super::extras::Language;
use super::output::{ArgvRunConfig, ArgvStep, CleanConfig, StringOrVec};
use super::tools::{LangContext, require_tool};

/// Build an [`ArgvRunConfig`] for a single-step clean command that `cd`s into `output_dir`
/// then runs `command` with `args`, none of it shell text.
///
/// `output_dir` is `[crates.output]`/scaffold-output-derived and therefore a free-form,
/// user-authored path -- it used to be spliced into `format!("cd {output_dir} && ...")`
/// shell strings here, which let `;`, backticks, or `$(...)` in the path execute arbitrary
/// commands (worst case: the default clean commands run `rm -rf`). `current_dir` and argv
/// arguments make it a single opaque element instead, the same fix shape as the Go test-app
/// run default. ~keep
fn cd_and_run(output_dir: &str, command: &str, args: &[&str]) -> ArgvRunConfig {
    ArgvRunConfig {
        work_dir: output_dir.to_owned(),
        env: Vec::new(),
        steps: vec![ArgvStep {
            command: command.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }],
    }
}

/// Return the default clean configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` is provided but not used; clean commands don't depend on the
/// chosen package manager.
///
/// Languages whose clean command relies only on POSIX shell builtins
/// (e.g. plain `rm -rf`) leave `precondition` as `None` since `rm` is
/// effectively always present on supported platforms.
pub(crate) fn default_clean_config(lang: Language, output_dir: &str, _ctx: &LangContext) -> CleanConfig {
    match lang {
        Language::Rust => CleanConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            clean: Some(StringOrVec::Single("cargo clean".to_string())),
            argv_clean: None,
        },
        Language::Python => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(
                output_dir,
                "rm",
                &[
                    "-rf",
                    "__pycache__",
                    ".pytest_cache",
                    ".mypy_cache",
                    ".ruff_cache",
                    "dist",
                ],
            )),
        },
        Language::Node | Language::Wasm => CleanConfig {
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("rm -rf node_modules dist .turbo".to_string())),
            argv_clean: None,
        },
        Language::Go => CleanConfig {
            precondition: Some(require_tool("go")),
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(output_dir, "go", &["clean", "-cache"])),
        },
        Language::Ruby => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(output_dir, "rm", &["-rf", "tmp", "vendor", ".bundle"])),
        },
        Language::Php => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(output_dir, "rm", &["-rf", "vendor", "var"])),
        },
        Language::Java => CleanConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            clean: None,
            argv_clean: Some(ArgvRunConfig {
                work_dir: ".".to_owned(),
                env: Vec::new(),
                steps: vec![ArgvStep {
                    command: "mvn".to_owned(),
                    args: vec![
                        "-f".to_owned(),
                        format!("{output_dir}/pom.xml"),
                        "clean".to_owned(),
                        "--batch-mode".to_owned(),
                        "--no-transfer-progress".to_owned(),
                    ],
                }],
            }),
        },
        Language::Csharp => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: None,
        },
        Language::Elixir => CleanConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            clean: None,
            argv_clean: Some(ArgvRunConfig {
                work_dir: output_dir.to_owned(),
                env: Vec::new(),
                steps: vec![
                    ArgvStep {
                        command: "mix".to_owned(),
                        args: vec!["clean".to_owned()],
                    },
                    ArgvStep {
                        command: "rm".to_owned(),
                        args: vec!["-rf".to_owned(), "deps".to_owned(), "_build".to_owned()],
                    },
                ],
            }),
        },
        Language::R => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(output_dir, "rm", &["-rf", "src/rust/target"])),
        },
        Language::Ffi => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: None,
        },
        Language::Kotlin => CleanConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            clean: None,
            argv_clean: Some(cd_and_run(output_dir, "gradle", &["clean"])),
        },
        Language::KotlinAndroid => CleanConfig {
            // `precondition` still interpolates `output_dir` into shell text (`test -x
            // {output_dir}/gradlew`) -- CleanConfig::precondition has no typed argv
            // alternative yet, so this specific check remains a known, unfixed instance of
            // the same defect shape as `argv_clean` exists to close. Tracked as a remaining
            // gap rather than silently left implying it was covered. ~keep
            precondition: Some(format!("test -x {output_dir}/gradlew")),
            before: None,
            clean: None,
            // The executable path is resolved relative to *this* process's cwd before
            // `current_dir` below ever takes effect (a documented `std::process::Command`
            // gotcha), so `./gradlew` alone would look in alef's own cwd, not `output_dir`.
            // `{output_dir}/gradlew` names the same file the shell version's `cd
            // {output_dir} && ./gradlew` invoked, resolved the way this API actually
            // resolves it. ~keep
            argv_clean: Some(ArgvRunConfig {
                work_dir: output_dir.to_owned(),
                env: Vec::new(),
                steps: vec![ArgvStep {
                    command: format!("{output_dir}/gradlew"),
                    args: vec!["clean".to_owned()],
                }],
            }),
        },
        Language::Swift => CleanConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            clean: Some(StringOrVec::Single("swift package clean".to_string())),
            argv_clean: None,
        },
        Language::Dart => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: None,
        },
        Language::Zig => CleanConfig {
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("rm -rf zig-out zig-cache .zig-cache".to_string())),
            argv_clean: None,
        },
        Language::Gleam => CleanConfig {
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("rm -rf build".to_string())),
            argv_clean: None,
        },
        Language::C => CleanConfig {
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("cd e2e/c && make clean".to_string())),
            argv_clean: None,
        },
        Language::Jni => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
            argv_clean: None,
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

    fn cfg(lang: Language, dir: &str) -> CleanConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_clean_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_has_no_clean_command() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.clean.is_none());
    }

    #[test]
    fn languages_with_project_agnostic_cleaning_have_defaults() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi | Language::Csharp | Language::Dart) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(
                c.clean.is_some() || c.argv_clean.is_some(),
                "{lang} should have a default clean command"
            );
        }
    }

    #[test]
    fn toolchain_clean_has_precondition() {
        for lang in [Language::Rust, Language::Go, Language::Java, Language::Elixir] {
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn pure_shell_clean_omits_precondition() {
        for lang in [
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::R,
        ] {
            let c = cfg(lang, "packages/test");
            assert!(
                c.precondition.is_none(),
                "{lang} pure-shell clean should not have a precondition"
            );
        }
    }

    #[test]
    fn rust_uses_cargo_clean() {
        let c = cfg(Language::Rust, "packages/rust");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("cargo clean"));
    }

    #[test]
    fn python_removes_pycache_and_dist() {
        let c = cfg(Language::Python, "packages/python");
        assert!(
            c.clean.is_none(),
            "python clean should be argv-only, got: {:?}",
            c.clean
        );
        let argv = c.argv_clean.expect("python should have an argv clean command");
        assert_eq!(argv.work_dir, "packages/python");
        assert_eq!(argv.steps.len(), 1);
        let step = &argv.steps[0];
        assert_eq!(step.command, "rm");
        for expected in [
            "-rf",
            "__pycache__",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            "dist",
        ] {
            assert!(
                step.args.iter().any(|a| a == expected),
                "expected arg {expected:?} in {:?}",
                step.args
            );
        }
    }

    /// RED (pre-fix)/GREEN (post-fix): a Python package output directory containing shell
    /// metacharacters must never be reinterpreted by a shell -- it must arrive as a single,
    /// literal `current_dir`/argv element. This is the exact defect shape the Go test-app
    /// run default had (see `test_apps_run_defaults` tests): a config-supplied path spliced
    /// unquoted into `format!("cd {output_dir} && rm -rf ...")` let `;`, backticks, or
    /// `$(...)` in the path execute arbitrary commands, worst case alongside `rm -rf`.
    #[test]
    fn python_clean_treats_malicious_output_dir_as_a_single_literal_argument() {
        let malicious = "packages/python; touch pwned; echo";
        let c = cfg(Language::Python, malicious);
        assert!(
            c.clean.is_none(),
            "a shell-string `clean` here would let the payload execute; must be argv-only"
        );
        let argv = c.argv_clean.expect("python should have an argv clean command");
        assert_eq!(
            argv.work_dir, malicious,
            "the whole payload must survive verbatim as one `current_dir` value, not be shell-split"
        );
    }

    #[test]
    fn node_removes_node_modules() {
        let c = cfg(Language::Node, "packages/node");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("node_modules"));
        assert!(clean.contains("dist"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        assert_eq!(
            node.clean.unwrap().commands().join(" "),
            wasm.clean.unwrap().commands().join(" "),
        );
    }

    #[test]
    fn go_uses_go_clean() {
        let c = cfg(Language::Go, "packages/go");
        let argv = c.argv_clean.expect("go should have an argv clean command");
        assert_eq!(argv.work_dir, "packages/go");
        assert_eq!(argv.steps.len(), 1);
        assert_eq!(argv.steps[0].command, "go");
        assert_eq!(argv.steps[0].args, vec!["clean", "-cache"]);
    }

    #[test]
    fn java_uses_maven_clean() {
        let c = cfg(Language::Java, "packages/java");
        let argv = c.argv_clean.expect("java should have an argv clean command");
        assert_eq!(argv.steps.len(), 1);
        let step = &argv.steps[0];
        assert_eq!(step.command, "mvn");
        assert!(step.args.contains(&"clean".to_string()));
        assert!(step.args.contains(&"packages/java/pom.xml".to_string()));
    }

    #[test]
    fn csharp_requires_an_explicit_project_aware_clean_command() {
        let c = cfg(Language::Csharp, "packages/csharp");
        assert!(c.clean.is_none());
    }

    #[test]
    fn kotlin_android_uses_its_generated_gradle_wrapper() {
        let c = cfg(Language::KotlinAndroid, "packages/kotlin-android");
        assert_eq!(
            c.precondition.as_deref(),
            Some("test -x packages/kotlin-android/gradlew")
        );
        let argv = c.argv_clean.expect("kotlin_android should have an argv clean command");
        assert_eq!(argv.work_dir, "packages/kotlin-android");
        assert_eq!(argv.steps.len(), 1);
        assert_eq!(argv.steps[0].command, "packages/kotlin-android/gradlew");
        assert_eq!(argv.steps[0].args, vec!["clean"]);
    }

    #[test]
    fn dart_requires_an_explicit_project_aware_clean_command() {
        let c = cfg(Language::Dart, "packages/dart/lib/src");
        assert!(c.clean.is_none());
    }

    #[test]
    fn elixir_uses_mix_clean() {
        let c = cfg(Language::Elixir, "packages/elixir");
        let argv = c.argv_clean.expect("elixir should have an argv clean command");
        assert_eq!(argv.work_dir, "packages/elixir");
        assert_eq!(argv.steps.len(), 2);
        assert_eq!(argv.steps[0].command, "mix");
        assert_eq!(argv.steps[0].args, vec!["clean"]);
        assert_eq!(argv.steps[1].command, "rm");
        assert_eq!(argv.steps[1].args, vec!["-rf", "deps", "_build"]);
    }

    #[test]
    fn r_removes_rust_target() {
        let c = cfg(Language::R, "packages/r");
        let argv = c.argv_clean.expect("r should have an argv clean command");
        assert_eq!(argv.steps[0].command, "rm");
        assert!(argv.steps[0].args.iter().any(|a| a == "src/rust/target"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let argv = c.argv_clean.expect("go should have an argv clean command");
        assert_eq!(argv.work_dir, "my/custom/path");
    }
}
