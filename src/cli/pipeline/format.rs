use crate::core::config::{Language, ResolvedCrateConfig};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tracing::{debug, warn};

/// One residual formatter invocation poly cannot perform (project-wide tools that
/// don't fit poly's per-file model): `cargo sort`, `mix format`, `dotnet format`.
#[derive(Debug)]
struct ResidualStep {
    command: String,
    args: Vec<String>,
    work_dir: PathBuf,
}

/// A code formatter that shapes generated output and must be present for a run.
#[derive(Clone, Copy)]
struct RequiredFormatter {
    tool: &'static str,
    install_hint: &'static str,
}

/// The formatters whose presence is required for deterministic generation of
/// `languages`.
///
/// `rustfmt` and `poly` are always required: every binding emits a Rust glue
/// crate that rustfmt reflows, and poly formats each language's emitted package.
/// `cargo-sort` is required only when a language whose residual pass runs
/// `cargo sort` is generated (wasm, ffi, ruby, elixir, r). `mix` is required only
/// when Elixir is generated: poly's own pass excludes `.ex`/`.exs` files (see
/// [`POLY_ELIXIR_EXCLUDE_GLOBS`]) because its pure-Rust Elixir formatter misindents
/// them, so `mix format` is the sole formatter for that output and its absence
/// must not pass unnoticed.
fn required_formatters(languages: &[Language]) -> Vec<RequiredFormatter> {
    let mut required = vec![
        RequiredFormatter {
            tool: "rustfmt",
            install_hint: "rustup component add rustfmt",
        },
        RequiredFormatter {
            tool: "poly",
            install_hint: "install polylint (`poly`) and put it on PATH",
        },
    ];
    let needs_cargo_sort = languages.iter().any(|language| {
        matches!(
            language,
            Language::Wasm | Language::Ffi | Language::Ruby | Language::Elixir | Language::R
        )
    });
    if needs_cargo_sort {
        required.push(RequiredFormatter {
            tool: "cargo-sort",
            install_hint: "cargo install cargo-sort",
        });
    }
    if languages.contains(&Language::Elixir) {
        required.push(RequiredFormatter {
            tool: "mix",
            install_hint: "install Elixir (https://elixir-lang.org/install.html); `mix` ships with it",
        });
    }
    required
}

/// Warn (never fail) when a formatter that shapes generated output is missing
/// from PATH.
///
/// alef always applies formatting when the tools are present — poly in
/// particular formats through `poly fmt` whenever it is on PATH and the pass is
/// skipped otherwise. A missing formatter (`rustfmt`, `poly`, `cargo-sort`, or
/// `mix`) can leave output un(der)-formatted and host-dependent, which may trip
/// the freshness check (#184); rather than abort generation, warn and name each
/// missing tool and how to install it so the operator can restore deterministic
/// output. `mix` in particular has no fallback: unlike the other residuals it is
/// the *sole* formatter for `.ex`/`.exs` output (poly is deliberately excluded,
/// see [`POLY_ELIXIR_EXCLUDE_GLOBS`]), so a missing `mix` means generated Elixir
/// source is left completely unformatted, not merely under-formatted.
pub fn warn_missing_formatters(languages: &[Language]) {
    let missing: Vec<RequiredFormatter> = required_formatters(languages)
        .into_iter()
        .filter(|formatter| !is_tool_available(formatter.tool))
        .collect();
    if missing.is_empty() {
        return;
    }
    let details = missing
        .iter()
        .map(|formatter| format!("  - {}: {}", formatter.tool, formatter.install_hint))
        .collect::<Vec<_>>()
        .join("\n");
    warn!(
        "code formatter(s) not found on PATH; generated output may be un(der)-formatted and \
         host-dependent (#184). Install to restore deterministic formatting:\n{details}"
    );
}

