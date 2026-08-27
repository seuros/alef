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

/// `Chunked` is gated on a feature this crate declares but does not default-enable (see
/// `FIXTURE_CARGO_TOML`'s `[features]` table) -- alef forwards it into every binding crate's own
/// manifest and turns it on by default there, so a clippy-lane binding crate always compiles
/// with it active. Regression coverage for two defects at once: (1) the forwarding row must
/// exist at all -- pyo3 previously declared no such row, so this gate's own `#[cfg(feature =
/// "chunking-tokenizers")]` re-emission tripped `unexpected_cfgs` under `-D warnings`; (2) the
/// resulting `From<Binding> for Mode` / `From<Mode> for Binding` match must NOT gain a trailing
/// `_ => Default::default()` merely because a variant is cfg-gated -- the arm carries the same
/// gate as the variant, so with the feature always active in this gate's own build the catch-all
/// is unreachable and trips `unreachable_patterns` under `-D warnings`. ~keep
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    Fast,
    Thorough,
    #[cfg(feature = "chunking-tokenizers")]
    Chunked,
}

/// A free function gated the same way, covering the function-position half of the same
/// forwarding requirement (`Mode::Chunked` above only covers the enum-variant position).
#[cfg(feature = "chunking-tokenizers")]
pub fn count_tokens(text: String) -> Result<u64, String> {
    Ok(text.len() as u64)
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
    Inline {
        mime_type: String,
        bytes_len: u64,
    },
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

/// No cast should be emitted on either side of the JNI boundary: `f64`'s JNI wire type
/// (`jni::sys::jdouble`) is a type alias for `f64` itself. Regression coverage for the bug
/// where JNI's `primitive_cast` (param unmarshalling) and `emit_return_marshal_with_indent`
/// (return marshalling) assumed every primitive needs a cast to its own wire type and cast an
/// already-`f64` value to `f64`, tripping `clippy::unnecessary_cast` under `-D warnings`
/// (alef commit c82f8f117). Unit coverage for the underlying helpers already existed; this
/// free function is what lets the live JNI crate this gate compiles actually contain the
/// generated call-site and return-site casts, so a regression here fails `cargo clippy`. ~keep
pub fn round_trip_cost(cost_usd: f64) -> f64 {
    cost_usd
}

/// The pointee type a capsule (host-native passthrough) function returns. Kept out of the
/// binding surface itself (`alef(skip)`) since only its fully-qualified path matters to the
/// FFI backend, mirroring how a real capsule pointee (e.g. tree-sitter's `TSLanguage`) lives
/// in a crate alef never parses. ~keep
#[cfg_attr(alef, alef(skip))]
pub struct RawLanguage {
    pub value: u64,
}

/// A capsule-configured type (see `[crates.ffi.capsule_types.Language]` in
/// `FIXTURE_ALEF_TOML`): the FFI backend returns `into_raw()`'s own pointer verbatim instead
/// of boxing it, so the exported C function's declared return and `into_raw()`'s real return
/// are the same `*const RawLanguage` by construction. Regression coverage for the bug where
/// `capsule_into_raw_expr` appended a redundant `as *const RawLanguage` to an
/// already-`*const RawLanguage` expression, tripping `clippy::unnecessary_cast` under
/// `-D warnings` (alef commit c82f8f117). Unit coverage for `capsule_into_raw_expr` already
/// existed; this type and `get_language` are what let the live FFI crate this gate compiles
/// actually contain the generated capsule return, so a regression here fails `cargo clippy`.
/// ~keep
pub struct Language {
    handle: u64,
}

impl Language {
    #[cfg_attr(alef, alef(skip))]
    pub fn into_raw(self) -> *const RawLanguage {
        Box::into_raw(Box::new(RawLanguage { value: self.handle })) as *const RawLanguage
    }
}

pub fn get_language(name: String) -> Result<Language, String> {
    if name.is_empty() {
        return Err("empty language name".to_string());
    }
    Ok(Language { handle: 1 })
}
"#;

// The fixture's own core crate derives serde, so it needs the dependency to compile. It went
// unnoticed until the core crate became reachable from the emitted binding crates: before that
// nothing ever built it, so an uncompilable fixture still passed every lane. ~keep
//
// `[lints.rust] unexpected_cfgs`'s `check-cfg` allowlist is required, not decorative: the
// fixture source above uses alef's own `#[cfg_attr(alef, alef(skip))]` exclusion marker (see
// `RawLanguage` and `Language::into_raw`), and `cfg(alef)` is never a real, declared cfg --
// there is no `alef` proc-macro crate this fixture depends on, so the attribute inside each
// `cfg_attr` never actually runs at real compile time. Without this allowlist, rustc's
// `unexpected_cfgs` lint fires on both call sites and the clippy lane's `-D warnings` denies
// it: this crate is a path dependency of every clippy-lane binding crate, so an unconfigured
// core manifest fails builds it does not otherwise participate in. `alef scaffold` patches
// this same allowlist into `[workspace.lints.rust]` when the root manifest already declares a
// `[workspace]` (see `cli::pipeline::workspace_lints`), but this fixture's root is a plain
// `[package]` manifest -- the common case for a small, pre-existing consumer crate -- so it
// carries the allowlist itself, exactly as a real consumer in that shape has to today. ~keep
pub(crate) const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"toolkit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                                              [features]\nchunking-tokenizers = []\n\n\
                                              [dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n\n\
                                              [lints.rust]\nunexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(alef)'] }\n";

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

[crates.ffi.capsule_types.Language]
into_raw_type = "toolkit::RawLanguage"
c_return_type = "RawLanguage"
"#;
