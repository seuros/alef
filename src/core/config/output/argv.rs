//! The typed argv representation for defaults alef generates itself.
//!
//! Split out of `output.rs` to keep that file under the repo's 1,000-line cap. `TestAppRunConfig`
//! and `CleanConfig` both hold an `Option<ArgvRunConfig>` alongside their shell-string field
//! (`run`/`clean`); see those structs' own doc comments in `output.rs` for the shell/argv split.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One argv-only step of an [`ArgvRunConfig`]: a program and its literal arguments, invoked
/// directly via `Command::new(command).args(args)` -- never through a shell. Each argument is
/// passed to the child process as a single opaque element, so shell metacharacters inside an
/// argument (`;`, backticks, `$(...)`, quotes) are inert: there is no shell parsing them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArgvStep {
    pub command: String,
    pub args: Vec<String>,
}

/// A sequence of argv-only steps sharing one working directory and one set of environment
/// variables -- the typed alternative to `StringOrVec` for run commands alef *generates*
/// itself and that must embed a config-supplied value (e.g. a Go module path) as a literal
/// argument. Never populated by hand-writing shell syntax: there is nothing to quote or
/// escape here, because no step is ever handed to a shell in the first place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArgvRunConfig {
    pub work_dir: String,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub steps: Vec<ArgvStep>,
}