/// Run language-native formatters on emitted packages after generation.
///
/// Formatting is always delegated to the `poly` (polylint) CLI. On a full regen
/// (`only_languages = None`, the `alef all` path) this converges to a fixed point:
/// see [`converge_full_regen_formatting`]. On a partial regen (a single language's
/// files changed) a single `poly fmt --fix` pass runs over the changed language's
/// package directory, followed by that language's residual native pass for the
/// project-wide tools poly cannot wrap (wasm/ruby/elixir/R native crate sort, plus
/// `mix format` for Elixir's `.ex`/`.exs` source — poly is excluded from those,
/// see [`POLY_ELIXIR_EXCLUDE_GLOBS`]).
///
/// Best-effort: a missing `poly` binary, a poly error, or a missing residual tool
/// is logged as a warning and never aborts the generate command.
pub fn format_generated(
    files: &[(Language, Vec<crate::core::backend::GeneratedFile>)],
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
) {
    let mut seen = HashSet::new();
    let poly_langs: Vec<Language> = files
        .iter()
        .map(|(lang, _)| *lang)
        .filter(|lang| seen.insert(*lang) && only_languages.is_none_or(|filter| filter.contains(lang)))
        .collect();

    if poly_langs.is_empty() {
        return;
    }

    match only_languages {
        None => converge_full_regen_formatting(base_dir),
        Some(_) => {
            let paths = poly_paths(config, base_dir, only_languages, &poly_langs);
            poly_format(&paths, base_dir);
            for &lang in &poly_langs {
                let lang_str = lang.to_string().to_lowercase();
                for step in language_residuals(config, lang, base_dir) {
                    run_residual(&step, &lang_str);
                }
            }
        }
    }
}

/// Maximum `poly fmt --fix` passes attempted while converging a full regen.
///
/// Some poly-bundled engines (`.cs`, `.java`, `.json` today) are not single-pass
/// idempotent on freshly generated output: a first `poly fmt --fix` pass can still
/// leave `poly fmt --check` reporting drift. Looping converges them so a full
/// regen is committable without a manual cleanup pass downstream (see #184-style
/// freshness-check failures).
const MAX_POLY_FMT_PASSES: u32 = 3;

/// Self-cleaning full-regen formatting pass, used on the `alef all` path
/// (`only_languages = None`).
///
/// Loops `poly fmt --fix <base_dir>` to a fixed point (detected via `poly fmt
/// --check`, bounded by [`MAX_POLY_FMT_PASSES`]), folding a workspace-wide
/// `cargo fmt --all` and a workspace-wide `cargo sort -n -w` into *every* pass of
/// the same loop. Running them inside the loop — rather than once, after —
/// means that if either tool disagrees with poly's own formatting, the next
/// pass's `poly fmt --fix`/`--check` observes and reconciles the drift instead of
/// leaving the tree dirty.
///
/// This replaces the old per-language cargo-sort residuals on a full regen: those
/// only covered the language whose crate directory they targeted (and the
/// workspace-wide `-w` variant ran only when the ffi target was generated),
/// leaving other generated crates (python, node, php, swift, dart, …) unsorted —
/// exactly the gap that trips poly's own workspace-wide cargo-sort check
/// downstream. A single `cargo sort -n -w` at the repo root covers every crate in
/// the workspace regardless of which languages this run generated.
///
/// `.ex`/`.exs` output is handled outside this loop entirely: poly's own pass
/// excludes them (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]), so [`run_elixir_mix_format`]
/// runs once, after the loop settles, regardless of whether poly converged --
/// the two concerns are independent and neither blocks the other.
///
/// Best-effort throughout: a missing `poly`, `cargo`, `rustfmt`, `cargo-sort`, or
/// `mix` is a warning, never a failure, and generation is never aborted.
fn converge_full_regen_formatting(base_dir: &Path) {
    let poly_present = is_tool_available("poly");
    if !poly_present {
        warn!("poly not found on PATH (skipping post-generation formatting)");
    }
    let root = vec![base_dir.to_path_buf()];

    for _pass in 1..=MAX_POLY_FMT_PASSES {
        if poly_present {
            poly_format(&root, base_dir);
        }
        run_cargo_fmt(base_dir);
        run_workspace_cargo_sort(base_dir);

        if !poly_present || poly_fmt_is_clean(base_dir) {
            run_elixir_mix_format(base_dir);
            return;
        }
    }
    warn!(
        "poly fmt did not converge after {MAX_POLY_FMT_PASSES} passes (non-fatal); generated \
         output may have residual formatting drift"
    );
    run_elixir_mix_format(base_dir);
}

