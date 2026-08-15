use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::config::extras::Language;

/// Custom modules that alef should declare (mod X;) but not generate.
/// These are hand-written modules imported by the generated lib.rs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomModulesConfig {
    #[serde(default)]
    pub python: Vec<String>,
    #[serde(default)]
    pub node: Vec<String>,
    #[serde(default)]
    pub ruby: Vec<String>,
    #[serde(default)]
    pub php: Vec<String>,
    #[serde(default)]
    pub elixir: Vec<String>,
    #[serde(default)]
    pub wasm: Vec<String>,
    /// Hand-written Rust modules to declare in the generated FFI `lib.rs` via
    /// `pub mod <name>;`. Alef requires `<name>.rs` or `<name>/mod.rs` to already
    /// exist under the FFI crate's `src/` directory before generation runs, and
    /// fails with a clear error naming the missing path if neither does — the
    /// module's *contents* are never generated, only the declaration.
    ///
    /// Declarations are emitted at the top of the file, in configured order,
    /// immediately after the generated imports. Only `pub mod` is emitted (no
    /// `pub use <name>::*;` re-export): a `#[no_mangle] extern "C" fn` is exported
    /// into the compiled artifact, and seen by cbindgen's header generation,
    /// regardless of the enclosing module's path, so flattening the names into
    /// crate-root scope isn't needed and would risk colliding with a generated
    /// item of the same name. Contrast with `[crates.wasm].custom_rust_modules`,
    /// which does add `pub use` because wasm-bindgen's glue needs the re-exported
    /// names in scope unqualified.
    ///
    /// This is the mechanism for compiling a hand-authored opaque-handle wrapper
    /// (e.g. around a core type marked `alef(skip)`) into the FFI crate: without
    /// an entry here, a hand-written sibling `.rs` file under the crate's `src/`
    /// is never reached from the generated crate root, so its `extern "C"`
    /// functions are silently absent from both the cdylib and the header.
    #[serde(default)]
    pub ffi: Vec<String>,
    #[serde(default)]
    pub go: Vec<String>,
    #[serde(default)]
    pub java: Vec<String>,
    #[serde(default)]
    pub csharp: Vec<String>,
    #[serde(default)]
    pub r: Vec<String>,
}

impl CustomModulesConfig {
    pub fn for_language(&self, lang: Language) -> &[String] {
        match lang {
            Language::Python => &self.python,
            Language::Node => &self.node,
            Language::Ruby => &self.ruby,
            Language::Php => &self.php,
            Language::Elixir => &self.elixir,
            Language::Wasm => &self.wasm,
            Language::Ffi => &self.ffi,
            Language::Go => &self.go,
            Language::Java => &self.java,
            Language::Csharp => &self.csharp,
            Language::R => &self.r,
            Language::Rust => &[],
            Language::Kotlin
            | Language::KotlinAndroid
            | Language::Swift
            | Language::Dart
            | Language::Gleam
            | Language::Zig
            | Language::Jni
            | Language::C => &[],
        }
    }
}

/// Custom classes/functions from hand-written modules to register in module init.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomRegistration {
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub init_calls: Vec<String>,
}

/// Per-language custom registrations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomRegistrationsConfig {
    #[serde(default)]
    pub python: Option<CustomRegistration>,
    #[serde(default)]
    pub node: Option<CustomRegistration>,
    #[serde(default)]
    pub ruby: Option<CustomRegistration>,
    #[serde(default)]
    pub php: Option<CustomRegistration>,
    #[serde(default)]
    pub elixir: Option<CustomRegistration>,
    #[serde(default)]
    pub wasm: Option<CustomRegistration>,
}

impl CustomRegistrationsConfig {
    pub fn for_language(&self, lang: Language) -> Option<&CustomRegistration> {
        match lang {
            Language::Python => self.python.as_ref(),
            Language::Node => self.node.as_ref(),
            Language::Ruby => self.ruby.as_ref(),
            Language::Php => self.php.as_ref(),
            Language::Elixir => self.elixir.as_ref(),
            Language::Wasm => self.wasm.as_ref(),
            _ => None,
        }
    }
}
