//! Every e2e backend must resolve an unset `result_var` to the documented default.
//!
//! `[e2e.call].result_var` carries a serde default of `"result"`, so *omitting* the key always
//! produces a name. Writing it blank — `result_var = ""` — does not: `#[serde(default)]` fires on
//! an absent key, never on a present-but-empty one, and nothing validates the value. A blank name
//! spliced straight into a binding emits `val  = Sample.process()` / `let  = ...`: a binding with
//! no identifier, which no target language parses.
//!
//! Four emitters had each re-derived a local `if result_var.is_empty() { "result" }` fallback and
//! the rest had not, so the same call rendered under two different names depending on which
//! emitter you read. `CallConfig::effective_result_var` is now the only place that rule lives.
//!
//! These tests iterate [`all_generators`] rather than naming languages, so a backend added later
//! is covered on the day it is added rather than being silently exempt. Both emission axes are
//! driven — `render_snippet_body` (documentation snippets) and `generate` (the e2e project) —
//! because the raw reads were spread across both, and a snippet-only harness would have declared
//! the test-file emitters healthy without looking at them.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, all_generators};
use alef::e2e::fixture::{Fixture, FixtureGroup};

/// A name no emitter, template or crate name could produce on its own, so finding it in the
/// output proves the configured value reached the binding rather than merely coinciding with it.
const PROBE_RESULT_VAR: &str = "capturedValue";

/// Blank bindings, spelled per language keyword. A healthy emitter never produces one; before the
/// fix, an unset `result_var` produced exactly these.
const BLANK_BINDINGS: &[&str] = &["let  =", "val  =", "var  =", "const  =", "final  =", "auto  ="];

/// Anti-vacuity floors, deliberately well under the real counts.
///
/// A harness that renders nothing passes every equality assertion it makes, which is how the
/// original defect survived: each module's fixtures supplied the value the code failed to
/// default, so nothing ever exercised the blank case. These floors make "examined nothing"
/// a failure rather than a pass. The observed counts are printed on failure. ~keep
const MINIMUM_EXERCISED_RENDERS: usize = 6;
const MINIMUM_LANGUAGES_NAMING_A_RESULT: usize = 3;

/// The TOML a real `alef.toml` would carry, with the `result_var` line under test spliced in.
///
/// Going through `NewAlefConfig` rather than building a `CallConfig` by hand is what makes the
/// blank arm meaningful: it proves the blank survives config loading, which is the only way it
/// reaches an emitter in production.
fn config_for(result_var_line: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "MyLib"
{result_var_line}
result_is_simple = true
async = false
returns_result = true
"#
    );
    let config: NewAlefConfig = toml::from_str(&toml_src).expect("the e2e config under test must parse");
    let e2e = config.crates[0]
        .e2e
        .clone()
        .expect("the crate declares an [e2e] section");
    let resolved = config.resolve().expect("the config under test must resolve").remove(0);
    (e2e, resolved)
}

fn probe_fixture() -> Fixture {
    Fixture {
        id: "widget_lookup".to_string(),
        category: Some("widgets".to_string()),
        description: "look up a widget".to_string(),
        input: serde_json::json!({}),
        assertions: serde_json::from_value(serde_json::json!([{ "type": "not_error" }]))
            .expect("the probe assertion must deserialize"),
        ..Fixture::default()
    }
}

/// One backend's output for one `result_var` setting, or the message it failed with.
///
/// A backend that cannot render this minimal fixture is not a failure of the rule under test —
/// but it must fail the *same way* whether the name is blank or defaulted, so the failure is
/// compared rather than skipped.
#[derive(Debug, PartialEq, Eq)]
enum Rendered {
    Emitted(String),
    Failed(String),
}

fn render_snippet(generator: &dyn E2eCodegen, result_var_line: &str) -> Rendered {
    let (e2e, resolved) = config_for(result_var_line);
    match generator.render_snippet_body(&probe_fixture(), &e2e, &resolved, &[], &[]) {
        Ok(body) => Rendered::Emitted(body),
        Err(error) => Rendered::Failed(format!("{error:#}")),
    }
}

