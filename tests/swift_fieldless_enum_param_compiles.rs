// Test module: diagnostic output to stdout/stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Regression test: a fieldless enum passed as a function/method parameter must produce a
//! swift crate that actually *compiles*.
//!
//! Before this fix, the swift backend emitted a call like
//! `client.0.analyze(&input, <toolkit::Mode as ::std::convert::From<String>>::from(mode))` for
//! `fn analyze(&self, input: String, mode: Mode) -> Result<String, String>`. Two independent
//! defects hid on that one line:
//!
//! 1. `<toolkit::Mode as ::std::convert::From<String>>::from(...)` assumes a `From<String>`
//!    impl on the consumer's own enum that alef never emits (and, being both a foreign trait
//!    and a foreign type from the swift crate's point of view, *cannot* legally emit -- it
//!    would be an orphan impl, E0117). It fails with E0277: `Mode: From<String>` is not
//!    satisfied.
//! 2. `&input` borrows a `String` parameter the core method takes by value. It fails with
//!    E0308: expected `String`, found `&String`.
//!
//! Both were invisible to text-assertion tests: the emitted line reads as plausible Rust. Only
//! `rustc` rejects it. This test therefore does not assert on emitted text -- it drives the real
//! `alef` binary over a fixture crate and runs `cargo build` over the emitted swift crate, the
//! same tool a consumer's build uses.
//!
//! # Why this fixture is not the multi-language gate fixture
//!
//! [`generated_output_downstream_gate`]'s fixture emits every language into one project and
//! relies on `[workspace.output_template]` defaults for `crates/{name}-ffi`. Swift's generated
//! `Cargo.toml` unconditionally depends on that FFI crate by a fixed relative path
//! (`../../../crates/{name}-ffi`, see `cargo_toml_depends_on_ffi_crate` in
//! `src/backends/swift/gen_rust_crate/cargo.rs`) purely so the FFI crate's `#[no_mangle]`
//! exports get linked into the final staticlib -- nothing in the swift-generated Rust actually
//! calls into it beyond one keep-linked probe function. This test hand-places a minimal stub
//! at that exact path instead of asking `ffi` to also generate: it is the smallest fixture that
//! reproduces the real dependency edge without dragging in the (separate, pre-existing)
//! question of what alef's default `output_template` should resolve the ffi crate to.
//!
//! Ignored like the other downstream-toolchain lanes ([`generated_output_downstream_gate`]):
//! it compiles a real crate against the Swift toolchain, too slow for the default
//! `cargo test --workspace` matrix. Run explicitly via
//! `cargo test --test swift_fieldless_enum_param_compiles -- --ignored`.

use std::path::Path;
use std::process::Command;

const FIXTURE_CORE_CARGO_TOML: &str = r#"[package]
name = "toolkit"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
"#;

const FIXTURE_CORE_SOURCE: &str = r#"
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    Fast,
    Thorough,
}

pub struct Session {
    token: String,
}

impl Session {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub fn analyze(&self, input: String, mode: Mode) -> Result<String, String> {
        let _ = (input, mode);
        Err("unimplemented".to_string())
    }
}
"#;

/// A trivial stand-in for the real `alef`-generated FFI crate. The swift crate's `Cargo.toml`
/// depends on `toolkit-ffi` purely to keep its `#[no_mangle]` exports linked into the final
/// staticlib (see the module doc), and the swift-generated `lib.rs` references exactly one
/// symbol from it -- a "keep linked" probe named `{crate}_version`. Nothing else about the real
/// FFI crate's content matters to this test, which is scoped to the swift crate's own generated
/// code.
const FIXTURE_FFI_CARGO_TOML: &str = r#"[package]
name = "toolkit-ffi"
version = "0.1.0"
edition = "2024"
"#;

const FIXTURE_FFI_SOURCE: &str = r#"
/// # Safety
/// Stub used only to satisfy the swift crate's keep-linked probe.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toolkit_version() -> *const std::ffi::c_char {
    static VERSION: &[u8] = b"0.1.0\0";
    VERSION.as_ptr().cast::<std::ffi::c_char>()
}
"#;

const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
alef_version = "__ALEF_VERSION__"
languages = ["swift"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.generate]
public_api = true
"#;

fn run(program: &str, args: &[&str], cwd: &Path) -> (bool, String) {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("running `{program} {}` in {}: {error}", args.join(" "), cwd.display()));
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

#[test]
#[ignore = "compiles a real swift crate against the Swift toolchain; run via \
            `cargo test --test swift_fieldless_enum_param_compiles -- --ignored`"]
fn emitted_swift_crate_compiles_with_a_fieldless_enum_parameter() {
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    let root = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());

    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_CORE_SOURCE).expect("write fixture core source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CORE_CARGO_TOML).expect("write fixture core Cargo.toml");

    std::fs::create_dir_all(root.join("crates/toolkit-ffi/src")).expect("create fixture ffi src directory");
    std::fs::write(root.join("crates/toolkit-ffi/src/lib.rs"), FIXTURE_FFI_SOURCE)
        .expect("write fixture ffi stub source");
    std::fs::write(root.join("crates/toolkit-ffi/Cargo.toml"), FIXTURE_FFI_CARGO_TOML)
        .expect("write fixture ffi stub Cargo.toml");

    let config = FIXTURE_ALEF_TOML.replace("__ALEF_VERSION__", env!("CARGO_PKG_VERSION"));
    std::fs::write(root.join("alef.toml"), config).expect("write fixture alef.toml");

    // `alef generate`'s swift post-build step itself runs `cargo build` over the crate it just
    // emitted (see the doc-comment on this test): a failing enum-parameter conversion surfaces
    // right here, as a failure of `alef generate` -- there is no separate "now compile it"
    // step to run afterward.
    let (passed, output) = run(env!("CARGO_BIN_EXE_alef"), &["generate"], &root);
    assert!(
        passed,
        "`alef generate` failed over the fieldless-enum-param fixture -- the emitted swift \
         crate did not compile:\n{output}"
    );
}
