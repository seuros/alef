use crate::cli::pipeline::helpers::{
    check_precondition, precondition_passes, run_before, run_command, run_command_captured,
};
use crate::cli::registry;
use crate::core::config::{BuildCommandConfig, Language, ResolvedCrateConfig};
use crate::core::template_versions as tv;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info, warn};

mod frb_cache;
mod observability;

pub fn build(config: &ResolvedCrateConfig, languages: &[Language], release: bool) -> anyhow::Result<()> {
    let crate_name = &config.name;
    let base_dir = std::env::current_dir()?;

    let mut independent = Vec::new();
    let mut ffi_dependent = Vec::new();
    let mut need_ffi = false;

    let mut rust_langs: Vec<Language> = Vec::new();

    // Reconciled against `dispatched_count` at the end of this function: every
    // announced language must be accounted for as skipped, blocked on an unmet
    // precondition, or dispatched below. ~keep
    let total_announced = languages.len();
    let mut skipped_count = 0_usize;
    // Kept apart from `failures` all the way to the exit code: these languages were never
    // compiled, so folding them in would assert something about generated code that this run
    // never tested. ~keep
    let mut unmet: Vec<String> = Vec::new();

    for &lang in languages {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        match backend_readiness(lang, &build_cmd_cfg) {
            BackendReadiness::Ready => {}
            BackendReadiness::ToolchainMissing => {
                observability::skipped(lang, "required tool is not on PATH");
                skipped_count += 1;
                continue;
            }
            BackendReadiness::DependenciesUnfetched { check, remediation } => {
                observability::unmet_precondition(
                    lang,
                    &format!("dependency precondition failed ({check})"),
                    &remediation,
                );
                unmet.push(format!("{lang} (run `{remediation}`)"));
                continue;
            }
        }
        if lang == Language::Rust {
            rust_langs.push(lang);
            continue;
        }
        // `try_get_backend`, not `get_backend`: the latter panics for docs-only/
        // consumer-only targets (Rust, C). Rust is already routed above; a
        // language like C configured in `[workspace] languages` must be skipped
        // gracefully here rather than crashing the whole build. ~keep
        let Some(backend) = registry::try_get_backend(lang) else {
            info!("No binding backend for {lang}, skipping");
            observability::skipped(lang, "no binding backend");
            skipped_count += 1;
            continue;
        };
        if let Some(bc) = backend.build_config_with_config(config) {
            if bc.depends_on_ffi() {
                ffi_dependent.push((lang, bc));
                need_ffi = true;
            } else {
                independent.push((lang, bc));
            }
        } else {
            info!("No build config for {lang}, skipping");
            observability::skipped(lang, "no build config");
            skipped_count += 1;
        }
    }
    let dispatched_count = rust_langs.len() + independent.len() + ffi_dependent.len();

    // Every stage below records its own per-language failures into `failures`
    // instead of bailing out with `?`. ~keep A missing/misconfigured recipe or a
    // real compile failure in one backend must not erase build signal for every
    // other, unrelated backend — see the "false" command-substitution incident:
    // one unconfigured backend used to fail-fast the whole build for languages
    // that had nothing to do with it. The run still fails overall, but only
    // after every backend got a chance to run and report its own outcome.
    let mut failures: Vec<String> = Vec::new();

    for &lang in &rust_langs {
        let result = observability::observe(lang, || {
            let build_cmd_cfg = config.build_command_config_for_language(lang);
            run_before(lang, build_cmd_cfg.before.as_ref())?;
            let cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = cmds {
                for cmd in cmd_list.commands() {
                    info!("Building {lang}: {cmd}");
                    run_command(cmd).with_context(|| format!("failed to build {lang}"))?;
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            failures.push(format!("{lang}: {err:#}"));
        }
    }

    if need_ffi
        && !independent
            .iter()
            .any(|(_, bc)| bc.tool == "cargo" && bc.crate_suffix == "-ffi")
    {
        let ffi_crate = output_path_for(Language::Ffi, config)
            .map(resolve_crate_dir)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| Box::leak(format!("{crate_name}-ffi").into_boxed_str()));
        info!("Building FFI crate: {ffi_crate}");
        let mut cmd = format!("cargo build -p {ffi_crate}");
        if release {
            cmd.push_str(" --release");
        }
        let result = observability::observe(Language::Ffi, || run_command(&cmd).context("failed to build FFI crate"));
        if let Err(err) = result {
            failures.push(format!("{}: {err:#}", Language::Ffi));
        }
    }

    // Before-hooks run sequentially (they may touch shared resources like a
    // lockfile) but a failing hook only takes its own language out of the
    // parallel dispatch below — it does not stop the remaining before-hooks
    // or the rest of the build. `before` is rare in practice, so we only pay
    // for its own started/completed observability pair when one is actually
    // configured; the language's real build attempt is observed separately
    // once it reaches the parallel dispatch. ~keep
    let mut independent_ready = Vec::with_capacity(independent.len());
    for (lang, bc) in independent {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        let before = build_cmd_cfg.before;
        let before_result = if before.is_some() {
            observability::observe(lang, || run_before(lang, before.as_ref()))
        } else {
            Ok(())
        };
        match before_result {
            Ok(()) => independent_ready.push((lang, bc)),
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }
    let independent = independent_ready;

    let build_results: Vec<anyhow::Result<(String, String)>> = independent
        .par_iter()
        .map(|(lang, bc)| {
            observability::observe(*lang, || {
                let build_cmd_cfg = config.build_command_config_for_language(*lang);
                let override_cmds = if release {
                    build_cmd_cfg.build_release.as_ref()
                } else {
                    build_cmd_cfg.build.as_ref()
                };
                if let Some(cmd_list) = override_cmds
                    && config.build_commands.contains_key(&lang.to_string())
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
                info!("Building {lang} ({})...", bc.tool);
                let build_cmd = build_command_for(*lang, bc, config, release);
                run_command_captured(&build_cmd)
                    .with_context(|| format!("failed to build language bindings for {lang}"))
            })
        })
        .collect();

    for ((lang, bc), result) in independent.iter().zip(build_results) {
        match result {
            Ok((stdout, stderr)) => {
                if !stdout.is_empty() {
                    info!("[{lang} build] {stdout}");
                }
                if !stderr.is_empty() {
                    debug!("[{lang} build] {stderr}");
                }
                if let Err(err) = run_post_build(*lang, bc, config, &base_dir) {
                    failures.push(format!("{lang}: post-build failed: {err:#}"));
                }
            }
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }

    // ffi_dependent backends are attempted unconditionally, even if the FFI
    // crate build above failed: attempting them still yields a true,
    // per-backend outcome (they'll fail for a real reason if the FFI crate is
    // genuinely broken), which is strictly more informative than skipping
    // them and losing their signal entirely. ~keep
    let mut ffi_dependent_ready = Vec::with_capacity(ffi_dependent.len());
    for (lang, bc) in ffi_dependent {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        let before = build_cmd_cfg.before;
        let before_result = if before.is_some() {
            observability::observe(lang, || run_before(lang, before.as_ref()))
        } else {
            Ok(())
        };
        match before_result {
            Ok(()) => ffi_dependent_ready.push((lang, bc)),
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }
    let ffi_dependent = ffi_dependent_ready;

    let build_results: Vec<anyhow::Result<(String, String)>> = ffi_dependent
        .par_iter()
        .map(|(lang, bc)| {
            observability::observe(*lang, || {
                let build_cmd_cfg = config.build_command_config_for_language(*lang);
                let override_cmds = if release {
                    build_cmd_cfg.build_release.as_ref()
                } else {
                    build_cmd_cfg.build.as_ref()
                };
                if let Some(cmd_list) = override_cmds
                    && config.build_commands.contains_key(&lang.to_string())
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
                info!("Building {lang} ({})...", bc.tool);
                let build_cmd = build_command_for(*lang, bc, config, release);
                run_command_captured(&build_cmd)
                    .with_context(|| format!("failed to build language bindings for {lang}"))
            })
        })
        .collect();

    for ((lang, bc), result) in ffi_dependent.iter().zip(build_results) {
        match result {
            Ok((stdout, stderr)) => {
                if !stdout.is_empty() {
                    info!("[{lang} build] {stdout}");
                }
                if !stderr.is_empty() {
                    debug!("[{lang} build] {stderr}");
                }
                if let Err(err) = run_post_build(*lang, bc, config, &base_dir) {
                    failures.push(format!("{lang}: post-build failed: {err:#}"));
                }
            }
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }

    // Reconciliation, not just a status line: `dispatched_count` is exactly
    // `rust_langs.len() + independent.len() + ffi_dependent.len()` captured
    // right after classification, before any before-hook filtering or `?`
    // could shrink it — so if this doesn't equal `total_announced -
    // skipped_count`, some announced language fell through the classification
    // loop without either a skip or a dispatch, which is a bug in the loop
    // above, not downstream. Every dispatched language is guaranteed a
    // terminal observability event by construction: rust_langs, independent,
    // and ffi_dependent are each fully drained by an unconditional loop or
    // `.par_iter().collect()` (no `?` early-return anywhere in between), so
    // silently losing one after this point cannot happen without also
    // failing this assertion. ~keep
    debug_assert_eq!(
        skipped_count + unmet.len() + dispatched_count,
        total_announced,
        "every announced language must be skipped, blocked on a precondition, or dispatched"
    );
    // Not `dispatched_count - failures.len()`: `failures` can also include the
    // implicit FFI-crate auto-build (see `need_ffi` above), which fires as a
    // side effect for backends that depend on it and isn't itself one of the
    // `dispatched_count` entries when "ffi" wasn't explicitly requested — so
    // that subtraction could under-report. Report what's exact instead. ~keep
    info!(
        "Backend build summary: {total_announced} announced, {skipped_count} skipped, \
         {} blocked on unmet preconditions, {dispatched_count} dispatched, {} language-level failure(s)",
        unmet.len(),
        failures.len()
    );

    build_outcome(&failures, &unmet)
}

/// Turn the two per-language buckets into this command's exit status.
///
/// Both buckets are fatal, and they are reported in separate sentences that never merge counts.
/// The reasoning behind each half:
///
/// - Unmet preconditions are fatal because the alternative is worse. Nothing was built for those
///   languages, so exiting 0 would let anything reading this exit code — CI, a release script,
///   the snippet validation that links these very artifacts — proceed as though the artifacts
///   exist. The remediation is one command in this same checkout, so failing costs the developer
///   a single retry and buys everyone downstream a truthful signal.
/// - They are nonetheless not `failures`: the count, the wording, and the per-language outcome all
///   say "not built" rather than "built and broken", which is the distinction that makes
///   "run `mix deps.get`" actionable where a bare failure was not. ~keep
///
/// A *missing toolchain* is deliberately absent from both buckets and stays a non-fatal skip: it
/// is a statement about the machine, not about this checkout, and a developer without `gradle`
/// installed must still be able to build the languages they do have. ~keep
fn build_outcome(failures: &[String], unmet: &[String]) -> anyhow::Result<()> {
    let mut parts = Vec::new();
    if !failures.is_empty() {
        parts.push(format!(
            "backend build failed for {} language(s): {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    if !unmet.is_empty() {
        parts.push(format!(
            "{} language(s) were not built because their preconditions are unmet (no build was attempted, so this \
             is not a compile failure): {}",
            unmet.len(),
            unmet.join("; ")
        ));
    }
    if parts.is_empty() {
        return Ok(());
    }
    anyhow::bail!("{}", parts.join(" | "));
}

/// Whether a backend can be built here at all, and if not, which kind of "not" it is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendReadiness {
    Ready,
    /// The tool this backend builds with is not installed on this machine. Not the checkout's
    /// fault and not fixable from inside it — skipped, non-fatal. ~keep
    ToolchainMissing,
    /// The tool is here and the checkout is not prepared for it: dependencies were never fetched,
    /// or the interpreter environment the build installs into does not exist. Fixable by
    /// `remediation`, and fatal so that nothing downstream mistakes the missing artifact for a
    /// built one. ~keep
    DependenciesUnfetched {
        check: String,
        remediation: String,
    },
}

/// Classify a backend before dispatching it.
///
/// The tool check runs first: a dependency check phrased against a missing tool's project layout
/// would report the wrong cause. ~keep
fn backend_readiness(lang: Language, build_cmd_cfg: &BuildCommandConfig) -> BackendReadiness {
    if !check_precondition(lang, build_cmd_cfg.precondition.as_deref()) {
        return BackendReadiness::ToolchainMissing;
    }
    let Some(check) = build_cmd_cfg.dependency_precondition.as_deref() else {
        return BackendReadiness::Ready;
    };
    if precondition_passes(&lang.to_string(), check) {
        return BackendReadiness::Ready;
    }
    // Config validation rejects a `dependency_precondition` without a `dependency_remediation`, so
    // this fallback is unreachable through a loaded config — it exists so a future built-in that
    // forgets the pair degrades into a vague message rather than a panic. ~keep
    let remediation = build_cmd_cfg
        .dependency_remediation
        .clone()
        .unwrap_or_else(|| format!("(no `dependency_remediation` declared for {lang})"));
    BackendReadiness::DependenciesUnfetched {
        check: check.to_string(),
        remediation,
    }
}

/// Resolve the crate directory from the output config path.
/// Output paths like `crates/sample-markdown-node/src/` → `crates/sample-markdown-node`.
fn resolve_crate_dir(output_path: &Path) -> &Path {
    if output_path.file_name().is_some_and(|n| n == "src") {
        output_path.parent().unwrap_or(output_path)
    } else {
        output_path
    }
}

/// Get the output path for a language from config.
fn output_path_for(lang: Language, config: &ResolvedCrateConfig) -> Option<&Path> {
    match lang {
        Language::Python => config.explicit_output.python.as_deref(),
        Language::Node => config.explicit_output.node.as_deref(),
        Language::Ruby => config.explicit_output.ruby.as_deref(),
        Language::Php => config.explicit_output.php.as_deref(),
        Language::Ffi => config.explicit_output.ffi.as_deref(),
        Language::Go => config.explicit_output.go.as_deref(),
        Language::Java => config.explicit_output.java.as_deref(),
        Language::Csharp => config.explicit_output.csharp.as_deref(),
        Language::Kotlin => config.explicit_output.kotlin.as_deref(),
        Language::KotlinAndroid => config.explicit_output.kotlin_android.as_deref(),
        Language::Wasm => config.explicit_output.wasm.as_deref(),
        Language::Elixir => config.explicit_output.elixir.as_deref(),
        Language::R => config.explicit_output.r.as_deref(),
        Language::Rust | Language::C | Language::Jni => None,
        Language::Swift | Language::Dart | Language::Gleam | Language::Zig => None,
    }
}

/// Generate the shell command to build a specific language.
fn build_command_for(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    release: bool,
) -> String {
    let release_flag = if release { " --release" } else { "" };

    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .and_then(|p| p.to_str())
        .unwrap_or("");

    match bc.tool {
        "maturin" => {
            format!("maturin develop --manifest-path {crate_dir}/Cargo.toml{release_flag}")
        }
        "napi" => {
            format!(
                "npx --yes -p @napi-rs/cli@{} napi build --platform --manifest-path {}/Cargo.toml -o {}{}",
                tv::npm::NAPI_RS_CLI_CRATE,
                crate_dir,
                crate_dir,
                release_flag
            )
        }
        "wasm-pack" => {
            let profile = if release { "--release" } else { "--dev" };
            // `crate_dir` is empty whenever `[crates.output] wasm` is not set explicitly
            // (the common case) — fall back to the same default formula `package_dir`
            // already uses for scaffolding, matching the `gradle`/`swift`/`zig` arms below.
            let wasm_crate_dir = if crate_dir.is_empty() {
                config.package_dir(lang)
            } else {
                crate_dir.to_string()
            };
            // Build every configured target into `pkg/<target>`, byte-for-byte matching the
            // `build:wasm:<target>` scripts `scaffold::languages::wasm` writes into the crate's
            // own `package.json` (and which `publish::package::wasm` re-runs via `npm run
            // build:all`). Nothing consumes a bare `pkg/`: the scaffolded manifest resolves
            // `main`/`types` to `pkg/<nodejs>/…` and `module` to `pkg/<web>/…`, and `alef e2e`
            // depends on `pkg/nodejs` specifically (`ResolvedCrateConfig::wasm_crate_path`).
            // The previous `--target web` with wasm-pack's default out-dir wrote a bare `pkg/`
            // that no manifest, publish step or e2e suite reads, while never producing the
            // `pkg/nodejs` e2e needs — which is why consumers hand-rolled an extra build step.
            // Driving off `wasm_targets()` rather than a hardcoded pair also honours a crate
            // that narrows `[crates.wasm] targets` to keep its published package small. `cd`
            // into the crate dir instead of passing a positional `<path>`, since wasm-pack
            // resolves `--out-dir` against the process cwd, not against that argument. ~keep
            let targets = config.wasm_targets();
            if targets.is_empty() {
                // `scaffold::languages::wasm` rejects an empty target list, but that runs only
                // when scaffolding; reaching here with `targets = []` would otherwise emit a
                // dangling `cd <dir> && ` that the shell reports as a syntax error naming
                // neither alef nor the setting at fault. ~keep
                return format!(
                    "echo 'alef: [crates.wasm] targets is empty for {lang}; list at least one \
                     wasm-pack target (web, bundler, nodejs, deno)' >&2 && false"
                );
            }
            let builds = targets
                .iter()
                .map(|target| format!("wasm-pack build {profile} --target {target} --out-dir pkg/{target}"))
                .collect::<Vec<_>>()
                .join(" && ");
            format!("cd {wasm_crate_dir} && {builds}")
        }
        "cargo" => {
            if crate_dir.is_empty() && !bc.crate_suffix.is_empty() {
                return format!("cargo build -p {}{}{}", config.name, bc.crate_suffix, release_flag);
            }
            let native_dir = Path::new(crate_dir).join("native");
            let native_manifest = native_dir.join("Cargo.toml");
            if native_manifest.exists() {
                let dir = native_dir.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else if let Some(standalone) = {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut found: Option<std::path::PathBuf> = None;
                for _ in 0..3 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[workspace]")
                        {
                            found = Some(p.clone());
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                found
            } {
                let dir = standalone.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut package_name: Option<String> = None;
                let mut package_dir: Option<std::path::PathBuf> = None;
                for _ in 0..4 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[package]")
                        {
                            for line in contents.lines() {
                                let trimmed = line.trim();
                                if let Some(rest) = trimmed.strip_prefix("name") {
                                    let rest = rest.trim_start_matches([' ', '=']).trim();
                                    let rest = rest.trim_matches(['"', '\'']);
                                    if !rest.is_empty() {
                                        package_name = Some(rest.to_string());
                                        package_dir = Some(p.clone());
                                        break;
                                    }
                                }
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                let is_excluded_from_workspace = if let Some(pdir) = &package_dir {
                    let mut q = pdir.clone();
                    let mut excluded = false;
                    while q.pop() {
                        let manifest = q.join("Cargo.toml");
                        if manifest.exists()
                            && let Ok(contents) = std::fs::read_to_string(&manifest)
                            && contents.contains("[workspace]")
                        {
                            let rel = pdir.strip_prefix(&q).unwrap_or(pdir).to_string_lossy().into_owned();
                            let rel_norm = rel.replace('\\', "/");
                            excluded = contents.lines().map(|l| l.trim()).any(|l| {
                                l.contains(&format!("\"{rel_norm}\"")) && {
                                    let needle = format!("\"{rel_norm}\"");
                                    let exclude_section = contents.split("exclude").nth(1).unwrap_or("");
                                    let members_section = contents.split("members").nth(1).unwrap_or("");
                                    let in_exclude = exclude_section.contains(&needle);
                                    let in_members =
                                        members_section.contains(&needle) && !exclude_section.contains(&needle);
                                    in_exclude && !in_members
                                }
                            });
                            break;
                        }
                    }
                    excluded
                } else {
                    false
                };
                if is_excluded_from_workspace {
                    if let Some(pdir) = package_dir {
                        let dir = pdir.display();
                        format!("cd {dir} && cargo build{release_flag}")
                    } else {
                        format!("cd {crate_dir} && cargo build{release_flag}")
                    }
                } else {
                    let crate_name = package_name.unwrap_or_else(|| {
                        Path::new(crate_dir)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(crate_dir)
                            .to_string()
                    });
                    format!("cargo build -p {crate_name}{release_flag}")
                }
            }
        }
        "mix" => {
            let dir = config
                .explicit_output
                .elixir
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/elixir");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("mix.exs").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mix compile")
        }
        "mvn" => {
            let dir = config
                .explicit_output
                .java
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/java");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("pom.xml").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mvn package -DskipTests --batch-mode --no-transfer-progress")
        }
        "dotnet" => {
            let dir = config
                .explicit_output
                .csharp
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/csharp");
            let scan_for_csproj = |start: &std::path::Path| -> Option<String> {
                if start
                    .read_dir()
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
                    })
                    .unwrap_or(false)
                {
                    return Some(start.to_string_lossy().to_string());
                }
                start.read_dir().ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .find(|e| {
                            e.path().is_dir()
                                && e.path().read_dir().ok().is_some_and(|sub| {
                                    sub.filter_map(|s| s.ok())
                                        .any(|s| s.path().extension().is_some_and(|ext| ext == "csproj"))
                                })
                        })
                        .map(|e| e.path().to_string_lossy().to_string())
                })
            };
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                let mut found = scan_for_csproj(&p);
                while found.is_none() && p.pop() {
                    found = scan_for_csproj(&p);
                }
                found.unwrap_or_else(|| dir.to_string())
            };
            let dotnet_config = if release { "Release" } else { "Debug" };
            format!("cd {build_dir} && dotnet build --configuration {dotnet_config} --verbosity quiet")
        }
        "go" => {
            let dir = config
                .explicit_output
                .go
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/go");
            format!("cd {dir} && go build ./...")
        }
        "gradle" => {
            let release_property = if release { " -Prelease" } else { "" };
            let source_dir = if crate_dir.is_empty() {
                config.package_dir(lang)
            } else {
                crate_dir.to_string()
            };
            // `crate_dir`/`package_dir` is a *source* output path (e.g. a Kotlin
            // package-namespace directory `.kt` files get written into) — it need not
            // be the gradle project root gradle itself will accept. `gradle <task>`
            // walks up from the invoked directory looking only for a settings file and
            // treats that file's directory as the build root; a directory that is
            // merely nested under that root but not itself a registered project is
            // rejected ("Project directory '...' is not part of the build defined by
            // settings file '...'") — that rejection, observed against a real consumer
            // whose `[crates.output] kotlin_android` pointed at a deep namespace source
            // dir, is this arm's bug. So we search for `settings.gradle.kts`/
            // `settings.gradle` first, walking the full ancestor chain, mirroring
            // gradle's own root-detection precedence: a settings file is authoritative
            // over a build file, because a `build.gradle(.kts)` alone only marks *a*
            // project (possibly a subproject with no invocable root of its own). Only
            // if no settings file is found anywhere upward do we fall back to
            // `build.gradle.kts`/`build.gradle` as a lower-confidence signal for a
            // settings-less single-module project; finding neither falls back to
            // `source_dir` unchanged, matching every other backend arm in this
            // function (mix/mvn/dotnet) rather than walking to the filesystem root or
            // panicking.
            //
            // Deliberately NOT sharing a helper with the "dotnet" arm's `scan_for_csproj`
            // above: that is the only other precedent for this shape, so per this repo's
            // "extract on the third repetition" convention the two stay separate — kept
            // structurally parallel (closure + walk-up loop) instead. ~keep
            let find_marker_dir = |start: &str, markers: &[&str]| -> Option<String> {
                let mut p = std::path::PathBuf::from(start);
                loop {
                    if markers.iter().any(|marker| p.join(marker).exists()) {
                        return Some(p.to_string_lossy().into_owned());
                    }
                    if !p.pop() {
                        return None;
                    }
                }
            };
            let build_dir = find_marker_dir(&source_dir, &["settings.gradle.kts", "settings.gradle"])
                .or_else(|| find_marker_dir(&source_dir, &["build.gradle.kts", "build.gradle"]))
                .unwrap_or(source_dir);
            format!("cd {build_dir} && gradle build{release_property}")
        }
        "swift" => {
            let package_dir = config.package_dir(lang);
            let configuration = if release { " --configuration release" } else { "" };
            format!("swift build --package-path {package_dir}{configuration}")
        }
        "zig" => {
            let package_dir = config.package_dir(lang);
            let release_flag = if release { " --release=fast" } else { "" };
            format!("cd {package_dir} && zig build{release_flag}")
        }
        "gleam" => {
            let package_dir = config.package_dir(lang);
            format!("cd {package_dir} && gleam build")
        }
        // Every backend registers a `tool` here from the fixed set matched above (or defines its
        // own `[build] build`/`build_release` commands, handled by the caller before this
        // function runs). Reaching this arm means a backend's `BuildConfig.tool` genuinely has no
        // known default — fail loudly and name the missing tool rather than silently substituting
        // a bare `false`, which previously reported as an inscrutable "Command failed: false". ~keep
        _ => format!(
            "echo 'alef: no default build command for tool \"{}\" (language: {lang}); add [crates.build_commands.{lang}] build = [...] to alef.toml' >&2 && false",
            bc.tool
        ),
    }
}

/// Run post-build processing steps (e.g., patching .d.ts files).
pub fn run_post_build(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> anyhow::Result<()> {
    use crate::core::backend::PostBuildStep;

    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .unwrap_or(Path::new(""));

    for step in &bc.post_build {
        match step {
            PostBuildStep::PatchFile { path, find, replace } => {
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-build patch target {}", file_path.display()))?;
                    if content.contains(replace) {
                        debug!("Post-build patch target already patched: {}", file_path.display());
                        continue;
                    }
                    let patched = content.replace(find, replace);
                    if patched != content {
                        std::fs::write(&file_path, &patched)
                            .with_context(|| format!("failed to write patched file {}", file_path.display()))?;
                        info!("Patched {}: replaced '{}' → '{}'", file_path.display(), find, replace);
                    }
                } else {
                    debug!("Post-build patch target not found: {}", file_path.display());
                }
            }
            PostBuildStep::RunCommand { cmd, args } => {
                let work_dir = base_dir.join(crate_dir);
                run_run_command(cmd, args, &work_dir, &config.name)
                    .with_context(|| format!("post-build RunCommand '{cmd}' failed"))?;
            }
            PostBuildStep::PostProcessFile { path, processor } => {
                use crate::core::backend::PostProcessor;
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-process target {}", file_path.display()))?;
                    let processed = match processor {
                        PostProcessor::FrbDartSealedVariants => {
                            crate::backends::dart::rewrite_frb_sealed_variants(&content, &config.dart_pubspec_name())
                        }
                        PostProcessor::FrbDartExcludeFunctions(excluded) => {
                            let exclude_set: std::collections::HashSet<&str> =
                                excluded.iter().map(|s| s.as_str()).collect();
                            crate::backends::dart::filter_excluded_functions(&content, &exclude_set)
                        }
                        PostProcessor::FrbDartOptionalFieldsWithDefaults => {
                            crate::backends::dart::make_struct_fields_with_defaults_optional(&content)
                        }
                        PostProcessor::FrbDartFixHandlerExecutorCalls => {
                            crate::backends::dart::fix_handler_executor_calls(&content)
                        }
                        PostProcessor::FrbDartInjectTextMethods(type_names) => {
                            crate::backends::dart::inject_display_as_text_methods(&content, type_names)
                        }
                        PostProcessor::DartStripTrailingWhitespace => {
                            crate::backends::dart::strip_trailing_whitespace(&content)
                        }
                    };
                    if processed != content {
                        std::fs::write(&file_path, &processed)
                            .with_context(|| format!("failed to write post-processed file {}", file_path.display()))?;
                        info!("PostProcessed {}: {:?}", file_path.display(), processor);
                    } else {
                        debug!(
                            "PostProcessFile {}: no changes (already rewritten or absent variants)",
                            file_path.display()
                        );
                    }
                } else {
                    debug!("PostProcessFile target not found: {}", file_path.display());
                }
            }
            PostBuildStep::CarryFrbCfgGates {
                source_path,
                target_path,
            } => {
                let source_file = base_dir.join(crate_dir).join(source_path);
                let target_file = base_dir.join(crate_dir).join(target_path);
                if source_file.exists() && target_file.exists() {
                    let source_content = std::fs::read_to_string(&source_file)
                        .with_context(|| format!("failed to read cfg-gate source {}", source_file.display()))?;
                    let target_content = std::fs::read_to_string(&target_file)
                        .with_context(|| format!("failed to read cfg-gate target {}", target_file.display()))?;
                    let rewritten = crate::backends::dart::carry_lib_rs_cfg_gates_into_frb_generated(
                        &source_content,
                        &target_content,
                    );
                    if rewritten != target_content {
                        std::fs::write(&target_file, &rewritten)
                            .with_context(|| format!("failed to write cfg-gated file {}", target_file.display()))?;
                        info!(
                            "Carried #[cfg] gates from {} into {}",
                            source_file.display(),
                            target_file.display()
                        );
                    } else {
                        debug!("CarryFrbCfgGates {}: no changes needed", target_file.display());
                    }
                } else {
                    debug!(
                        "CarryFrbCfgGates source or target not found: {} / {}",
                        source_file.display(),
                        target_file.display()
                    );
                }
            }
            PostBuildStep::StageDartNatives { lib_stem } => {
                let package_root = base_dir.join("packages/dart");
                let status =
                    crate::publish::dart_native::stage_dart_native_libraries(base_dir, &package_root, lib_stem)
                        .with_context(|| format!("failed to stage Dart native libraries for stem '{lib_stem}'"))?;
                match status {
                    crate::publish::dart_native::NativeLibraryStageStatus::Staged => {
                        info!("Staged native libraries for Dart package from build output (stem: '{lib_stem}')");
                    }
                    crate::publish::dart_native::NativeLibraryStageStatus::Missing => {
                        debug!("No Dart native libraries available to stage for development stem '{lib_stem}'");
                    }
                }
            }
            PostBuildStep::MaterializeSwiftBridge {
                binding_crate_name,
                package_root,
            } => {
                let package_root = base_dir.join(package_root);
                let materialized = crate::backends::swift::gen_bindings::bridge_artifacts::emit_swift_bridge_files(
                    "",
                    binding_crate_name,
                    &package_root,
                )
                .with_context(|| format!("failed to re-materialize swift-bridge files for '{binding_crate_name}'"))?;
                if let Some(files) = materialized {
                    for f in files {
                        if let Some(parent) = f.path.parent() {
                            std::fs::create_dir_all(parent)
                                .with_context(|| format!("failed to create directory {}", parent.display()))?;
                        }
                        std::fs::write(&f.path, &f.content)
                            .with_context(|| format!("failed to write {}", f.path.display()))?;
                    }
                }
                info!("Re-materialized swift-bridge files for '{binding_crate_name}' from fresh build output");
            }
            PostBuildStep::RewriteWasmPackageName {
                package_json_path,
                package_name,
            } => {
                // Unlike every other step above, `package_json_path` is already relative to
                // `base_dir` (not `crate_dir`): the wasm crate directory itself may be
                // `config.package_dir(Language::Wasm)`'s default-formula fallback rather than
                // `crate_dir` (empty whenever `[crates.output] wasm` isn't set explicitly — see
                // `build_command_for`'s "wasm-pack" arm), so the caller resolves the full path
                // up front instead of relying on this function's `crate_dir`. ~keep
                let file_path = base_dir.join(package_json_path);
                if file_path.exists() {
                    rewrite_wasm_package_json_name(&file_path, package_name)
                        .with_context(|| format!("failed to rewrite wasm package name in {}", file_path.display()))?;
                } else {
                    debug!(
                        "wasm-pack package.json not found for name rewrite: {}",
                        file_path.display()
                    );
                }
            }
        }
    }

    Ok(())
}

/// Rewrite the `"name"` field of a wasm-pack-generated `package.json` in place.
///
/// wasm-pack always writes `"name"` as a plain top-level string field, but the value itself
/// (derived from the wasm crate's `Cargo.toml`) is not known until build time, so this can't
/// be a static [`PostBuildStep::PatchFile`] find/replace — the "find" half would have to be
/// discovered from the very file being patched. A regex on the `"name": "..."` field is a
/// minimal, order- and formatting-preserving edit; a full `serde_json` parse+reserialize would
/// risk reordering keys or changing indentation on every build. ~keep
fn rewrite_wasm_package_json_name(path: &Path, new_name: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let name_field = regex::Regex::new(r#""name"\s*:\s*"[^"]*""#).expect("static regex is valid");
    let escaped_name = new_name.replace('\\', "\\\\").replace('"', "\\\"");
    let replacement = format!("\"name\": \"{escaped_name}\"");
    let rewritten = name_field.replacen(&content, 1, replacement.as_str());
    if rewritten != content {
        std::fs::write(path, rewritten.as_ref()).with_context(|| format!("failed to write {}", path.display()))?;
        info!("Rewrote wasm package name in {} to '{new_name}'", path.display());
    } else {
        debug!(
            "wasm package.json {}: name already '{new_name}' or no name field found",
            path.display()
        );
    }
    Ok(())
}

/// Hard upper bound on how long a post-build `RunCommand` may run before alef
/// considers it hung and kills it. Cold-cache `cargo build --release` for the
/// swift binding crate against a polyglot project's full feature set
/// legitimately takes 10-20 minutes; FRB codegen on a warm cache finishes in
/// under a minute. 30 minutes accommodates both without false-positiving
/// slow first-runs on cold CI caches.
const RUN_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

/// Interval between `try_wait()` polls. Short enough to react promptly to a
/// finished child, long enough not to burn CPU in a tight loop.
const RUN_COMMAND_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Execute a `RunCommand` post-build step.
///
/// Spawns `cmd` with `args` in `base_dir`, streaming stdout/stderr through
/// alef's own stdio so interactive subprocess progress is visible. Enforces a
/// `RUN_COMMAND_TIMEOUT` ceiling; on timeout the child is SIGKILL'd and the
/// call returns an error. Returns an error on non-zero exit status.
///
/// Escape hatch: the env var `ALEF_SKIP_COMMANDS` accepts a comma-separated
/// list of `cmd` names to skip without running. Useful in environments where
/// a post-build tool is unavailable, hangs (e.g. `flutter_rust_bridge_codegen`
/// installing Flutter via FVM under CI), or simply isn't desired this run.
/// Each skipped command logs a `warn!` so the omission is visible.
fn run_run_command(cmd: &str, args: &[&str], base_dir: &Path, cache_scope: &str) -> anyhow::Result<()> {
    if let Ok(skip_list) = std::env::var("ALEF_SKIP_COMMANDS")
        && skip_list.split(',').any(|s| s.trim() == cmd)
    {
        warn!("[{cmd}] skipped via ALEF_SKIP_COMMANDS env var");
        return Ok(());
    }
    let mut command = std::process::Command::new(cmd);
    command
        .args(args)
        .current_dir(base_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    frb_cache::configure(&mut command, cmd, cache_scope)?;

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "[{cmd}] not on PATH — skipping post-build step. Install '{cmd}' to regenerate at build time; falling back to committed generated files."
            );
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err).context(format!("failed to spawn '{cmd}'"))),
    };

    let started_at = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started_at.elapsed() > RUN_COMMAND_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("'{cmd}' exceeded {}s timeout; killed", RUN_COMMAND_TIMEOUT.as_secs());
                }
                std::thread::sleep(RUN_COMMAND_POLL_INTERVAL);
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context(format!("failed to wait for '{cmd}'")));
            }
        }
    };

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        anyhow::bail!("'{cmd}' exited with status {code}");
    }

    Ok(())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    /// Real shell commands, not a stubbed runner: `true`/`false` exercise the same spawn path a
    /// `command -v` or `[ -d deps ]` precondition takes, so what the test proves is what runs. ~keep
    fn cfg(precondition: &str, dependency_precondition: Option<&str>) -> BuildCommandConfig {
        BuildCommandConfig {
            precondition: Some(precondition.to_string()),
            dependency_precondition: dependency_precondition.map(str::to_string),
            dependency_remediation: dependency_precondition.map(|_| "cd packages/elixir && mix deps.get".to_string()),
            before: None,
            build: None,
            build_release: None,
        }
    }

    /// The defect this change exists to close: the tool is installed, the checkout is not
    /// prepared, and that must not be reported the same way as generated code failing to
    /// compile. ~keep
    #[test]
    fn should_report_unfetched_dependencies_when_the_tool_is_present_but_deps_are_not() {
        let readiness = backend_readiness(Language::Elixir, &cfg("true", Some("false")));

        assert_eq!(
            readiness,
            BackendReadiness::DependenciesUnfetched {
                check: "false".to_string(),
                remediation: "cd packages/elixir && mix deps.get".to_string(),
            }
        );
    }

    /// The mandatory control. A backend whose preconditions all pass is dispatched, so a build
    /// that then fails is still a `failure` — the fix must not be able to pass by reclassifying
    /// everything it touches. ~keep
    #[test]
    fn should_stay_ready_when_every_precondition_passes_so_a_real_compile_failure_still_fails() {
        assert_eq!(
            backend_readiness(Language::Elixir, &cfg("true", Some("true"))),
            BackendReadiness::Ready
        );
        assert_eq!(
            backend_readiness(Language::Go, &cfg("true", None)),
            BackendReadiness::Ready
        );

        let error = build_outcome(&["go: undefined: Foo".to_string()], &[]).expect_err("compile failure is fatal");
        let message = error.to_string();
        assert!(message.contains("backend build failed for 1 language(s)"), "{message}");
        assert!(!message.contains("preconditions are unmet"), "{message}");
    }

    #[test]
    fn should_report_a_missing_tool_as_a_toolchain_skip_not_as_unfetched_dependencies() {
        let readiness = backend_readiness(Language::Elixir, &cfg("false", Some("false")));

        assert_eq!(readiness, BackendReadiness::ToolchainMissing);
    }

    /// A machine without a language's toolchain must still be able to build the rest, so this
    /// bucket alone leaves the exit status clean. ~keep
    #[test]
    fn should_exit_clean_when_the_only_thing_that_happened_was_a_toolchain_skip() {
        assert!(build_outcome(&[], &[]).is_ok());
    }

    /// Non-zero, but never described as a build failure: nothing was compiled, so the message
    /// says what to run instead of implying the generated code is broken. Exiting 0 here is what
    /// would let a downstream consumer treat a missing artifact as a built one. ~keep
    #[test]
    fn should_fail_the_run_for_unmet_preconditions_while_naming_them_separately_from_failures() {
        let error = build_outcome(&[], &["elixir (run `cd packages/elixir && mix deps.get`)".to_string()])
            .expect_err("unmet preconditions must not exit clean");
        let message = error.to_string();

        assert!(message.contains("1 language(s) were not built"), "{message}");
        assert!(message.contains("not a compile failure"), "{message}");
        assert!(message.contains("mix deps.get"), "{message}");
        assert!(!message.contains("backend build failed"), "{message}");
    }

    /// Both buckets in one run stay countable on their own — the reader must be able to tell how
    /// many backends were actually compiled and wrong. ~keep
    #[test]
    fn should_keep_failure_and_unmet_counts_separate_when_both_occur() {
        let error = build_outcome(
            &["go: undefined: Foo".to_string()],
            &["elixir (run `mix deps.get`)".to_string()],
        )
        .expect_err("either bucket is fatal");
        let message = error.to_string();

        assert!(message.contains("backend build failed for 1 language(s)"), "{message}");
        assert!(message.contains("1 language(s) were not built"), "{message}");
    }
}

