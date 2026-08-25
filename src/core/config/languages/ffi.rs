use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single capsule (Language-passthrough) type at the C-ABI layer.
///
/// When a Rust type is listed in [`FfiConfig::capsule_types`], the C FFI backend
/// does NOT box it into an opaque `*mut {Type}` handle. Instead, the generated C
/// function returns the host ecosystem's native grammar pointer directly by calling
/// `value.into_raw()` and casting the result to `*const {c_return_type}`. The
/// matching opaque `_new`/`_free`/`_to_json` symbols are suppressed for that type.
///
/// This is the load-bearing layer consumed by every C-ABI binding (Go, Java, C#,
/// Swift, Dart, Zig, Kotlin Android): each of those constructs its own host-native
/// `Language` wrapper from this raw pointer.
///
/// Both `into_raw_type` and `c_return_type` are **required** — there are no
/// defaults. A missing or empty value is a deserialization error (for required
/// fields) or an emission-time error (for fields validated later).
///
/// TOML form:
/// ```toml
/// [crates.ffi.capsule_types.Language]
/// into_raw_type = "my_crate::ffi::MyRawType"
/// c_return_type = "MyRawType"
/// package = "my-crate"
/// package_version = "1.0"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FfiCapsuleTypeConfig {
    /// Fully-qualified Rust pointee type of the `*const {into_raw_type}` that
    /// `value.into_raw()` already returns. The generated body calls `value.into_raw()`
    /// as-is (no cast is added): since this field IS the declared return type of
    /// `into_raw()`, and the exported function's return type is declared as the same
    /// `*const {into_raw_type}`, an `as` cast would be a no-op and trip
    /// `clippy::unnecessary_cast`.
    /// **Required** — there is no default.
    pub into_raw_type: String,
    /// The bare C type name the exported function returns (used by cbindgen to
    /// declare the return as `const {c_return_type} *`).
    /// **Required** — there is no default.
    pub c_return_type: String,
    /// Cargo crate that provides `into_raw_type`. When set, `scaffold_ffi`
    /// injects it as a direct dependency of the FFI crate so the capsule shim
    /// can name the pointee type. `None` skips injection (e.g. when the pointee
    /// type is already reachable via transitive deps).
    #[serde(default)]
    pub package: Option<String>,
    /// Version requirement for [`Self::package`]. Ignored when `package` is `None`.
    #[serde(default)]
    pub package_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FfiConfig {
    pub prefix: Option<String>,
    #[serde(default = "default_error_style")]
    pub error_style: String,
    pub header_name: Option<String>,
    /// Native library name for Go cgo/Java Panama/C# P/Invoke (e.g., "sample_pack_ffi").
    /// Defaults to `{prefix}_ffi`.
    #[serde(default)]
    pub lib_name: Option<String>,
    /// If true, generate visitor/callback FFI support.
    #[serde(default)]
    pub visitor_callbacks: bool,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Core-crate features that must be *declared* on the generated FFI crate so
    /// that `#[cfg(feature = "X")]` gates in the FFI source compile cleanly under
    /// `RUSTFLAGS="-D warnings"`, but must NOT be enabled by default.
    ///
    /// Use this for mutually-exclusive alternatives to a default feature — e.g. a
    /// `wasm-http` HTTP backend that the FFI source references in
    /// `#[cfg(any(feature = "native-http", feature = "wasm-http"))]` but which must
    /// never be active alongside the default `native-http`. Each entry `X` emits a
    /// `X = ["<core-crate>/X"]` line in `[features]` without adding `X` to `default`.
    ///
    /// Defaults to empty — crates that don't reference non-default core features in
    /// their FFI `#[cfg]` gates can ignore this knob entirely.
    #[serde(default)]
    pub extra_features: Vec<String>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from FFI binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from FFI binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Map of Rust type name -> capsule config for host-native Language passthrough.
    /// Types listed here are NOT boxed into opaque `*mut {Type}` handles; instead the
    /// generated C function returns the host runtime's grammar pointer directly via
    /// `value.into_raw()`. See [`FfiCapsuleTypeConfig`]. This is the foundation that the
    /// Go, Java, C#, Swift, Dart, Zig, and Kotlin Android bindings build their host
    /// `Language` wrappers on top of.
    #[serde(default)]
    pub capsule_types: HashMap<String, FfiCapsuleTypeConfig>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Rust expression used to construct an error value of this crate's
    /// `error_type` from a runtime `String` message inside generated FFI
    /// trait-bridge plugin shims (`plugin_impl_initialize`, `plugin_impl_shutdown`).
    ///
    /// The expression has access to a local variable `msg: String` containing
    /// the underlying error message and is interpolated verbatim. Example
    /// values:
    ///
    /// ```toml
    /// # downstream whose error type has a struct variant with two fields:
    /// plugin_error_constructor = """
    /// sample_core::SampleCrateError::Plugin { message: msg, plugin_name: String::new() }
    /// """
    ///
    /// # downstream whose error type implements `From<String>`:
    /// plugin_error_constructor = "MyError::from(msg)"
    /// ```
    ///
    /// Defaults to `None`. When unset, the plugin shim still emits — backends
    /// fall back to a `format!("{}: {}", prefix, msg)`-style construction via
    /// the configured `error_constructor`. Downstreams that don't expose
    /// trait-bridged plugins can ignore this knob entirely.
    #[serde(default)]
    pub plugin_error_constructor: Option<String>,
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated FFI Cargo.toml. Used when some `cfg(...)` target requires a
    /// reduced feature set (e.g. the `x86_64-linux-android` emulator cannot
    /// link ONNX Runtime, so sample_core ships an `android-target` feature
    /// flag that drops every ORT-dependent extractor). ~keep
    ///
    /// When this list is non-empty the scaffold emits
    /// `[target.'cfg(not(<any-cfg>))'.dependencies]` for the default branch
    /// plus one `[target.'cfg(<cfg>)'.dependencies]` block per override,
    /// instead of the unconditional `[dependencies]` block.
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
    /// Feature names that should be declared as opt-in flags in the generated FFI
    /// crate's `[features]` table but excluded from its `default = [...]` array and
    /// from the core-crate dependency's own explicit `features = [...]` list.
    ///
    /// [`effective_ffi_default_features`](crate::codegen::cfg::effective_ffi_default_features)
    /// otherwise puts every name explicitly configured via [`Self::features`] straight into the
    /// FFI crate's unconditional `default = [...]` list, which forwards
    /// `<name> = ["<core-crate>/<name>"]` on every platform. Cargo unions feature sets across
    /// every dependency edge to the same resolved package, so that unconditional forwarding
    /// re-enables a feature a [`Self::target_dep_overrides`] entry excluded for a specific `cfg`
    /// target -- defeating the override. Listing the name here keeps it declared (so
    /// `cargo build --features <name>` still works) without auto-enabling it, the same tradeoff
    /// `RubyConfig::excluded_default_features` makes for the Magnus crate.
    ///
    /// Distinct from [`Self::extra_features`]: that field is for a name the FFI source
    /// references in a cfg gate but that must never default (a mutually-exclusive alternative);
    /// this field is for a name that legitimately belongs in [`Self::features`] or in
    /// `collect_cfg_features`'s discovered set, but must be stripped back out of both defaulting
    /// surfaces once a [`Self::target_dep_overrides`] entry needs to exclude it somewhere. ~keep
    #[serde(default)]
    pub excluded_default_features: Vec<String>,
}

/// A per-target replacement for the core-crate feature set emitted into the
/// generated FFI Cargo.toml. See [`FfiConfig::target_dep_overrides`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FfiTargetDepOverride {
    /// Cargo cfg expression, without the surrounding `cfg(...)`.
    /// Example: `all(target_os = "android", target_arch = "x86_64")`.
    pub cfg: String,
    /// Replacement feature set used for the core-crate dependency when this
    /// target matches. An empty list means "no features".
    #[serde(default)]
    pub features: Vec<String>,
    /// When false (default), emit `default-features = false` for this target.
    /// When true, allow the core dep's default features through.
    #[serde(default)]
    pub default_features: bool,
}

fn default_error_style() -> String {
    "last_error".to_string()
}