/// Check `poly fmt --check <base_dir>` for a clean (already-formatted) tree. Used
/// only to detect convergence inside [`converge_full_regen_formatting`]'s loop.
///
/// Excludes the same Elixir globs as [`poly_format`] (see
/// [`POLY_ELIXIR_EXCLUDE_GLOBS`]): without this, the check would judge
/// `mix format`'s correct output by poly's own (incompatible) Elixir formatting
/// opinion and never report clean, spinning the convergence loop to its cap on
/// every full regen that generates Elixir.
fn poly_fmt_is_clean(base_dir: &Path) -> bool {
    let path_str = base_dir.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["fmt".to_owned(), "--check".to_owned(), path_str];
    push_poly_elixir_excludes(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_formatter("poly", &arg_refs, base_dir).is_ok()
}

/// Run `cargo fmt --all` at the workspace root, when `cargo`, `rustfmt`, and a
/// root `Cargo.toml` are all present. Folded into
/// [`converge_full_regen_formatting`]'s loop rather than run once afterward, so a
/// later `poly fmt` pass reconciles anything cargo fmt changes that poly's own
/// per-file rustfmt invocation did not already produce. Best-effort: a missing
/// root `Cargo.toml` is a debug/skip (not every generated tree is a cargo
/// workspace); a missing tool is a warning/skip; a non-zero exit is a warning.
fn run_cargo_fmt(base_dir: &Path) {
    if !base_dir.join("Cargo.toml").exists() {
        debug!(
            "no root Cargo.toml at {}, skipping workspace cargo fmt",
            base_dir.display()
        );
        return;
    }
    if !is_tool_available("cargo") || !is_tool_available("rustfmt") {
        warn!("cargo/rustfmt not found on PATH (skipping workspace cargo fmt)");
        return;
    }
    match run_formatter("cargo", &["fmt", "--all"], base_dir) {
        Ok(()) => debug!("cargo fmt --all ok"),
        Err(e) => warn!("cargo fmt --all failed (non-fatal): {e}"),
    }
}

/// Run `cargo sort -n -w` once at the workspace root, covering every crate in the
/// workspace regardless of which languages this run generated. See
/// [`converge_full_regen_formatting`] for why this replaces the per-language
/// residuals on a full regen. The `-n` flag skips cargo-sort's own post-sort
/// formatting pass (which would otherwise fight poly's TOML formatter over
/// whitespace/quote style); it does not affect table or dependency ordering, so
/// it does not change what poly's bundled cargo-sort check accepts as sorted.
/// Best-effort: a missing root `Cargo.toml` is a debug/skip; a missing
/// `cargo-sort` binary is a warning/skip; a non-zero exit is a warning.
fn run_workspace_cargo_sort(base_dir: &Path) {
    if !base_dir.join("Cargo.toml").exists() {
        debug!(
            "no root Cargo.toml at {}, skipping workspace cargo sort",
            base_dir.display()
        );
        return;
    }
    if !is_tool_available("cargo-sort") {
        warn!("cargo-sort not found on PATH (skipping workspace cargo sort)");
        return;
    }
    match run_formatter("cargo", &["sort", "-n", "-w"], base_dir) {
        Ok(()) => debug!("cargo sort -n -w ok"),
        Err(e) => warn!("cargo sort -n -w failed (non-fatal): {e}"),
    }
}

/// Run `poly fmt --fix <base_dir>`. Best-effort: a missing `poly` binary or
/// non-zero exit is logged as a warning and never propagated.
pub fn poly_fmt(base_dir: &Path) {
    let paths = vec![base_dir.to_path_buf()];
    poly_format(&paths, base_dir);
}

/// Run `poly lint <base_dir>`. Propagates failure — a non-zero exit is an error.
pub fn poly_lint(base_dir: &Path) -> anyhow::Result<()> {
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping lint)");
        return Ok(());
    }
    let path_str = base_dir.to_string_lossy().into_owned();
    let arg_refs: Vec<&str> = vec!["lint", &path_str];
    match run_formatter("poly", &arg_refs, base_dir) {
        Ok(()) => {
            debug!("poly lint ok");
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("poly lint failed: {e}")),
    }
}

