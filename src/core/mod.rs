//! Core types and configuration for alef polyglot binding generator.
//! Defines IR types, config schema, and backend trait.

pub mod backend;
pub mod config;
pub mod error;
pub mod extension;
pub mod hash;
pub mod ir;
pub mod jni;
pub mod keep_marker;
pub mod keywords;
pub mod template_env;
pub mod template_versions;
pub mod toolchain;
pub mod validation;
pub mod version;

pub use backend::{Backend, Capabilities, GeneratedFile};
pub use config::resolve_output_dir;
pub use error::AlefError;
pub use extension::{Extension, ExtensionConfig};
pub use ir::ApiSurface;
pub use template_env::TemplateEnv;
pub use toolchain::tool_command;
