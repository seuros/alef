use super::extras::Language;
use super::output::{StringOrVec, UpdateConfig};
use super::tools::{LangContext, require_ruby_bundler, require_tool, ruby_bundle};

fn ruby_update_command(output_dir: &str) -> String {
    let get_frozen = ruby_bundle("config get frozen");
    let unfreeze = ruby_bundle("config set --local frozen false");
    let update = ruby_bundle("update --all");
    let restore = ruby_bundle("config set --local frozen \"$prev_frozen\"");
    format!(
        "cd {output_dir} && prev_frozen=$({get_frozen} 2>/dev/null | awk '/Set for your local app/ {{print $NF}}'); {unfreeze}; {update}; status=$?; if [ -n \"$prev_frozen\" ] && [ \"$prev_frozen\" != \"false\" ]; then {restore}; fi; exit $status"
    )
}

/// The optional `-Dmaven.version.rules=…` argument, emitted only when the scaffolded
/// `versions-rules.xml` is actually on disk.
///
/// `output_dir` arrives already shell-quoted (`'packages/java'`), and that is exactly why this
/// fragment cannot be written as one flat double-quoted string: inside `echo "…"` a single quote
/// is a *literal character*, not a quoting operator, so interpolating there emits
/// `file:///repo/'packages/java'/versions-rules.xml` — a URI with apostrophes in it that names no
/// file, silently disarming the rules maven was told to read. The interpolation therefore closes
/// the double quotes around it (`…/"{output_dir}"/versions-rules.xml`) so the already-quoted word
/// concatenates into the same argument while still being quoted *by the shell*, which keeps a
/// `$(…)` or backtick in a configured output path inert. Verified against a real `sh`, not by
/// reading. ~keep
fn maven_version_rules_flag(output_dir: &str) -> String {
    format!(
        "$([ -f {output_dir}/versions-rules.xml ] && echo \"-Dmaven.version.rules=file://${{PWD}}/\"{output_dir}\"/versions-rules.xml\")"
    )
}