#[cfg(test)]
mod build_command_tests {
    use super::*;
    use crate::core::backend::{BuildConfig, BuildDependency};

    #[test]
    fn csharp_build_command_uses_verbosity_flag_not_query_mode() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        let command = build_command_for(Language::Csharp, &build_config, &config, false);

        assert!(
            command.contains("--verbosity quiet"),
            "C# build must use explicit quiet verbosity: {command}"
        );
        assert!(
            !command.contains(" -q"),
            "C# build must not use dotnet query mode shorthand: {command}"
        );
    }

    #[test]
    fn kotlin_gradle_build_command_runs_in_generated_package() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        assert_eq!(
            build_command_for(Language::Kotlin, &build_config, &config, false),
            "cd packages/kotlin && gradle build"
        );
        assert_eq!(
            build_command_for(Language::Kotlin, &build_config, &config, true),
            "cd packages/kotlin && gradle build -Prelease"
        );
    }

    /// Regression test for the real consumer failure this arm was rewritten for: an
    /// explicit `[crates.output] kotlin_android` pointing at a deep, gradle-marker-free
    /// namespace source directory (`.../src/main/kotlin/io/<ns>/<pkg>/android/`) used to be
    /// `cd`-ed into verbatim, and gradle rejected it ("Project directory '...' is not part
    /// of the build defined by settings file '...'"). The fix must walk up to the nearest
    /// `settings.gradle.kts` and build from there instead. ~keep
    #[test]
    fn gradle_build_walks_up_to_settings_gradle_root_from_deep_namespace_dir() {
        let root = tempfile::tempdir().expect("failed to create temp dir");
        let project_root = root.path().join("packages/kotlin-android");
        let deep_source_dir = project_root.join("src/main/kotlin/io/alpha/mylib/android");
        std::fs::create_dir_all(&deep_source_dir).expect("failed to create deep namespace dir");
        std::fs::write(
            project_root.join("settings.gradle.kts"),
            "rootProject.name = \"alpha\"\n",
        )
        .expect("failed to write settings.gradle.kts fixture");
        std::fs::write(project_root.join("build.gradle.kts"), "// android library build\n")
            .expect("failed to write build.gradle.kts fixture");

        // Prove the fixture actually built the shape under test before asserting on it:
        // both markers must exist at the project root and be absent from the deep dir.
        assert!(project_root.join("settings.gradle.kts").is_file());
        assert!(project_root.join("build.gradle.kts").is_file());
        assert!(!deep_source_dir.join("settings.gradle.kts").exists());
        assert!(!deep_source_dir.join("build.gradle.kts").exists());

        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.alpha"

[crates.output]
kotlin_android = "{deep_source_dir}/"
"#,
            deep_source_dir = deep_source_dir.display(),
        ))
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        let expected_root = project_root.display().to_string();
        assert_eq!(
            build_command_for(Language::KotlinAndroid, &build_config, &config, false),
            format!("cd {expected_root} && gradle build")
        );
        assert_eq!(
            build_command_for(Language::KotlinAndroid, &build_config, &config, true),
            format!("cd {expected_root} && gradle build -Prelease")
        );
    }

    /// Boundary case: no `settings.gradle*`/`build.gradle*` exists anywhere up the ancestor
    /// chain, so the walk-up must fall back to the original source directory unchanged
    /// (like the mix/mvn/dotnet arms above it) instead of walking to the filesystem root or
    /// panicking. ~keep
    #[test]
    fn gradle_build_falls_back_to_source_dir_when_no_marker_found() {
        let root = tempfile::tempdir().expect("failed to create temp dir");
        let deep_source_dir = root
            .path()
            .join("packages/kotlin-android/src/main/kotlin/io/alpha/mylib/android");
        std::fs::create_dir_all(&deep_source_dir).expect("failed to create deep namespace dir");

        // Prove the fixture is genuinely marker-free before asserting on the fallback.
        assert!(deep_source_dir.is_dir());
        for ancestor in deep_source_dir.ancestors() {
            for marker in [
                "settings.gradle.kts",
                "settings.gradle",
                "build.gradle.kts",
                "build.gradle",
            ] {
                assert!(
                    !ancestor.join(marker).exists(),
                    "fixture must not contain {marker} under {}",
                    ancestor.display()
                );
            }
            if ancestor == root.path() {
                break;
            }
        }

        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.alpha"

[crates.output]
kotlin_android = "{deep_source_dir}/"
"#,
            deep_source_dir = deep_source_dir.display(),
        ))
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        // The configured output path carries a trailing separator and the fallback returns it
        // verbatim, so the expectation must too -- `cd <dir>/` is what a real run emits. ~keep
        let expected = deep_source_dir.display().to_string();
        assert_eq!(
            build_command_for(Language::KotlinAndroid, &build_config, &config, false),
            format!("cd {expected}/ && gradle build")
        );
    }

    #[test]
    fn unknown_build_tool_fails_instead_of_reporting_success() {
        let config = ResolvedCrateConfig::default();
        let build_config = BuildConfig {
            tool: "unsupported",
            crate_suffix: "",
            build_dep: BuildDependency::None,
            post_build: Vec::new(),
        };

        let command = build_command_for(Language::Kotlin, &build_config, &config, false);
        assert!(
            command.ends_with("&& false"),
            "unknown build tool must still exit non-zero: {command}"
        );
        assert!(
            command.contains("no default build command for tool \"unsupported\""),
            "unknown build tool failure must name the missing tool instead of a bare `false`: {command}"
        );
    }

    fn wasm_config(extra: &str) -> ResolvedCrateConfig {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
{extra}
"#
        ))
        .unwrap();
        alef_cfg.resolve().unwrap().remove(0)
    }

    fn wasm_build_config() -> BuildConfig {
        BuildConfig {
            tool: "wasm-pack",
            crate_suffix: "-wasm",
            build_dep: BuildDependency::None,
            post_build: Vec::new(),
        }
    }

    /// Regression test: `alef e2e` (local mode) resolves the wasm package through
    /// `pkg/nodejs` (`ResolvedCrateConfig::wasm_crate_path`), and the scaffolded crate
    /// manifest resolves `main`/`types` to `pkg/nodejs/…` and `module` to `pkg/web/…`.
    /// `alef build` used to run a single `--target web` with wasm-pack's *default* out-dir,
    /// producing a bare `pkg/` that none of those consumers read while never producing
    /// `pkg/nodejs` at all — so a fresh checkout needed a hand-rolled extra build step.
    /// The emitted command must match the scaffolded `build:wasm:<target>` scripts exactly,
    /// since `publish::package::wasm` re-runs those same scripts via `npm run build:all`
    /// and the two must not disagree about where a target lands. ~keep
    #[test]
    fn wasm_build_command_builds_every_default_target_into_its_own_pkg_subdir() {
        let config = wasm_config("");

        let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

        assert_eq!(
            command,
            "cd crates/sample-lib-wasm && \
             wasm-pack build --dev --target web --out-dir pkg/web && \
             wasm-pack build --dev --target bundler --out-dir pkg/bundler && \
             wasm-pack build --dev --target nodejs --out-dir pkg/nodejs && \
             wasm-pack build --dev --target deno --out-dir pkg/deno"
        );
    }

    /// The bare `pkg/` that the pre-fix command produced is exactly what nothing consumes;
    /// asserting its absence is what distinguishes this fix from "also happens to build web".
    #[test]
    fn wasm_build_command_never_leaves_a_target_in_the_bare_pkg_dir() {
        let command = build_command_for(Language::Wasm, &wasm_build_config(), &wasm_config(""), true);

        assert!(
            !command.contains("--target web --out-dir pkg "),
            "no target may use wasm-pack's default bare `pkg/` out-dir: {command}"
        );
        for target in ["web", "bundler", "nodejs", "deno"] {
            assert!(
                command.contains(&format!("--target {target} --out-dir pkg/{target}")),
                "release build must still pair every target with its own out-dir: {command}"
            );
        }
        assert!(command.contains("--release"), "{command}");
        assert!(!command.contains("--dev"), "{command}");
    }

    /// Failure path: a crate that narrows `[crates.wasm] targets` to keep its published
    /// package small must get exactly that set — not a hardcoded web+nodejs pair. This is
    /// the case a hardcoded command silently gets wrong, and it also proves the emitted
    /// set is genuinely read from config rather than a constant that happens to match the
    /// default. Note `pkg/nodejs` is absent here, so `alef e2e` cannot resolve the package
    /// for such a crate — a config-consistency problem that belongs to config validation,
    /// not something the build command should paper over by force-building nodejs. ~keep
    #[test]
    fn wasm_build_command_honours_a_narrowed_target_set() {
        let config = wasm_config("\n[crates.wasm]\ntargets = [\"web\"]\n");

        let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

        assert_eq!(
            command,
            "cd crates/sample-lib-wasm && wasm-pack build --dev --target web --out-dir pkg/web"
        );
        assert!(!command.contains("nodejs"), "{command}");
    }

    /// An empty target list must fail loudly and name the setting, matching the unknown-tool
    /// arm, rather than emitting a dangling `cd <dir> && ` the shell rejects as a syntax error
    /// that names neither alef nor the offending config key.
    #[test]
    fn wasm_build_command_fails_loudly_on_an_empty_target_set() {
        let config = wasm_config("\n[crates.wasm]\ntargets = []\n");

        let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

        assert!(command.ends_with("&& false"), "must exit non-zero: {command}");
        assert!(command.contains("[crates.wasm] targets is empty"), "{command}");
        assert!(
            !command.contains("wasm-pack build"),
            "must not emit a truncated wasm-pack invocation: {command}"
        );
    }

    #[test]
    fn swift_build_command_uses_swift_build_with_package_path() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "swift",
            crate_suffix: "-swift",
            build_dep: BuildDependency::None,
            post_build: Vec::new(),
        };

        assert_eq!(
            build_command_for(Language::Swift, &build_config, &config, false),
            "swift build --package-path packages/swift"
        );
        assert_eq!(
            build_command_for(Language::Swift, &build_config, &config, true),
            "swift build --package-path packages/swift --configuration release"
        );
    }

    #[test]
    fn zig_build_command_uses_zig_build() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "zig",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        assert_eq!(
            build_command_for(Language::Zig, &build_config, &config, false),
            "cd packages/zig && zig build"
        );
        assert_eq!(
            build_command_for(Language::Zig, &build_config, &config, true),
            "cd packages/zig && zig build --release=fast"
        );
    }

    #[test]
    fn gleam_build_command_uses_gleam_build() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "gleam",
            crate_suffix: "",
            build_dep: BuildDependency::Rustler,
            post_build: Vec::new(),
        };

        assert_eq!(
            build_command_for(Language::Gleam, &build_config, &config, false),
            "cd packages/gleam && gleam build"
        );
        assert_eq!(
            build_command_for(Language::Gleam, &build_config, &config, true),
            "cd packages/gleam && gleam build"
        );
    }

    // Regression test for a real crash found while investigating the "false"
    // command-substitution incident: `registry::get_backend` panics for `C`
    // (it has no binding backend — it's an e2e/consumer-only target), and the
    // classification loop used to call it unconditionally for every
    // non-Rust language. A `[workspace] languages` list that includes "c"
    // (a documented, valid e2e target) would crash the whole build instead
    // of skipping C gracefully like any other backend-less language. ~keep
    #[test]
    fn c_language_is_skipped_gracefully_instead_of_panicking() {
        let config = ResolvedCrateConfig::default();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(&config, &[Language::C], false)));

        match result {
            Ok(build_result) => assert!(
                build_result.is_ok(),
                "C has no binding backend and must be skipped cleanly: {build_result:?}"
            ),
            Err(_) => panic!("building an unsupported binding target must not panic"),
        }
    }

    /// Regression test: wasm-pack derives `pkg/nodejs/package.json`'s `"name"` from the wasm
    /// crate's `Cargo.toml`, not from `config.wasm_package_name()` — so every `file:` dependency
    /// and `require()`/`import` specifier the wasm e2e codegen emits (which use
    /// `wasm_package_name()`) would name a package the directory does not declare unless
    /// something patches it after the build. This proves the patch is surgical: only the
    /// `"name"` field changes, every other field (including unrelated `"name"`-shaped strings
    /// elsewhere in the file) is untouched.
    #[test]
    fn rewrite_wasm_package_json_name_only_touches_the_name_field() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let manifest_path = dir.path().join("package.json");
        std::fs::write(
            &manifest_path,
            r#"{
  "name": "sample-core-wasm",
  "version": "0.1.0",
  "description": "not a name field: \"name\" appears here too",
  "main": "sample_core_wasm.js"
}
"#,
        )
        .expect("failed to write fixture package.json");

        rewrite_wasm_package_json_name(&manifest_path, "@xberg-io/sample-crate-wasm").expect("rewrite must succeed");

        let rewritten = std::fs::read_to_string(&manifest_path).expect("failed to read rewritten package.json");
        assert!(
            rewritten.contains(r#""name": "@xberg-io/sample-crate-wasm""#),
            "{rewritten}"
        );
        assert!(!rewritten.contains("sample-core-wasm"), "{rewritten}");
        assert!(
            rewritten.contains(r#""main": "sample_core_wasm.js""#),
            "unrelated fields must survive untouched: {rewritten}"
        );
        assert!(
            rewritten.contains(r#"not a name field: \"name\" appears here too"#),
            "a `\"name\"` substring inside another field's value must not be touched: {rewritten}"
        );
    }

    #[test]
    fn rewrite_wasm_package_json_name_is_idempotent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let manifest_path = dir.path().join("package.json");
        std::fs::write(&manifest_path, r#"{"name": "already-correct"}"#).expect("failed to write fixture");

        rewrite_wasm_package_json_name(&manifest_path, "already-correct").expect("rewrite must succeed");

        let rewritten = std::fs::read_to_string(&manifest_path).expect("failed to read package.json");
        assert_eq!(rewritten, r#"{"name": "already-correct"}"#);
    }

    /// End-to-end regression through the public `run_post_build` entry point, proving the
    /// `PostBuildStep::RewriteWasmPackageName` variant is actually wired up (not just the
    /// private helper it calls) and resolves `package_json_path` relative to `base_dir`
    /// directly — not `base_dir.join(crate_dir)`, unlike every other `PostBuildStep` — since
    /// the crate directory (`config.package_dir(Language::Wasm)`'s default-formula fallback,
    /// used when `[crates.output] wasm` isn't set) must already be baked into the path by
    /// the caller that constructs this step.
    #[test]
    fn run_post_build_rewrites_wasm_package_json_name_relative_to_base_dir() {
        let base_dir = tempfile::tempdir().expect("failed to create temp dir");
        let pkg_dir = base_dir.path().join("crates/sample-lib-wasm/pkg/nodejs");
        std::fs::create_dir_all(&pkg_dir).expect("failed to create pkg/nodejs");
        std::fs::write(pkg_dir.join("package.json"), r#"{"name": "sample-lib-wasm-crate"}"#)
            .expect("failed to write fixture package.json");

        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "wasm-pack",
            crate_suffix: "-wasm",
            build_dep: BuildDependency::None,
            post_build: vec![crate::core::backend::PostBuildStep::RewriteWasmPackageName {
                package_json_path: std::path::PathBuf::from("crates/sample-lib-wasm/pkg/nodejs/package.json"),
                package_name: config.wasm_package_name(),
            }],
        };

        run_post_build(Language::Wasm, &build_config, &config, base_dir.path()).expect("post-build must succeed");

        let rewritten =
            std::fs::read_to_string(pkg_dir.join("package.json")).expect("failed to read rewritten package.json");
        assert!(
            rewritten.contains(&format!(r#""name": "{}""#, config.wasm_package_name())),
            "{rewritten}"
        );
    }
}

#[cfg(all(test, unix))]
mod build_orchestration_tests {
    use super::*;

    fn hermetic_config(toml: &str) -> ResolvedCrateConfig {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(toml).unwrap();
        alef_cfg.resolve().unwrap().remove(0)
    }

    /// `go` is `ffi_dependent` (`BuildDependency::Ffi`) while `php` and `node`
    /// are `independent`. `php` fails; the old code's `result?` in the
    /// `independent` consumption loop returned from `build()` right there,
    /// before the `ffi_dependent` stage ever ran — silently dropping `go`
    /// (and every other `ffi_dependent` language) with zero log output. This
    /// is the "false" command-substitution incident's real blast radius.
    ///
    /// `ffi` is included as an independent target purely so `independent`
    /// already contains a `tool == "cargo" && crate_suffix == "-ffi"` entry:
    /// that short-circuits `build()`'s auto FFI-crate-build step (which would
    /// otherwise shell out to a real `cargo build -p <crate>-ffi` against a
    /// package that doesn't exist in this synthetic config), keeping the test
    /// hermetic — only `sh -c true`/`sh -c false`/`touch` ever run.
    ///
    /// Proof that `node` and `go` were actually dispatched uses marker files
    /// written by their build commands, not `tracing-test`'s `logs_contain`:
    /// `node`/`go` build inside `independent`/`ffi_dependent`'s
    /// `.par_iter()`, which runs on rayon's worker threads. `tracing-test`
    /// scopes captured logs to a span entered via a thread-local guard on the
    /// test's own thread — that guard does not propagate to rayon's pool, so
    /// log lines from those closures would not carry the test's scope prefix
    /// and `logs_contain` would be unreliable here regardless of whether the
    /// underlying fix is correct. ~keep
    #[test]
    fn one_backend_failure_does_not_block_the_others() {
        let marker_dir = tempfile::tempdir().expect("failed to create temp dir for build markers");
        let marker_node = marker_dir.path().join("node.built");
        let marker_go = marker_dir.path().join("go.built");

        let config = hermetic_config(&format!(
            r#"
[workspace]
languages = ["php", "node", "ffi", "go"]

[workspace.build_commands.php]
precondition = "true"
build = "false"

[workspace.build_commands.node]
precondition = "true"
build = "touch {node_marker}"

[workspace.build_commands.ffi]
precondition = "true"
build = "true"

[workspace.build_commands.go]
precondition = "true"
build = "touch {go_marker}"

[[crates]]
name = "orchestration-test-lib"
sources = ["src/lib.rs"]
"#,
            node_marker = marker_node.display(),
            go_marker = marker_go.display(),
        ));

        let result = build(
            &config,
            &[Language::Php, Language::Node, Language::Ffi, Language::Go],
            false,
        );

        assert!(result.is_err(), "php's failure must surface in the aggregate result");
        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("php"),
            "aggregate error must name the failed language: {message}"
        );

        assert!(
            marker_node.exists(),
            "node (independent, ordered after php in the list) must still be attempted and succeed"
        );
        assert!(
            marker_go.exists(),
            "go (ffi_dependent) must still be attempted and succeed even though the independent \
             stage had a failure — that's the class of language that used to be silently dropped"
        );
    }
}

#[cfg(all(test, unix))]
mod run_command_tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_skip_env(previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("ALEF_SKIP_COMMANDS", value),
                None => std::env::remove_var("ALEF_SKIP_COMMANDS"),
            }
        }
    }

    #[test]
    fn run_run_command_succeeds_for_echo() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("echo", &["alef-runcommand-ok"], &dir, "sample");
        restore_skip_env(previous);
        assert!(result.is_ok(), "echo should succeed: {result:?}");
    }

    #[test]
    fn run_run_command_fails_for_false() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("false", &[], &dir, "sample");
        restore_skip_env(previous);
        assert!(result.is_err(), "false should return Err");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("exited with status"),
            "error should mention exit status: {msg}"
        );
    }

    #[test]
    fn run_run_command_honors_skip_env_var() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        let dir = std::env::temp_dir();
        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "noop,false , another");
        }
        let skipped = run_run_command("false", &[], &dir, "sample");
        assert!(
            skipped.is_ok(),
            "listed command must return Ok without spawning: {skipped:?}"
        );

        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "something-else");
        }
        let honored = run_run_command("false", &[], &dir, "sample");
        restore_skip_env(previous);
        assert!(
            honored.is_err(),
            "unlisted command must still spawn and surface failure"
        );
    }
}
