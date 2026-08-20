//! The missing release gate: drive the generation pipeline end to end over a
//! fixture workspace and assert it completes.
//!
//! Every individual stage in this pipeline has unit tests. The pipeline itself had
//! none, and five consumer regenerations died in five different stages while the
//! suite stayed green. Two of them died by `panic!` while logging zero ERROR lines,
//! so a test that only inspected a `Result` would have reported them as ordinary
//! failures with no diagnostic — hence [`StageRun::Panicked`] is a separate,
//! separately-named outcome from [`StageRun::Failed`] here.
//!
//! Scope: this gate stops at the end of *generation*. Every one of the fatal
//! regeneration failures happened before any language toolchain ran, so nothing
//! here builds a binding, invokes cargo, or validates a snippet against a built
//! artifact. It is meant to be cheap enough to run on every change.
//!
//! Not covered, deliberately and honestly:
//! - **FFI header parity** (`pipeline::check_ffi_header_freshness`). It is
//!   `pub(crate)`, so an integration test cannot call it; in `alef all` it is
//!   reached only through `bin_cli::helpers::complete_generated_artifacts`, which
//!   runs a real cargo build first; and the rustfmt-wrapped `#[cfg(any(...))]` that
//!   defeated its scanner is produced by the *formatting* stage, not by generation,
//!   so an in-process generation gate cannot even manufacture the input. What is
//!   covered instead is the precondition: the cfg-split export survives extraction
//!   and reaches the FFI backend with its predicate intact.
//! - **Post-build processing** (`complete_generated_artifacts`), for the same
//!   reason: it shells out to per-language build tooling.
//! - **`generate_service_api`**, because the fixture declares no services and a
//!   stage that legitimately processes zero items would be indistinguishable from
//!   one that never ran — the exact ambiguity this gate exists to reject.

use alef::cli::pipeline;
use alef::core::backend::GeneratedFile;
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

// ---------------------------------------------------------------------------
// Fixture surface
// ---------------------------------------------------------------------------

/// Each element below exists because it maps to a stage a real regeneration died
/// in; the stage it reaches is named in a comment at that stage's call site in
/// [`drive_pipeline`].
const FIXTURE_SOURCE: &str = r#"
#[derive(Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub title: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub document: Document,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub usage: Usage,
    pub metadata: Metadata,
    pub choices: Vec<Choice>,
}

/// Opaque handle: no public fields, so the extractor keeps `new` as a
/// constructor `MethodDef` instead of dropping it the way it drops `new` on a
/// record type. That is what carries the identifier through to the docs
/// signature renderer.
pub struct Client {
    api_key: String,
}

impl Client {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub fn label(&self) -> String {
        self.api_key.clone()
    }
}

pub trait Validator {
    fn validate(&self, input: String) -> bool;
}

pub fn complete(prompt: String) -> Result<CompletionResponse, String> {
    let _ = prompt;
    Err("unimplemented".to_string())
}

#[cfg(all(feature = "native-http", not(target_os = "windows")))]
pub fn probe_transport() -> String {
    "posix".to_string()
}

#[cfg(all(feature = "native-http", target_os = "windows"))]
pub fn probe_transport() -> String {
    "windows".to_string()
}
"#;

const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"gatelib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `__FIELDS_C_TYPES__` is the sabotage seam: the negative control blanks the
/// entry for the intermediate hop and nothing else.
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["ffi", "python", "node", "wasm"]

[[crates]]
name = "gatelib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.generate]
public_api = true

[[crates.trait_bridges]]
trait_name = "Validator"
register_fn = "register_validator"
unregister_fn = "unregister_validator"
clear_fn = "clear_validators"
registry_getter = "validator_registry"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["c", "node", "wasm", "python"]
result_fields = ["id", "model", "usage", "choices", "metadata"]

[crates.e2e.fields]
__FIELD_ALIASES__

[crates.e2e.fields_c_types]
"completion_response.usage" = "Usage"
__FIELDS_C_TYPES__

[crates.e2e.snippets]
output = "docs-site/src/snippets"
languages = ["wasm", "node"]

[crates.e2e.call]
function = "complete"
module = "gatelib"
result_var = "result"

# NOTE: deliberately no `[crates.e2e.call.overrides.c] result_type`. `complete` is a free
# `pub fn complete(prompt: String) -> Result<CompletionResponse, String>` in the fixture
# crate, so the C generated-test-file path must derive `CompletionResponse` from the IR
# return type on its own. Pinning it here would mask the thing this gate is for: the
# fallback would otherwise PascalCase the call name to `Complete`, which is not a type,
# and `ensure_leaf_field_exists` default-allows every leaf under a non-IR parent -- so the
# nested-field walk below would report success while verifying nothing. The fixture keeps a
# function whose name differs from its return type on purpose; that is the realistic shape
# and the one a call-name guess gets wrong.