/// Return the default update configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides the package manager selection.
pub fn default_update_config(lang: Language, output_dir: &str, ctx: &LangContext) -> UpdateConfig {
    let output_dir = super::shell::quote_word(output_dir);
    match lang {
        Language::Rust => UpdateConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            update: Some(StringOrVec::Single("cargo update".to_string())),
            upgrade: Some(StringOrVec::Multiple(vec![
                "cargo upgrade --incompatible".to_string(),
                "cargo update".to_string(),
            ])),
        },
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let (update_cmd, upgrade_cmd) = match pm {
                "pip" => (
                    format!("cd {output_dir} && pip install -U -e ."),
                    format!("cd {output_dir} && pip install -U -e ."),
                ),
                "poetry" => (
                    format!("cd {output_dir} && poetry update"),
                    format!("cd {output_dir} && poetry update --with dev"),
                ),
                _ => (
                    format!("cd {output_dir} && uv sync --upgrade --no-install-project --no-install-workspace"),
                    format!(
                        "cd {output_dir} && uv sync --all-packages --all-extras --upgrade --no-install-project --no-install-workspace"
                    ),
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Single(update_cmd)),
                upgrade: Some(StringOrVec::Single(upgrade_cmd)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let (update_cmds, upgrade_cmds) = match pm {
                "npm" => (
                    vec![format!("cd {output_dir} && npm update")],
                    vec![format!(
                        "cd {output_dir} && npm install -g npm-check-updates && ncu -u && npm install"
                    )],
                ),
                "yarn" => (
                    vec![format!("cd {output_dir} && yarn upgrade")],
                    vec![format!("cd {output_dir} && yarn upgrade --latest")],
                ),
                _ => (
                    // `--config.auto-install-peers=false --config.dedupe-peer-dependents=false`:
                    // without these, `pnpm up` promotes optional peer deps of installed packages
                    // (e.g. napi-rs's @emnapi/*, @octokit/core, typanion) into the project's own
                    // `dependencies` and stamps them with the workspace version — corrupting
                    // package.json on every update. ~keep
                    vec![
                        "corepack up".to_string(),
                        "pnpm up -r --config.auto-install-peers=false --config.dedupe-peer-dependents=false"
                            .to_string(),
                    ],
                    vec![
                        "corepack use pnpm@latest".to_string(),
                        "pnpm up --latest -r -w --config.auto-install-peers=false --config.dedupe-peer-dependents=false"
                            .to_string(),
                    ],
                ),
            };
            UpdateConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                update: Some(StringOrVec::Multiple(update_cmds)),
                upgrade: Some(StringOrVec::Multiple(upgrade_cmds)),
            }
        }
        Language::Ruby => {
            let command = ruby_update_command(&output_dir);
            UpdateConfig {
                precondition: Some(require_ruby_bundler()),
                before: None,
                update: Some(StringOrVec::Single(command.clone())),
                upgrade: Some(StringOrVec::Single(command)),
            }
        }
        Language::Php => UpdateConfig {
            precondition: Some(require_tool("composer")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && composer update"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer update --with-all-dependencies"
            ))),
        },
        Language::Go => UpdateConfig {
            precondition: Some(require_tool("go")),
            before: None,
            update: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
            upgrade: Some(StringOrVec::Multiple(vec![
                format!("cd {output_dir} && go get -u ./..."),
                format!("cd {output_dir} && go mod tidy"),
            ])),
        },
        Language::Java => {
            let rules_flag = maven_version_rules_flag(&output_dir);
            UpdateConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                update: Some(StringOrVec::Single(format!(
                    "mvn -f {output_dir}/pom.xml versions:use-latest-releases {rules_flag} --batch-mode --no-transfer-progress"
                ))),
                upgrade: Some(StringOrVec::Single(format!(
                    "mvn -f {output_dir}/pom.xml versions:use-latest-releases -DallowMajorUpdates=true {rules_flag} --batch-mode --no-transfer-progress"
                ))),
            }
        }
        Language::Csharp => UpdateConfig {
            precondition: Some(format!(
                "command -v dotnet >/dev/null 2>&1 && [ -n \"$(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)\" ]"
            )),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "dotnet outdated --upgrade --version-lock major $(find {output_dir} -maxdepth 3 \\( -name '*.sln' -o -name '*.csproj' \\) 2>/dev/null | head -1)"
            ))),
        },
        Language::Elixir => UpdateConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && mix deps.update --all"))),
        },
        Language::R => UpdateConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"remotes::update_packages(ask = FALSE)\""
            ))),
        },
        Language::Ffi => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
        Language::C => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
        },
        Language::Kotlin | Language::KotlinAndroid => UpdateConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "cd {output_dir} && gradle dependencyUpdates"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && gradle dependencyUpdates --refresh-dependencies"
            ))),
        },
        Language::Swift => UpdateConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            update: Some(StringOrVec::Single(format!(
                "swift package update --package-path {output_dir}"
            ))),
            upgrade: Some(StringOrVec::Single(format!(
                "swift package update --package-path {output_dir}"
            ))),
        },
        Language::Dart => UpdateConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && dart pub upgrade"))),
            upgrade: Some(StringOrVec::Single(format!(
                "cd {output_dir} && dart pub upgrade --major-versions"
            ))),
        },
        Language::Zig => UpdateConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && zig build --fetch"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && zig build --fetch"))),
        },
        Language::Gleam => UpdateConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            update: Some(StringOrVec::Single(format!("cd {output_dir} && gleam deps update"))),
            upgrade: Some(StringOrVec::Single(format!("cd {output_dir} && gleam deps update"))),
        },
        Language::Jni => UpdateConfig {
            precondition: None,
            before: None,
            update: None,
            upgrade: None,
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

    fn cfg(lang: Language, dir: &str) -> UpdateConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_update_config(lang, dir, &ctx)
    }

    #[test]
    fn generated_update_quotes_configured_output_directory() {
        let malicious = "packages/python; touch /tmp/alef-update; #";
        let commands = cfg(Language::Python, malicious)
            .update
            .expect("python update command")
            .commands()
            .join(" ");
        assert!(commands.contains(&format!("cd {}", super::super::shell::quote_word(malicious))));
    }

    #[test]
    fn ffi_has_no_update_commands() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.update.is_none());
        assert!(c.upgrade.is_none());
    }

    #[test]
    fn non_ffi_languages_have_update_commands() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.update.is_some(), "{lang} should have a default update command");
            assert!(c.upgrade.is_some(), "{lang} should have a default upgrade command");
        }
    }

    #[test]
    fn ruby_update_uses_the_active_interpreter_and_bundler() {
        let config = cfg(Language::Ruby, "packages/ruby");
        assert_eq!(
            config.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        let update = config.update.expect("ruby update command").commands().join(" ");
        let upgrade = config.upgrade.expect("ruby upgrade command").commands().join(" ");
        for command in [update, upgrade] {
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle config get frozen"),
                "got: {command}"
            );
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle config set --local frozen false"),
                "got: {command}"
            );
            assert!(
                command.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle update --all"),
                "got: {command}"
            );
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn rust_update_uses_cargo() {
        let c = cfg(Language::Rust, "packages/rust");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("cargo update"));
        assert!(upgrade.contains("cargo upgrade --incompatible"));
        assert!(upgrade.contains("cargo update"));
    }

    #[test]
    fn rust_upgrade_is_multi_command() {
        let c = cfg(Language::Rust, "packages/rust");
        let upgrade = c.upgrade.unwrap();
        let cmds = upgrade.commands();
        assert!(cmds.len() >= 2);
    }

    #[test]
    fn python_update_uses_uv_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("uv sync"));
        assert!(upgrade.contains("--all-packages"));
        assert!(update.contains("--no-install-project"));
        assert!(upgrade.contains("--no-install-project"));
        assert!(update.contains("--no-install-workspace"));
        assert!(upgrade.contains("--no-install-workspace"));
    }

    #[test]
    fn python_update_dispatches_on_package_manager() {
        for (pm, expected) in [("pip", "pip install -U"), ("poetry", "poetry update")] {
            let tools = ToolsConfig {
                python_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Python, "packages/python", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn node_update_uses_pnpm_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("pnpm up"));
        assert!(upgrade.contains("pnpm up --latest"));
        // Both flags are required to stop `pnpm up` from promoting optional peer deps
        // into package.json with the workspace version stamped on them.
        for cmds in [&update, &upgrade] {
            assert!(cmds.contains("--config.auto-install-peers=false"));
            assert!(cmds.contains("--config.dedupe-peer-dependents=false"));
        }
    }

    #[test]
    fn node_update_dispatches_on_package_manager() {
        for (pm, expected) in [("npm", "npm update"), ("yarn", "yarn upgrade")] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_update_config(Language::Node, "packages/node", &ctx);
            assert!(
                c.update.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    /// The directory as it is spelled *inside the emitted shell command* — a quoted word, not a
    /// bare path. Expectations derive it from `quote_word` rather than restating one quoting
    /// spelling, so a change to the escaping policy cannot silently repoint a command at a
    /// different directory: the escaping is proved separately, and once, by
    /// `shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile value
    /// through a real shell. ~keep
    fn quoted(dir: &str) -> String {
        super::super::shell::quote_word(dir)
    }

    #[test]
    fn java_update_uses_maven_versions() {
        let c = cfg(Language::Java, "packages/java");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("versions:use-latest-releases"));
        assert!(upgrade.contains("allowMajorUpdates=true"));
        assert!(
            update.contains(&format!("[ -f {}/versions-rules.xml ]", quoted("packages/java"))),
            "java update should make versions-rules.xml optional, got: {update}"
        );
    }

    /// The emitted `-Dmaven.version.rules=` value is built inside an `echo "…"`, where a single
    /// quote is a literal character rather than a quoting operator. Asserting on the command
    /// *text* cannot tell a correct URI from one carrying stray apostrophes, so this runs the
    /// fragment through a real `sh` and checks the path it produces actually names the rules
    /// file on disk. Fails on the flat-double-quoted form this replaced. ~keep
    #[cfg(unix)]
    #[test]
    fn java_version_rules_flag_names_a_file_that_exists() {
        const PACKAGE: &str = "packages/java";
        let root = tempfile::tempdir().expect("tempdir");
        let package = root.path().join(PACKAGE);
        std::fs::create_dir_all(&package).expect("create package dir");
        std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");

        let fragment = maven_version_rules_flag(&quoted(PACKAGE));
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf '%s\\n' {fragment}")])
            .current_dir(root.path())
            .output()
            .expect("shell should start");
        // Joined rather than indexed: the `$(…)` is deliberately unquoted (quoting it would hand
        // maven an empty argument when no rules file exists), so a `$TMPDIR` containing a space
        // splits the substitution into several words. That is a real, pre-existing limitation of
        // the unquoted substitution and not what this test is about — reassembling keeps the test
        // measuring the emitted path rather than the runner's temp directory. ~keep
        let emitted = String::from_utf8_lossy(&output.stdout)
            .lines()
            .collect::<Vec<_>>()
            .join(" ");
        let uri = emitted
            .strip_prefix("-Dmaven.version.rules=file://")
            .unwrap_or_else(|| panic!("expected a file:// rules URI, got `{emitted}`"));
        assert!(
            !uri.contains('\''),
            "the rules URI carries literal apostrophes from the quoted output dir: `{uri}`"
        );
        assert!(
            std::path::Path::new(uri).is_file(),
            "maven is pointed at `{uri}`, which is not the rules file that exists on disk"
        );
    }

    /// A configured output path is consumer input reaching `sh -c`. Inside the `echo "…"` that
    /// builds the rules URI, `;` is inert but `$(…)` is not — so the check that matters is that
    /// no command substitution runs, not that no semicolon survives. ~keep
    #[cfg(unix)]
    #[test]
    fn java_version_rules_flag_does_not_execute_a_configured_output_path() {
        // The hostile directory and its rules file must really exist, or the `[ -f … ]` guard
        // short-circuits and the `echo` this test exists to exercise never runs — the check
        // would then pass while examining nothing. ~keep
        const HOSTILE: &str = "packages/java$(touch executed)";
        let root = tempfile::tempdir().expect("tempdir");
        let package = root.path().join(HOSTILE);
        std::fs::create_dir_all(&package).expect("create hostile package dir");
        std::fs::write(package.join("versions-rules.xml"), "<ruleset/>\n").expect("write rules file");
        let witness = root.path().join("executed");

        let fragment = maven_version_rules_flag(&quoted(HOSTILE));
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf '%s\\n' {fragment}")])
            .current_dir(root.path())
            .output()
            .expect("shell should start");

        assert!(output.status.success(), "the emitted fragment must be valid shell");
        assert!(
            !String::from_utf8_lossy(&output.stdout).trim().is_empty(),
            "the rules flag must have been emitted, or this test proved nothing"
        );
        assert!(
            !witness.exists(),
            "a command substitution in the configured output path was executed by the update command"
        );
    }

    #[test]
    fn csharp_update_resolves_csproj_in_subdir() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        let find = format!("find {}", quoted("packages/csharp"));
        assert!(update.contains(&find), "update should locate csproj, got: {update}");
        assert!(upgrade.contains(&find), "upgrade should locate csproj, got: {upgrade}");
    }

    #[test]
    fn csharp_precondition_requires_project_file() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let pre = c.precondition.unwrap();
        assert!(
            pre.contains(&format!("find {}", quoted("packages/csharp"))),
            "precondition should search for project file, got: {pre}"
        );
        assert!(pre.contains("dotnet"), "precondition should still require dotnet CLI");
    }

    #[test]
    fn output_dir_substituted_in_update_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let update = c.update.unwrap().commands().join(" ");
        assert!(update.contains("my/custom/path"));
    }

    #[test]
    fn r_update_is_non_interactive() {
        let c = cfg(Language::R, "packages/r");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(update.contains("ask = FALSE"), "R update must be non-interactive");
        assert!(upgrade.contains("ask = FALSE"), "R upgrade must be non-interactive");
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        let node_update = node.update.unwrap().commands().join(" ");
        let wasm_update = wasm.update.unwrap().commands().join(" ");
        assert_eq!(node_update, wasm_update);
    }

    #[test]
    fn kotlin_uses_gradle_dependency_updates() {
        let c = cfg(Language::Kotlin, "packages/kotlin");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("gradle dependencyUpdates"),
            "Kotlin update should use gradle dependencyUpdates, got: {update}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_package_update() {
        let c = cfg(Language::Swift, "packages/swift");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("swift package update"),
            "Swift update should use swift package update, got: {update}"
        );
        assert!(
            update.contains(&format!("--package-path {}", quoted("packages/swift"))),
            "Swift update should include package path, got: {update}"
        );
    }

    #[test]
    fn dart_uses_dart_pub_upgrade() {
        let c = cfg(Language::Dart, "packages/dart");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(
            update.contains("dart pub upgrade"),
            "Dart update should use dart pub upgrade, got: {update}"
        );
        assert!(
            upgrade.contains("--major-versions"),
            "Dart upgrade should include --major-versions, got: {upgrade}"
        );
    }

    #[test]
    fn gleam_uses_gleam_deps_update() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let update = c.update.unwrap().commands().join(" ");
        let upgrade = c.upgrade.unwrap().commands().join(" ");
        assert!(
            update.contains("gleam deps update"),
            "Gleam update should use gleam deps update, got: {update}"
        );
        assert!(
            upgrade.contains("gleam deps update"),
            "Gleam upgrade should use gleam deps update, got: {upgrade}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build_fetch() {
        let c = cfg(Language::Zig, "packages/zig");
        let update = c.update.unwrap().commands().join(" ");
        assert!(
            update.contains("zig build --fetch"),
            "Zig update should use zig build --fetch, got: {update}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