/// Return the fixed set of all cargo-sort residual steps that alef always runs
/// after formatting, regardless of which languages the config targets.
///
/// The fixed set covers: workspace-wide (via ffi), wasm, ruby, elixir, R.
/// Dart and swift have no cargo residuals (poly covers them).
///
/// Filters each language's residuals down to `cargo` steps: Elixir's residual
/// list also carries `mix deps.get`/`mix format` steps (see
/// [`language_residuals`]'s `Language::Elixir` arm), which are a distinct
/// formatting concern from cargo-sort and out of scope for a function whose name
/// and every caller assume "cargo sort only."
fn cargo_sort_residuals(config: &ResolvedCrateConfig, base_dir: &Path) -> Vec<ResidualStep> {
    let mut steps = Vec::new();
    for language in [
        Language::Ffi,
        Language::Wasm,
        Language::Ruby,
        Language::Elixir,
        Language::R,
    ] {
        steps.extend(
            language_residuals(config, language, base_dir)
                .into_iter()
                .filter(|step| step.command == "cargo"),
        );
    }
    steps
}

/// Run all cargo-sort residuals (ffi workspace, wasm, ruby, elixir, R). Best-effort.
pub(crate) fn run_cargo_sort_residuals(config: &ResolvedCrateConfig, base_dir: &Path) {
    for step in cargo_sort_residuals(config, base_dir) {
        run_residual(&step, "residual");
    }
}

/// Paths to hand to poly. Full regen → the repo root (one pass). Partial regen →
/// the package directory of each changed language (existing dirs only, deduped).
fn poly_paths(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    poly_langs: &[Language],
) -> Vec<PathBuf> {
    match only_languages {
        None => vec![base_dir.to_path_buf()],
        Some(_) => {
            let mut seen = HashSet::new();
            let mut dirs = Vec::new();
            for &lang in poly_langs {
                let dir = base_dir.join(config.package_dir(lang));
                if seen.insert(dir.clone()) && dir.exists() {
                    dirs.push(dir);
                }
            }
            dirs
        }
    }
}

/// Glob patterns excluded from every poly `fmt` invocation, `--fix` and `--check`
/// alike (see [`poly_format`] and [`poly_fmt_is_clean`]): `.ex`/`.exs` sources are
/// formatted solely by `mix format` (see `language_residuals`'s `Language::Elixir`
/// arm and [`run_elixir_mix_format`]).
///
/// poly's pure-Rust Elixir formatter misindents constructs that `mix format`
/// emits correctly — multi-line struct/map field continuation collapses from
/// mix's canonical 10-space width to flush-left 2-space, and `|>` pipe
/// continuation drops from 6 spaces to 4 — and then reports its own corrupted
/// output as `--check`-clean, so no freshness gate ever catches the drift.
/// Excluding poly from `.ex`/`.exs` entirely, rather than reformatting after it,
/// avoids that same class of bug recurring: a `--check` pass that still
/// considers itself authoritative over files it no longer formats.
///
/// Anchored with a bare `**/` prefix (no `packages/elixir/` path component) so
/// the same glob excludes correctly regardless of which root poly is given: the
/// repo root on a full regen, or the `packages/elixir` package directory itself
/// on a partial regen.
const POLY_ELIXIR_EXCLUDE_GLOBS: [&str; 2] = ["**/*.ex", "**/*.exs"];

/// Append `--exclude <glob>` for each of [`POLY_ELIXIR_EXCLUDE_GLOBS`] to `args`.
fn push_poly_elixir_excludes(args: &mut Vec<String>) {
    for glob in POLY_ELIXIR_EXCLUDE_GLOBS {
        args.push("--exclude".to_owned());
        args.push(glob.to_owned());
    }
}