[[crates.e2e.call.args]]
name = "prompt"
field = "input.prompt"
type = "string"

[crates.e2e.calls.clear_validators]
function = ""
module = "gatelib"

[crates.e2e.calls.clear_validators.overrides.node]
function = "clearValidators"

[crates.e2e.calls.clear_validators.overrides.wasm]
function = "clearValidators"

[crates.e2e.calls.clear_validators.overrides.c]
function = "clear_validators"

[crates.e2e.calls.clear_validators.overrides.python]
function = "clear_validators"
"#;

/// The intermediate-hop C type declarations the nested, alias-crossing assertion
/// needs. Withholding these is the negative control for the crawlberg failure.
const NESTED_C_TYPES: &str = r#""completion_response.metadata" = "Metadata"
"metadata.document" = "Document""#;

const FIELD_ALIASES: &str = r#""metadata.title" = "metadata.document.title""#;

const FIXTURE_COMPLETE_JSON: &str = r#"{
  "id": "complete_basic",
  "description": "A completion whose assertions cross a namespace alias",
  "docs": {
    "topic": "smoke",
    "stem": "basic-complete",
    "title": "A basic completion",
    "shows": ["id"]
  },
  "category": "smoke",
  "tags": ["smoke"],
  "input": {
    "prompt": "hello"
  },
  "assertions": [
    { "type": "not_error" },
    { "type": "equals", "field": "id", "value": "cmpl-1" },
    { "type": "equals", "field": "metadata.title", "value": "My Page" }
  ]
}
"#;

const FIXTURE_CLEAR_JSON: &str = r#"{
  "id": "clear_validators",
  "description": "Registry teardown routed through a trait bridge clear function",
  "docs": {
    "topic": "smoke",
    "stem": "clear-validators",
    "title": "Clearing registered validators",
    "shows": []
  },
  "category": "smoke",
  "tags": ["smoke"],
  "call": "clear_validators",
  "input": {},
  "assertions": [
    { "type": "not_error" }
  ]
}
"#;

