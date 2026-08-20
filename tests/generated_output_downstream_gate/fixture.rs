//! The synthetic consumer crate that `generated_output_downstream_gate` emits bindings for.
//!
//! Split out of the parent file to keep it under the repo's 1,000-line cap for
//! `tests/**/*.rs`; the parent `mod`s this in the same way it already `mod`s
//! `poly_fmt_exclusions`.

// ---------------------------------------------------------------------------
// Fixture: a synthetic consumer, deliberately nobody's real crate
// ---------------------------------------------------------------------------

pub(crate) const FIXTURE_SOURCE: &str = r#"
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Segment {
    pub index: u32,
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub total: u64,
    pub segments: Vec<Segment>,
    pub attachment: Attachment,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    Fast,
    Thorough,
}

/// A data-carrying enum, covering both variant shapes swift's `from_string` reconstruction
/// helper cannot handle: a wire string carries only a variant's discriminant, never its
/// field data. Regression coverage for the bug where `alef generate` emitted a bare
/// `EnumName::Variant` path for every variant of every enum regardless of fields, which
/// does not type-check against a tuple or struct variant (E0308 / E0533). ~keep
#[derive(Clone, Default, Serialize, Deserialize)]
pub enum Attachment {
    #[default]
    None,
    Url(String),
    Inline { mime_type: String, bytes_len: u64 },
}

/// No public fields, so the extractor keeps this opaque and every backend has to emit a
/// handle type for it. The handle is what the JNI emitter turns into a `jlong` round
/// trip — the surface the redundant-cast regression lived on. ~keep
pub struct Session {
    token: String,
}

impl Session {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub fn token(&self) -> String {
        self.token.clone()
    }

    pub fn analyze(&self, input: String, mode: Mode) -> Result<Report, String> {
        let _ = (input, mode);
        Err("unimplemented".to_string())
    }
}

pub fn summarize(input: String) -> Result<Report, String> {
    let _ = input;
    Err("unimplemented".to_string())
}
"#;

// The fixture's own core crate derives serde, so it needs the dependency to compile. It went
// unnoticed until the core crate became reachable from the emitted binding crates: before that
// nothing ever built it, so an uncompilable fixture still passed every lane. ~keep
pub(crate) const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"toolkit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n";

// `java` and `elixir` scaffolders bail `alef generate` outright when repository/license/
// authors are unset (`scaffold::languages::java`, `scaffold::languages::elixir`), and both
// are gate languages, so this metadata is required for `alef generate` to succeed over the
// fixture at all -- not merely to exercise a "configured" code path. ~keep
pub(crate) const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
alef_version = "__ALEF_VERSION__"
languages = [__LANGUAGES__]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.generate]
public_api = true

[crates.package_metadata]
repository = "https://github.com/example/toolkit"
license = "MIT"
authors = ["Example Author <author@example.invalid>"]
"#;