/// Format `paths` by invoking the `poly` CLI (`poly fmt --fix`), rewriting changed
/// files in place. `config_start` is poly's working directory; it walks up from
/// there for `poly.toml`. Best-effort: a missing `poly` binary or a non-zero exit
/// is logged and never propagated (matching the per-language formatter contract).
///
/// Excludes `.ex`/`.exs` (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]) so `mix format`
/// remains their sole formatter.
///
/// Executable permission bits are snapshotted before the pass and restored after:
/// poly rewrites changed files via atomic rename, which resets the mode to `0644`
/// and silently strips the exec bit from every generated shebang script it
/// reformats (`run_tests.php`, `download_ffi.sh`, `mvnw`, `gradlew`, …) — which
/// poly's own `file-safety` lint then rejects on the next commit.
pub(crate) fn poly_format(paths: &[PathBuf], config_start: &Path) {
    if let Err(error) = poly_format_strict(paths, config_start) {
        warn!("poly fmt failed (non-fatal): {error}");
    }
}

pub(crate) fn poly_format_strict(paths: &[PathBuf], config_start: &Path) -> anyhow::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    if !is_tool_available("poly") {
        anyhow::bail!("poly not found on PATH; generated output cannot be formatted");
    }
    let executable_modes = snapshot_executable_modes(paths);
    let mut args: Vec<String> = vec!["fmt".to_owned(), "--fix".to_owned()];
    args.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
    push_poly_elixir_excludes(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = run_poly_formatter(&arg_refs, config_start);
    restore_executable_modes(&executable_modes);
    result?;
    debug!("poly fmt over {} path(s) ok", paths.len());
    Ok(())
}

fn run_poly_formatter(args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("poly").args(args).current_dir(work_dir).output()?;
    if poly_format_exit_code_is_success(output.status.code()) {
        return Ok(());
    }
    Err(formatter_failure(&output))
}

fn poly_format_exit_code_is_success(exit_code: Option<i32>) -> bool {
    matches!(exit_code, Some(0 | 1))
}

/// Directory names the executable-mode snapshot never descends into. They hold
/// dependency caches and build output — never alef-generated scripts — and
/// walking them on a repo-root pass costs far more than the whole format run.
#[cfg(unix)]
const EXEC_SNAPSHOT_SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "vendor",
    "deps",
    "_build",
    "build",
    ".build",
    "zig-out",
    "dist",
    "__pycache__",
    ".gradle",
    ".dart_tool",
    ".zig-cache",
    ".cache",
];

/// Mode bits granting execute permission to owner, group, or other.
#[cfg(unix)]
const EXECUTE_BITS: u32 = 0o111;

/// Record the mode of every regular file under `paths` that is currently
/// executable, so [`restore_executable_modes`] can put back exactly what was
/// there if `poly fmt` drops it.
#[cfg(unix)]
fn snapshot_executable_modes(paths: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut snapshot = Vec::new();
    for root in paths {
        let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|entry| {
            !entry.file_type().is_dir()
                || entry
                    .file_name()
                    .to_str()
                    .is_none_or(|name| !EXEC_SNAPSHOT_SKIP_DIRS.contains(&name))
        });
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            let mode = metadata.permissions().mode();
            if mode & EXECUTE_BITS != 0 {
                snapshot.push((entry.into_path(), mode));
            }
        }
    }
    snapshot
}

/// Re-apply each recorded mode whose execute bits the formatter dropped.
#[cfg(unix)]
fn restore_executable_modes(snapshot: &[(PathBuf, u32)]) {
    use std::os::unix::fs::PermissionsExt as _;
    for (path, mode) in snapshot {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        if metadata.permissions().mode() & EXECUTE_BITS == mode & EXECUTE_BITS {
            continue;
        }
        match std::fs::set_permissions(path, std::fs::Permissions::from_mode(*mode)) {
            Ok(()) => debug!("restored exec bit on {}", path.display()),
            Err(e) => warn!("failed to restore exec bit on {}: {e}", path.display()),
        }
    }
}

#[cfg(not(unix))]
fn snapshot_executable_modes(_paths: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    Vec::new()
}

#[cfg(not(unix))]
fn restore_executable_modes(_snapshot: &[(PathBuf, u32)]) {}

