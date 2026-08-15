//! Service-API codegen for the C# backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **P/Invoke declarations** — [`DllImport`] stubs matching the C FFI contract
//!    (handlers, registration, entrypoints).
//! 2. **Service class** — An idiomatic C# wrapper that uses P/Invoke to invoke
//!    the Rust service, with registration methods and run/finalize entrypoints.
//!
//! The C# service class exposes:
//! - A constructor mirroring [`ServiceDef::constructor`].
//! - Configurator methods from [`ServiceDef::configurators`].
//! - Registration methods from [`ServiceDef::registrations`] that accept C# delegates
//!   and marshal them via `[UnmanagedCallersOnly]` trampolines + `GCHandle`.
//! - Entrypoint methods (run/finalize) from [`ServiceDef::entrypoints`].
//!
//! All names and signatures are derived entirely from the [`ApiSurface`] IR — no
//! transport- or domain-specific assumptions are made anywhere in this module.

use crate::codegen::naming::to_csharp_name;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

mod render;
use render::{gen_native_methods_cs, gen_service_cs};

pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let namespace = config.csharp_namespace();
    let prefix = config.ffi_prefix();

    let output_dir = config
        .output_paths
        .get("csharp")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/csharp/".to_owned());

    let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

    let mut files = Vec::new();

    for service in &api.services {
        let service_cs = gen_service_cs(api, service, &namespace, &prefix);
        let class_name = to_csharp_name(&service.name);
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.cs", class_name)),
            content: service_cs,
            generated_header: false,
        });
    }

    let native_methods = gen_native_methods_cs(api, &namespace, &prefix);
    files.push(GeneratedFile {
        path: base_path.join("ServiceNativeMethods.cs"),
        content: native_methods,
        generated_header: false,
    });

    Ok(files)
}

#[cfg(test)]
mod regressions;

#[cfg(test)]
mod tests;