fn render_project(generator: &dyn E2eCodegen, result_var_line: &str) -> Rendered {
    let (e2e, resolved) = config_for(result_var_line);
    let groups = vec![FixtureGroup {
        category: "widgets".to_string(),
        fixtures: vec![probe_fixture()],
    }];
    match generator.generate(&groups, &e2e, &resolved, &[], &[], &[], &[]) {
        Ok(files) => Rendered::Emitted(
            files
                .iter()
                .map(|file| format!("{}\n{}", file.path.display(), file.content))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Err(error) => Rendered::Failed(format!("{error:#}")),
    }
}

type Renderer = fn(&dyn E2eCodegen, &str) -> Rendered;

/// The two emission axes, each named for the failure message.
fn axes() -> [(&'static str, Renderer); 2] {
    [("snippet", render_snippet as Renderer), ("project", render_project)]
}

#[test]
fn a_blank_result_var_emits_the_same_source_as_the_documented_default_in_every_language() {
    let generators = all_generators();
    assert!(
        generators.len() >= 20,
        "the backend list returned only {} generators — the sweep is broken, and a sweep over \
         nothing passes for a healthy tree",
        generators.len()
    );

    let mut exercised: Vec<String> = Vec::new();
    let mut compared = 0_usize;

    for generator in &generators {
        let language = generator.language_name();
        for (axis, render) in axes() {
            let blank = render(generator.as_ref(), "result_var = \"\"");
            let defaulted = render(generator.as_ref(), "");
            compared += 1;

            // Rendering the same config twice separates "the blank name changed the output"
            // from "this backend is not deterministic" — otherwise a backend that reorders its
            // own output would be reported as a `result_var` regression. ~keep
            assert_eq!(
                render(generator.as_ref(), ""),
                defaulted,
                "{language} ({axis}): two renders of one config disagree, so this backend is not \
                 deterministic and the comparison below cannot attribute a difference to \
                 `result_var`"
            );

            match (&blank, &defaulted) {
                (Rendered::Emitted(blank_output), Rendered::Emitted(default_output)) => {
                    assert_eq!(
                        blank_output, default_output,
                        "{language} ({axis}): a blank `result_var` must emit exactly what the \
                         documented default emits"
                    );
                    assert!(
                        !blank_output.trim().is_empty(),
                        "{language} ({axis}): rendered nothing, so the comparison above compared \
                         two empty strings"
                    );
                    for blank_binding in BLANK_BINDINGS {
                        assert!(
                            !blank_output.contains(blank_binding),
                            "{language} ({axis}): emitted `{blank_binding}` — a binding with no \
                             identifier, which is not valid source in any target language"
                        );
                    }
                    exercised.push(format!("{language}/{axis}"));
                }
                (Rendered::Failed(blank_error), Rendered::Failed(default_error)) => {
                    assert_eq!(
                        blank_error, default_error,
                        "{language} ({axis}): a blank `result_var` must fail the same way the \
                         documented default fails"
                    );
                }
                _ => panic!(
                    "{language} ({axis}): a blank `result_var` and the documented default \
                     disagreed about whether generation succeeds at all.\n  blank: {blank:?}\n  \
                     default: {defaulted:?}"
                ),
            }
        }
    }

    assert_eq!(
        compared,
        generators.len() * 2,
        "every backend must be compared on both emission axes"
    );
    assert!(
        exercised.len() >= MINIMUM_EXERCISED_RENDERS,
        "only {} render(s) produced output ({exercised:?}); below the anti-vacuity floor of {}, \
         the equality assertions above are comparing failures rather than generated source",
        exercised.len(),
        MINIMUM_EXERCISED_RENDERS
    );
}

/// The control: resolving a blank name must not become "overwrite the name someone chose".
///
/// It doubles as the harness's own sensitivity check. The test above asserts two renders are
/// equal, which a harness that ignores `result_var` entirely would also satisfy; this one proves
/// the rendered output does move when the configured name moves, so that equality means
/// something.
#[test]
fn an_explicit_result_var_is_emitted_verbatim_wherever_a_language_names_one() {
    let generators = all_generators();
    let mut naming: Vec<String> = Vec::new();

    for generator in &generators {
        let language = generator.language_name();
        for (axis, render) in axes() {
            let Rendered::Emitted(probed) = render(generator.as_ref(), &format!("result_var = \"{PROBE_RESULT_VAR}\""))
            else {
                continue;
            };
            let Rendered::Emitted(defaulted) = render(generator.as_ref(), "") else {
                continue;
            };
            if probed == defaulted {
                // This backend does not name its result on this axis — a CLI-shaped backend, or
                // one that discards an unreferenced return value. Nothing to honour verbatim.
                continue;
            }
            assert!(
                probed.contains(PROBE_RESULT_VAR),
                "{language} ({axis}): the output changed with the configured `result_var` but \
                 does not contain `{PROBE_RESULT_VAR}`, so the name was transformed rather than \
                 honoured verbatim"
            );
            assert!(
                !defaulted.contains(PROBE_RESULT_VAR),
                "{language} ({axis}): emitted `{PROBE_RESULT_VAR}` without it being configured"
            );
            naming.push(format!("{language}/{axis}"));
        }
    }

    assert!(
        naming.len() >= MINIMUM_LANGUAGES_NAMING_A_RESULT,
        "only {} render(s) named the configured result variable ({naming:?}); below the \
         anti-vacuity floor of {}, nothing here proves the harness can see a `result_var` change \
         at all",
        naming.len(),
        MINIMUM_LANGUAGES_NAMING_A_RESULT
    );
}