/// Best-effort wiring of poly's git-hook shims (`poly hooks install`) into the
/// generated repo. This installs the pre-commit + commit-msg stages declared in
/// the scaffolded `poly.toml` `[hooks]` section — polylint, polyfmt, file_safety,
/// the `cargo` builtin (clippy / cargo-sort / machete / deny), and the
/// conventional-commit `commit` hook — so every generated repository lints,
/// formats, and validates on commit without any per-repo manual setup.
///
/// No-op when `poly` is absent from PATH or `base_dir` is not a git repository.
/// Idempotent — `poly hooks install` re-writes the same shims, so it is safe to
/// run on every scaffold pass. Never aborts generation.
pub(crate) fn install_poly_hooks(base_dir: &Path) {
    if !base_dir.join(".git").exists() {
        debug!(
            "not a git repository at {}, skipping poly hooks install",
            base_dir.display()
        );
        return;
    }
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping poly hooks install)");
        return;
    }
    match run_formatter("poly", &["hooks", "install"], base_dir) {
        Ok(()) => debug!("poly hooks install ok"),
        Err(e) => warn!("poly hooks install failed (non-fatal): {e}"),
    }
}

/// Build the residual formatter steps for a language — the project-wide tools
/// poly cannot wrap because it works per-file, not per-crate/per-project.
///
/// Most languages' only residual is `cargo sort -n` for binding crates whose
/// `Cargo.toml` is excluded from the root workspace (and therefore from poly's
/// pass) — a dependency-ordering tool (not a formatter) that ships with cargo and
/// is always present in alef's build environment. C# has no residual: it is
/// formatted entirely by poly's deterministic pure-Rust tier-2 tier.
///
/// Elixir is the one exception with two residual concerns: the `cargo sort` for
/// its out-of-workspace native NIF crate, *and* `mix format` for its `.ex`/`.exs`
/// source. Poly's own Elixir engine is excluded from formatting those files at
/// all (see [`POLY_ELIXIR_EXCLUDE_GLOBS`]) because it misindents constructs `mix
/// format` emits correctly and then reports its own corrupted output as
/// `--check`-clean — so unlike every other language here, Elixir has no
/// poly-formatted fallback and a missing `mix` (flagged loudly by
/// [`required_formatters`]) leaves its source completely unformatted, not merely
/// under-formatted. The `mix deps.get` step primes `.formatter.exs`'s
/// `import_deps: [:rustler]` (the scaffolded Elixir `.formatter.exs` imports
/// rustler's formatter rules), which requires the dependency to be fetched into
/// `deps/` before `mix format` can resolve it.
fn language_residuals(config: &ResolvedCrateConfig, lang: Language, base_dir: &Path) -> Vec<ResidualStep> {
    match lang {
        Language::Wasm => {
            let crate_dir = config
                .output_for("wasm")
                .map(resolve_crate_dir)
                .unwrap_or_else(|| Path::new("crates").join(format!("{}-wasm", config.name)));
            let crate_dir_str = crate_dir.to_string_lossy().into_owned().replace('\\', "/");
            vec![cargo_sort(vec![crate_dir_str], base_dir.to_path_buf())]
        }
        Language::Ffi => vec![cargo_sort(vec!["-w".to_owned()], base_dir.to_path_buf())],
        Language::Ruby => {
            let gem_name = config.ruby_gem_name();
            let native_subdir = format!("ext/{gem_name}/native");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/ruby"))]
        }
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            let native_subdir = format!("native/{app_name}_nif");
            let elixir_dir = base_dir.join("packages/elixir");
            vec![
                cargo_sort(vec![native_subdir], elixir_dir.clone()),
                mix_deps_get(elixir_dir.clone()),
                mix_format(elixir_dir),
            ]
        }
        Language::R => vec![cargo_sort(
            vec!["packages/r/src/rust".to_owned()],
            base_dir.to_path_buf(),
        )],
        _ => vec![],
    }
}

/// Construct a `cargo sort -n` residual step. The `-n` flag preserves single-line
/// array formatting, preventing cargo-sort from expanding dependency arrays that
/// alef emits on one line for readability.
fn cargo_sort(mut sort_args: Vec<String>, work_dir: PathBuf) -> ResidualStep {
    let mut args = vec!["sort".to_owned(), "-n".to_owned()];
    args.append(&mut sort_args);
    ResidualStep {
        command: "cargo".to_owned(),
        args,
        work_dir,
    }
}