/// Which deliberate defect, if any, this fixture workspace carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sabotage {
    /// The pipeline as consumers configure it.
    None,
    /// Point the e2e call at a function the crate does not export.
    ///
    /// One other sabotage was tried first and proved vacuous, which is the reason this
    /// control is worth its lines. Blanking the `fields_c_types` hops changes nothing,
    /// because those are an override the walk skips whenever the field chain resolves
    /// from the IR.
    ///
    /// An unexported call name fails at a stage this gate provably reaches, so it pins
    /// that the ledger really does classify a stage failure rather than reporting a
    /// broken run as healthy. ~keep
    UnknownCallFunction,
    /// Remove the `[crates.e2e.fields]` alias, leaving the fixture asserting
    /// `metadata.title` — a path whose leaf names no field of `Metadata`.
    ///
    /// This used to change nothing at all: the C nested-accessor walk validated every
    /// intermediate hop against the IR but let the leaf fall through to a `char*`
    /// default, emitting `gatelib_metadata_title()` — a symbol no binding generates —
    /// and reporting the stage complete. The availability oracle upstream
    /// (`FieldResolver::is_valid_for_result`) only inspects a path's FIRST segment, and
    /// `metadata` is real, so it waved the path through; no skip comment was written, so
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` had nothing to scan. A silently lost
    /// assertion is the defect class this whole gate exists for. ~keep
    MissingFieldAlias,
}

fn write_fixture_workspace(root: &Path, sabotage: Sabotage) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture e2e directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/complete_basic.json"), FIXTURE_COMPLETE_JSON).expect("write completion fixture");
    std::fs::write(root.join("fixtures/clear_validators.json"), FIXTURE_CLEAR_JSON).expect("write teardown fixture");

    let aliases = match sabotage {
        Sabotage::MissingFieldAlias => "",
        _ => FIELD_ALIASES,
    };
    let config = FIXTURE_ALEF_TOML
        .replace("__FIELDS_C_TYPES__", NESTED_C_TYPES)
        .replace("__FIELD_ALIASES__", aliases);
    let config = match sabotage {
        Sabotage::None | Sabotage::MissingFieldAlias => config,
        Sabotage::UnknownCallFunction => config.replace("function = \"complete\"", "function = \"no_such_export\""),
    };
    std::fs::write(root.join("alef.toml"), config).expect("write fixture alef.toml");
}

// ---------------------------------------------------------------------------
// Panic-vs-error isolation
// ---------------------------------------------------------------------------

static PANIC_SLOT: Mutex<Option<String>> = Mutex::new(None);
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();
/// True only while a pipeline stage is executing under `catch_unwind`. Outside a
/// stage the default hook must still print, or a failing assertion in the test
/// harness itself would be swallowed by our own hook — a gate that hides its own
/// failures is worse than no gate.
static SUPPRESS_PANIC_OUTPUT: AtomicBool = AtomicBool::new(false);

fn install_panic_capture() {
    PANIC_HOOK_INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            let location = info
                .location()
                .map(|location| format!("{}:{}", location.file(), location.line()))
                .unwrap_or_else(|| "<unknown location>".to_string());
            let mut slot = PANIC_SLOT.lock().unwrap_or_else(|error| error.into_inner());
            *slot = Some(format!("{message}  [panicked at {location}]"));
            drop(slot);
            if !SUPPRESS_PANIC_OUTPUT.load(Ordering::SeqCst) {
                previous(info);
            }
        }));
    });
}

fn take_captured_panic() -> String {
    PANIC_SLOT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .unwrap_or_else(|| "<panic payload was not captured>".to_string())
}

/// The three outcomes a pipeline stage can have. `Panicked` is separate from
/// `Failed` on purpose: a panic aborts the whole run, produces no ERROR event and
/// no `anyhow` context, and needs a different fix from a stage that reported a
/// problem properly.
#[derive(Debug)]
enum StageRun<T> {
    Completed(T),
    Failed(String),
    Panicked(String),
}

/// The panic slot and the output-suppression flag are process-global, so two
/// stages running concurrently could hand each other's payloads back. Always
/// acquired *after* `CWD_LOCK` where both are held, never before. ~keep
static STAGE_LOCK: Mutex<()> = Mutex::new(());

fn run_stage<T>(body: impl FnOnce() -> anyhow::Result<T>) -> StageRun<T> {
    install_panic_capture();
    let _serialized = STAGE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _ = take_captured_panic();
    SUPPRESS_PANIC_OUTPUT.store(true, Ordering::SeqCst);
    let result = std::panic::catch_unwind(AssertUnwindSafe(body));
    SUPPRESS_PANIC_OUTPUT.store(false, Ordering::SeqCst);
    match result {
        Ok(Ok(value)) => StageRun::Completed(value),
        Ok(Err(error)) => StageRun::Failed(format!("{error:#}")),
        Err(_) => StageRun::Panicked(take_captured_panic()),
    }
}

// ---------------------------------------------------------------------------
// Stage ledger
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum StageOutcome {
    /// The stage ran to completion and processed `items` units of work.
    Completed {
        items: usize,
    },
    Failed(String),
    Panicked(String),
    /// The pipeline stopped before this stage could run.
    NotReached,
}

#[derive(Debug, Default)]
struct StageLedger {
    entries: Vec<(&'static str, StageOutcome)>,
}

impl StageLedger {
    fn completed(&mut self, stage: &'static str, items: usize) {
        self.entries.push((stage, StageOutcome::Completed { items }));
    }

    fn failed(&mut self, stage: &'static str, message: String) {
        self.entries.push((stage, StageOutcome::Failed(message)));
    }

    fn panicked(&mut self, stage: &'static str, message: String) {
        self.entries.push((stage, StageOutcome::Panicked(message)));
    }

    fn not_reached(&mut self, stages: &[&'static str]) {
        for stage in stages {
            if !self.entries.iter().any(|(name, _)| name == stage) {
                self.entries.push((*stage, StageOutcome::NotReached));
            }
        }
    }

    fn outcome_of(&self, stage: &str) -> Option<&StageOutcome> {
        self.entries
            .iter()
            .find(|(name, _)| *name == stage)
            .map(|(_, outcome)| outcome)
    }

    fn summary(&self) -> String {
        self.entries
            .iter()
            .map(|(name, outcome)| match outcome {
                StageOutcome::Completed { items } => format!("  {name}: completed, {items} item(s)"),
                StageOutcome::Failed(message) => format!("  {name}: FAILED — {message}"),
                StageOutcome::Panicked(message) => format!("  {name}: PANICKED — {message}"),
                StageOutcome::NotReached => format!("  {name}: not reached"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every way this ledger can condemn a run, each named distinctly so the
    /// failure text says which kind of defect was found.
    fn verdict(&self) -> Result<(), String> {
        let mut problems: Vec<String> = Vec::new();
        for (name, outcome) in &self.entries {
            match outcome {
                StageOutcome::Panicked(message) => problems.push(format!(
                    "PIPELINE PANIC: stage `{name}` aborted the process instead of reporting a failure. \
                     A panic here kills every later stage, emits no ERROR event, and leaves the consumer \
                     with a raw backtrace. Payload: {message}"
                )),
                StageOutcome::Failed(message) => {
                    problems.push(format!("PIPELINE ERROR: stage `{name}` returned Err: {message}"));
                }
                StageOutcome::NotReached => problems.push(format!(
                    "PIPELINE STAGE NOT REACHED: `{name}` never ran because an earlier stage stopped the run."
                )),
                StageOutcome::Completed { items: 0 } => problems.push(format!(
                    "PIPELINE STAGE DID NO WORK: stage `{name}` completed but processed 0 items. \
                     \"generated 0 files\" and \"generated nothing because it never really ran\" are \
                     indistinguishable from the outside, so this gate treats a zero-item stage as a failure."
                )),
                StageOutcome::Completed { .. } => {}
            }
        }
        if problems.is_empty() {
            return Ok(());
        }
        Err(format!("{}\n\nStage ledger:\n{}", problems.join("\n"), self.summary()))
    }
}

// ---------------------------------------------------------------------------
// ERROR-event capture
// ---------------------------------------------------------------------------

/// A stage can fail by *logging* rather than by returning: the docs identifier
/// gate, for one, now reports a rejected identifier as a single ERROR event and
/// lets the run continue. A gate that inspects only `Result`s would call that run
/// green and hand the consumer a broken page.
///
/// Hand-rolled rather than `tracing_test::traced_test` on purpose: that macro
/// installs a thread-local subscriber filtered to the *test* crate's name, which
/// would capture neither `alef`'s own targets nor anything emitted from the rayon
/// worker threads several generation stages fan out onto — a capture that sees
/// nothing is indistinguishable from a run that logged nothing.
#[derive(Clone, Default)]
struct ErrorEventCollector {
    events: Arc<Mutex<Vec<String>>>,
}

impl ErrorEventCollector {
    fn clear(&self) {
        self.events.lock().unwrap_or_else(|error| error.into_inner()).clear();
    }

    fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl tracing::Subscriber for ErrorEventCollector {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::ERROR
    }

    fn new_span(&self, _attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() != tracing::Level::ERROR {
            return;
        }
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let target = event.metadata().target().to_string();
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(format!("[{target}]{}", visitor.0));
    }

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

static ERROR_EVENTS: OnceLock<ErrorEventCollector> = OnceLock::new();

fn error_event_collector() -> &'static ErrorEventCollector {
    ERROR_EVENTS.get_or_init(|| {
        let collector = ErrorEventCollector::default();
        // A failure here means another test in this binary already installed a
        // subscriber; the collector would then silently observe nothing, so it is
        // a hard error rather than a warning. ~keep
        tracing::subscriber::set_global_default(collector.clone())
            .expect("install the gate's ERROR-event collector as the global subscriber");
        collector
    })
}

// ---------------------------------------------------------------------------
// cwd isolation
// ---------------------------------------------------------------------------

/// `pipeline::extract` writes its IR cache to `.alef/<crate>/` resolved against the
/// process CWD, and the ownership guard reads `.alef-ownership.toml` the same way,
/// so the fixture workspace has to *be* the CWD or the run reads and writes alef's
/// own repository. CWD is process-global, hence the lock.
static CWD_LOCK: Mutex<()> = Mutex::new(());

struct WorkspaceCwd {
    _lock: MutexGuard<'static, ()>,
    original: PathBuf,
}

impl WorkspaceCwd {
    fn enter(root: &Path) -> Self {
        let lock = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original = std::env::current_dir().expect("read current directory");
        std::env::set_current_dir(root).expect("enter fixture workspace");
        Self { _lock: lock, original }
    }
}

impl Drop for WorkspaceCwd {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

// ---------------------------------------------------------------------------
// The pipeline drive
// ---------------------------------------------------------------------------

/// Every stage this gate drives, in pipeline order. Listed separately from the
/// drive itself so a stage that is never reached is reported by name rather than
/// silently absent from the ledger.
const DRIVEN_STAGES: &[&str] = &[
    "extract",
    "snippet-coverage-preflight",
    "generate-bindings",
    "write-bindings",
    "scaffold",
    "stubs",
    "public-api",
    "e2e-call-export-validation",
    "e2e-codegen",
    "readme",
    "docs",
];

#[derive(Default)]
struct PipelineRun {
    ledger: StageLedger,
    /// Every path the pipeline reported as written, across all writing stages.
    written_paths: Vec<PathBuf>,
    /// Paths the ownership guard declined, folded across writing stages the way
    /// `alef all` folds them. A set, not a list: more than one stage can target
    /// the same path and a repeated refusal is one frozen file, not two.
    refused_paths: std::collections::BTreeSet<PathBuf>,
    /// Paths seeded with unmarked content before the first write, to exercise the
    /// ownership guard.
    seeded_paths: Vec<PathBuf>,
    binding_contents: Vec<(PathBuf, String)>,
    api_function_names: Vec<String>,
    api_opaque_constructors: Vec<String>,
    e2e_paths: Vec<PathBuf>,
}

macro_rules! stage {
    ($run:expr, $name:literal, $body:expr) => {
        match run_stage(|| $body) {
            StageRun::Completed(value) => value,
            StageRun::Failed(message) => {
                $run.ledger.failed($name, message);
                $run.ledger.not_reached(DRIVEN_STAGES);
                return;
            }
            StageRun::Panicked(message) => {
                $run.ledger.panicked($name, message);
                $run.ledger.not_reached(DRIVEN_STAGES);
                return;
            }
        }
    };
}

fn drive_pipeline(root: &Path, config_path: &Path, resolved: &ResolvedCrateConfig, run: &mut PipelineRun) {
    let languages: Vec<Language> = resolved.languages.clone();

    // --- extract -----------------------------------------------------------
    let api = stage!(run, "extract", pipeline::extract(resolved, config_path, true));
    run.ledger
        .completed("extract", api.types.len() + api.enums.len() + api.functions.len());
    run.api_function_names = api.functions.iter().map(|function| function.name.clone()).collect();
    run.api_opaque_constructors = api
        .types
        .iter()
        .filter(|type_def| type_def.is_opaque)
        .flat_map(|type_def| {
            type_def
                .methods
                .iter()
                .map(move |method| format!("{}::{}", type_def.name, method.name))
        })
        .collect();

    let Some(e2e_config) = resolved.e2e.clone() else {
        run.ledger
            .failed("snippet-coverage-preflight", "fixture declares no [crates.e2e]".into());
        run.ledger.not_reached(DRIVEN_STAGES);
        return;
    };

    // --- stage 0: snippet coverage preflight -------------------------------
    // This is what xberg died on: a codegen predicate reused as an availability
    // gate rejected every trait-bridge registry function, and zero files were
    // written. The `clear_validators` fixture routes through the bridge's
    // `clear_fn` with no wasm `client_factory` override, which is exactly the
    // input that reaches the availability check. ~keep
    let coverage = stage!(
        run,
        "snippet-coverage-preflight",
        alef::e2e::evaluate_snippet_coverage(resolved, &e2e_config, &api.types, &api.enums, &api.functions).and_then(
            |coverage| {
                let coverage = coverage.ok_or_else(|| {
                    anyhow::anyhow!("fixture configures [crates.e2e.snippets] but no coverage ledger was produced")
                })?;
                alef::e2e::ensure_fresh_snippet_coverage_complete(&coverage)?;
                Ok(coverage)
            }
        )
    );
    run.ledger
        .completed("snippet-coverage-preflight", coverage.generated.len());

    // --- bindings ----------------------------------------------------------
    let bindings = stage!(
        run,
        "generate-bindings",
        pipeline::generate(&api, resolved, &languages, true, config_path, false)
    );
    let binding_file_count: usize = bindings.iter().map(|(_, files)| files.len()).sum();
    run.ledger.completed("generate-bindings", binding_file_count);
    for (_, files) in &bindings {
        for file in files {
            run.binding_contents.push((file.path.clone(), file.content.clone()));
        }
    }

    // Seed pre-existing, unmarked files at two paths the pipeline is about to
    // write, so the ownership guard has something to refuse and the refusal count
    // is an asserted number rather than an unobserved one.
    run.seeded_paths = seed_unmarked_files(root, &bindings, 2);

    // --- write bindings (ownership guard) ----------------------------------
    let write_report = stage!(run, "write-bindings", pipeline::write_files_report(&bindings, root));
    run.ledger.completed("write-bindings", write_report.changed_count());
    absorb(run, &write_report);

    // --- scaffold ----------------------------------------------------------
    let scaffold_files = stage!(
        run,
        "scaffold",
        pipeline::scaffold(&api, resolved, &languages, config_path)
    );
    let scaffold_report = stage!(
        run,
        "scaffold",
        pipeline::write_scaffold_files_report(&scaffold_files, root, true)
    );
    run.ledger.completed("scaffold", scaffold_report.changed_count());
    absorb(run, &scaffold_report);

    // --- stubs -------------------------------------------------------------
    let stubs = stage!(run, "stubs", pipeline::generate_stubs(&api, resolved, &languages));
    let stub_report = stage!(run, "stubs", pipeline::write_files_report(&stubs, root));
    run.ledger.completed("stubs", stub_report.changed_count());
    absorb(run, &stub_report);

    // --- public API --------------------------------------------------------
    let public_api = stage!(
        run,
        "public-api",
        pipeline::generate_public_api(&api, resolved, &languages, config_path)
    );
    let public_api_report = stage!(run, "public-api", pipeline::write_files_report(&public_api, root));
    run.ledger.completed("public-api", public_api_report.changed_count());
    absorb(run, &public_api_report);

    // --- e2e call export validation ----------------------------------------
    // Reached by the call whose base `function` is empty and whose per-language
    // overrides supply differently-cased spellings: the base-empty call must be
    // skipped here rather than reported as a missing export.
    let validated = stage!(run, "e2e-call-export-validation", {
        let mut checked = 0usize;
        let all_calls = std::iter::once(("_default", &e2e_config.call))
            .chain(e2e_config.calls.iter().map(|(name, call)| (name.as_str(), call)));
        for (call_name, call_config) in all_calls {
            if call_config.function.is_empty() || call_config.module.is_empty() {
                continue;
            }
            let module_path = call_config.module.replace('-', "_");
            match alef::extract::validate_call_export(&api, &module_path, &call_config.function) {
                alef::extract::ExportValidation::Ok => checked += 1,
                other => {
                    anyhow::bail!("e2e call '{call_name}' did not validate: {other:?}");
                }
            }
        }
        Ok::<usize, anyhow::Error>(checked)
    });
    run.ledger.completed("e2e-call-export-validation", validated);

    // --- e2e codegen -------------------------------------------------------
    // This is where crawlberg died: the C generator walks the *resolved* field
    // path of `metadata.title`, which the `[crates.e2e.fields]` alias expands
    // across a namespace hop, and needs a declared C type for every intermediate.
    //
    // `generate_e2e` isolates one backend's codegen failure from its siblings: it no
    // longer returns `Err` for a generator failure, it returns the files every other
    // backend still produced alongside the failure in a second slot. This gate still
    // needs the stage to read as failed -- the assertions below check for exactly that --
    // so the deferred error is turned into a `StageOutcome::Failed` by hand instead of
    // the `stage!` macro's `Err` arm doing it, right after the files that did generate
    // are written (mirroring what the real fix does at every caller: write what
    // succeeded, then report the failure). ~keep
    let (e2e_files, e2e_generator_error) = stage!(
        run,
        "e2e-codegen",
        alef::e2e::generate_e2e(
            resolved,
            &e2e_config,
            None,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
    );
    run.e2e_paths = e2e_files.iter().map(|file| file.path.clone()).collect();
    let e2e_written = stage!(
        run,
        "e2e-codegen",
        pipeline::write_scaffold_files_with_overwrite(&e2e_files, root, true)
    );
    for file in &e2e_files {
        run.written_paths.push(root.join(&file.path));
    }
    if let Some(error) = e2e_generator_error {
        run.ledger.failed("e2e-codegen", format!("{error:#}"));
        run.ledger.not_reached(DRIVEN_STAGES);
        return;
    }
    run.ledger.completed("e2e-codegen", e2e_written);

    // --- readme ------------------------------------------------------------
    let readme_files = stage!(run, "readme", pipeline::readme(&api, resolved, &languages));
    let readme_written = stage!(
        run,
        "readme",
        pipeline::write_scaffold_files_with_overwrite(&readme_files, root, true)
    );
    run.ledger.completed("readme", readme_written);
    for file in &readme_files {
        run.written_paths.push(root.join(&file.path));
    }

    // --- docs --------------------------------------------------------------
    // Reached by `Client::new`: a `pub fn new` on an opaque type renders as
    // `static new(...)` in TypeScript, which is what the docs identifier gate
    // rejected outright in tree-sitter-language-pack. `generate_docs_stage`
    // returns its pages even when a later step fails, so the pages are written
    // before the result is propagated — same ordering `alef all` uses. ~keep
    let docs_outcome = run_stage(|| {
        let (files, result) = alef::docs::generate_docs_stage(&api, resolved, &languages, None, root);
        let written = pipeline::write_scaffold_files_with_overwrite(&files, root, true)?;
        result?;
        Ok::<(usize, Vec<PathBuf>), anyhow::Error>((written, files.iter().map(|file| root.join(&file.path)).collect()))
    });
    match docs_outcome {
        StageRun::Completed((written, paths)) => {
            run.ledger.completed("docs", written);
            run.written_paths.extend(paths);
        }
        StageRun::Failed(message) => run.ledger.failed("docs", message),
        StageRun::Panicked(message) => run.ledger.panicked("docs", message),
    }

    run.ledger.not_reached(DRIVEN_STAGES);
}

fn absorb(run: &mut PipelineRun, report: &pipeline::WriteReport) {
    run.written_paths.extend(report.changed_paths.iter().cloned());
    run.refused_paths.extend(report.refused_paths.iter().cloned());
}

/// Write unmarked, non-alef content to the first `count` distinct generated text
/// paths, so the ownership guard has a concrete, known set to refuse.
fn seed_unmarked_files(root: &Path, bindings: &[(Language, Vec<GeneratedFile>)], count: usize) -> Vec<PathBuf> {
    let mut seeded = Vec::new();
    for (_, files) in bindings {
        for file in files {
            if seeded.len() == count {
                return seeded;
            }
            if !file.path.extension().is_some_and(|extension| extension == "rs") {
                continue;
            }
            let full_path = root.join(&file.path);
            let Some(parent) = full_path.parent() else { continue };
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
            let body = "// hand written by a human, never touched by alef\nfn untouched() {}\n";
            if std::fs::write(&full_path, body).is_ok() {
                seeded.push(full_path);
            }
        }
    }
    seeded
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

fn run_gate(sabotage: Sabotage) -> (PipelineRun, Vec<String>, tempfile::TempDir) {
    let collector = error_event_collector();
    collector.clear();
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    // Canonicalised because macOS hands back a `/var` symlink for `/private/var`,
    // and every path this gate compares is built from `std::env::current_dir`. ~keep
    let root = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());
    write_fixture_workspace(&root, sabotage);

    let _cwd = WorkspaceCwd::enter(&root);
    let config_path = root.join("alef.toml");
    let config_text = std::fs::read_to_string(&config_path).expect("read fixture alef.toml");
    let raw: NewAlefConfig = toml::from_str(&config_text).expect("fixture alef.toml must parse");
    let resolved = raw.resolve().expect("fixture alef.toml must resolve").remove(0);
    // `load_config` validates every resolved crate before any stage runs, so the gate must too --
    // otherwise it drives a config the real CLI would have rejected, and a config-shaped defect
    // reaches a backend here that could never reach one in production. ~keep
    alef::core::config::validation::validate_resolved(&resolved).expect("fixture alef.toml must validate");

    let mut run = PipelineRun::default();
    drive_pipeline(&root, &config_path, &resolved, &mut run);
    (run, collector.drain(), workspace)
}

#[test]
fn generation_pipeline_completes_over_a_representative_consumer_surface() {
    let (run, error_events, _workspace) = run_gate(Sabotage::None);

    // 1. No panic, and no stage-level error — reported as distinct classes.
    if let Err(report) = run.ledger.verdict() {
        panic!("the generation pipeline did not complete:\n{report}");
    }

    // 2. The artifacts the pipeline claims it wrote must actually be on disk.
    assert!(
        !run.written_paths.is_empty(),
        "no stage reported writing a single file; an `Ok` result with nothing on disk is the \
         failure mode this gate exists to catch"
    );
    let missing: Vec<&PathBuf> = run.written_paths.iter().filter(|path| !path.is_file()).collect();
    assert!(
        missing.is_empty(),
        "the pipeline reported writing {} file(s) that are not on disk: {missing:?}",
        missing.len()
    );

    // 3. The ownership guard refused exactly the files that were seeded unmarked,
    //    and refused them by name — not merely "some number greater than zero".
    assert_eq!(
        run.seeded_paths.len(),
        2,
        "the fixture must seed two unmarked files for the ownership guard; seeded {:?}",
        run.seeded_paths
    );
    for seeded in &run.seeded_paths {
        assert!(
            run.refused_paths.contains(seeded),
            "the ownership guard did not refuse the unmarked pre-existing file {}; refused: {:?}",
            seeded.display(),
            run.refused_paths
        );
    }
    assert_eq!(
        run.refused_paths.len(),
        run.seeded_paths.len(),
        "the ownership guard refused paths beyond the seeded ones: {:?}",
        run.refused_paths
    );

    // 4. Reach probes: each fixture element actually landed in the surface that
    //    carries it to the stage it is there to exercise.
    assert!(
        run.api_function_names.iter().any(|name| name == "probe_transport"),
        "the cfg-split export did not survive extraction, so the FFI backend never saw a \
         predicate to union; extracted functions: {:?}",
        run.api_function_names
    );
    assert!(
        run.binding_contents.iter().any(
            |(path, content)| path.extension().is_some_and(|extension| extension == "rs")
                && content.contains("probe_transport")
        ),
        "no generated Rust binding mentions the cfg-split export"
    );
    assert!(
        run.api_opaque_constructors.iter().any(|name| name == "Client::new"),
        "the `pub fn new` constructor on the opaque type was dropped during extraction, so the \
         docs identifier gate was never reached; opaque methods: {:?}",
        run.api_opaque_constructors
    );
    assert!(
        run.e2e_paths
            .iter()
            .any(|path| path.components().any(|component| component.as_os_str() == "c")),
        "the e2e stage generated no C suite, so the nested alias-crossing assertion path was \
         never walked; e2e paths: {:?}",
        run.e2e_paths
    );

    // 5. A stage may fail by logging rather than by returning: the docs identifier
    //    gate now reports violations as a single ERROR event and lets the run
    //    continue, so a green `Result` alone would hide it.
    assert!(
        error_events.is_empty(),
        "the pipeline completed but emitted {} ERROR event(s); a stage that reports a defect by \
         logging still leaves the consumer with broken output:\n{}",
        error_events.len(),
        error_events
            .iter()
            .map(|event| format!("  {event}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------------------------------------------------------------------------
// Negative controls — the gate must be shown to fail when the pipeline is broken
// ---------------------------------------------------------------------------

/// Without this the panic/error split above is untested machinery: a harness that
/// silently reclassified panics as errors would still report the run green.
#[test]
fn a_panicking_stage_is_classified_apart_from_a_failing_one() {
    let panicked = run_stage(|| -> anyhow::Result<()> { panic!("synthetic stage panic") });
    match panicked {
        StageRun::Panicked(message) => assert!(
            message.contains("synthetic stage panic"),
            "the panic payload was not captured: {message}"
        ),
        other => panic!("expected StageRun::Panicked, got {other:?}"),
    }

    let failed = run_stage(|| -> anyhow::Result<()> { anyhow::bail!("synthetic stage error") });
    match failed {
        StageRun::Failed(message) => assert!(
            message.contains("synthetic stage error"),
            "the error context was not captured: {message}"
        ),
        other => panic!("expected StageRun::Failed, got {other:?}"),
    }

    let mut ledger = StageLedger::default();
    ledger.panicked("synthetic", "boom".into());
    let verdict = ledger.verdict().expect_err("a panicked stage must condemn the run");
    assert!(
        verdict.contains("PIPELINE PANIC"),
        "a panic must be reported under its own loud heading, not as an ordinary failure: {verdict}"
    );
}

/// A stage that returns `Ok` having processed nothing is the defect class this
/// codebase already ships (a header check returns "fresh" when it found no
/// exports at all). The gate must not reproduce it.
#[test]
fn a_stage_that_processed_nothing_is_not_reported_as_healthy() {
    let mut ledger = StageLedger::default();
    ledger.completed("synthetic", 0);
    let verdict = ledger
        .verdict()
        .expect_err("a stage that processed zero items must condemn the run");
    assert!(
        verdict.contains("PIPELINE STAGE DID NO WORK"),
        "a zero-item stage must be reported distinctly from a stage that failed: {verdict}"
    );

    let mut healthy = StageLedger::default();
    healthy.completed("synthetic", 1);
    healthy.verdict().expect("a stage that did work must pass");
}

/// A broken run must not be reported as healthy. Points the e2e call at a function the
/// crate does not export and requires some stage to condemn the run.
#[test]
fn gate_condemns_a_run_whose_call_names_an_unexported_function() {
    let (run, _error_events, _workspace) = run_gate(Sabotage::UnknownCallFunction);

    let verdict = run.ledger.verdict();
    assert!(
        verdict.is_err(),
        "a call naming an unexported function produced a healthy verdict; the gate would report a \
         broken pipeline as green.\n{}",
        run.ledger.summary()
    );
    let message = verdict.unwrap_err();
    assert!(
        message.contains("PIPELINE PANIC") || message.contains("PIPELINE ERROR") || message.contains("DID NO WORK"),
        "the sabotaged run was condemned, but not under a diagnosis a reader can act on: {message}\n{}",
        run.ledger.summary()
    );
}

/// A fixture assertion whose field path does not resolve must produce a visible outcome.
/// Removing the alias leaves `metadata.title` naming a field `Metadata` does not have; the
/// e2e-codegen stage must fail, and its message must name the type, the field, and the
/// alias that fixes it — not merely condemn the run.
#[test]
fn gate_condemns_a_run_whose_assertion_field_path_does_not_resolve() {
    let (run, _error_events, _workspace) = run_gate(Sabotage::MissingFieldAlias);

    let outcome = run
        .ledger
        .outcome_of("e2e-codegen")
        .expect("the gate drives an e2e-codegen stage");
    let StageOutcome::Failed(message) = outcome else {
        panic!(
            "an assertion on an unresolvable field path did not fail e2e codegen; it was silently \
             dropped or rendered against a phantom accessor. Got {outcome:?}\n{}",
            run.ledger.summary()
        );
    };
    assert!(
        message.contains("IR type `Metadata` has no field `title`"),
        "the failure must name the type and the field it lacks: {message}"
    );
    assert!(
        message.contains("\"metadata.title\" = \"metadata.document.title\""),
        "the failure must spell the alias that fixes it: {message}"
    );
}