/// Construct a `mix deps.get` residual step. `.formatter.exs`'s
/// `import_deps: [:rustler]` requires the `rustler` dependency to be fetched into
/// `deps/` before `mix format` can resolve it, so this primes the dependency on
/// every pass rather than assuming a prior `mix deps.get` already ran. Runs
/// through the same best-effort [`run_residual`] as every other step: a network
/// failure here (offline, registry down) is a warning, not an abort, and
/// [`mix_format`] is still attempted afterward — its own failure (e.g. the import
/// dependency genuinely missing) is reported separately, never silently.
fn mix_deps_get(work_dir: PathBuf) -> ResidualStep {
    ResidualStep {
        command: "mix".to_owned(),
        args: vec!["deps.get".to_owned()],
        work_dir,
    }
}

/// Construct a `mix format` residual step — the sole formatter for `.ex`/`.exs`
/// output (poly's own pass excludes them, see [`POLY_ELIXIR_EXCLUDE_GLOBS`]).
fn mix_format(work_dir: PathBuf) -> ResidualStep {
    ResidualStep {
        command: "mix".to_owned(),
        args: vec!["format".to_owned()],
        work_dir,
    }
}

/// Run `mix deps.get` then `mix format` over the generated `packages/elixir`
/// package, if one exists at `base_dir`.
///
/// Used by [`converge_full_regen_formatting`] (the full-regen path), which has no
/// per-language awareness and so cannot go through [`language_residuals`]
/// directly; a partial regen gets the same two steps via that function's
/// `Language::Elixir` arm instead. Existence-gated the same way as
/// [`run_cargo_fmt`]/[`run_workspace_cargo_sort`] gate on a root `Cargo.toml`:
/// a missing `packages/elixir/mix.exs` is a debug/skip (not every crate targets
/// Elixir), not a warning. Both steps are best-effort via [`run_residual`] and
/// never abort generation.
fn run_elixir_mix_format(base_dir: &Path) {
    let elixir_dir = base_dir.join("packages/elixir");
    if !elixir_dir.join("mix.exs").exists() {
        debug!(
            "no packages/elixir/mix.exs at {}, skipping full-regen mix format",
            base_dir.display()
        );
        return;
    }
    run_residual(&mix_deps_get(elixir_dir.clone()), "elixir");
    run_residual(&mix_format(elixir_dir), "elixir");
}

/// Run a single residual step, best-effort: a missing work dir or tool is a
/// warning/skip, a non-zero exit is a warning. Never aborts generation.
fn run_residual(step: &ResidualStep, lang_str: &str) {
    if !step.work_dir.exists() {
        debug!(
            "  [{lang_str}] residual work dir does not exist: {}, skipping",
            step.work_dir.display()
        );
        return;
    }
    if !is_tool_available(&step.command) {
        warn!("[{lang_str}] residual formatter not found: {} (skipping)", step.command);
        return;
    }
    let args: Vec<&str> = step.args.iter().map(String::as_str).collect();
    match run_formatter(&step.command, &args, &step.work_dir) {
        Ok(()) => debug!("  [{lang_str}] {} {:?} ok", step.command, args),
        Err(e) => warn!("[{lang_str}] {} {:?} failed: {e}", step.command, args),
    }
}

/// Check if a tool is available on PATH.
fn is_tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a formatter command with arguments in a specific directory.
fn run_formatter(command: &str, args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new(command).args(args).current_dir(work_dir).output()?;

    if !output.status.success() {
        return Err(formatter_failure(&output));
    }

    Ok(())
}

fn formatter_failure(output: &Output) -> anyhow::Error {
    anyhow::anyhow!(
        "formatter exited with code {:?}: {}",
        output.status.code(),
        format_command_output(output)
    )
}

fn format_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
        (false, true) => format!("stdout:\n{stdout}"),
        (true, false) => format!("stderr:\n{stderr}"),
        (true, true) => "<no output>".to_string(),
    }
}

fn resolve_crate_dir(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output_path.to_path_buf())
}

#[cfg(test)]
mod tests;
