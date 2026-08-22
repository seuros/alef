//! `cmd/setup/main.go`'s generated shim writes a reference to
//! `RequireNativeSetup_<versionIdent>`, and `native_setup.go` defines
//! `const RequireNativeSetup_<version_ident>`. Two files, two rewriters, one symbol: if the
//! identifiers ever disagree the shim names a symbol that does not exist and the consumer's Go
//! build fails after `cmd/setup` runs — with no local cause, because each file is internally
//! consistent. That is alef#159, where `versionIdent` stayed at `3_11_1`
//! while the sentinel advanced to `3_11_2`.
//!
//! `version::sync_versions` closes it structurally by calling `to_go_version_ident` exactly once
//! and threading the single result into both rewriters. These tests pin that pairing as a
//! property rather than a convention, so a later refactor cannot reintroduce a second derivation
//! without failing here.

use super::*;

/// The identifier `native_setup.go`'s sentinel declares, read back out of real rewritten content.
fn sentinel_identifier(content: &str) -> Option<String> {
    let rest = content.split_once("const RequireNativeSetup_")?.1;
    let ident: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!ident.is_empty()).then_some(ident)
}

/// The identifier `cmd/setup/main.go`'s shim will interpolate, read back out of real rewritten
/// content.
fn version_ident_const(content: &str) -> Option<String> {
    let rest = content.split_once("versionIdent")?.1;
    let value = rest.split_once('"')?.1;
    value.split_once('"').map(|(ident, _)| ident.to_owned())
}

fn native_setup_at(ident: &str, version: &str) -> String {
    format!("package samplepack\n\nconst RequireNativeSetup_{ident} = \"{version}\"\n")
}

fn cmd_setup_at(ident: &str, version: &str) -> String {
    format!("package main\n\nconst (\n\tmoduleVersion     = \"{version}\"\n\tversionIdent      = \"{ident}\"\n)\n")
}

#[test]
fn a_version_bump_moves_the_shim_ident_and_the_sentinel_to_the_same_identifier() {
    let new_version = "3.11.4";
    // Single derivation, exactly as `sync_versions` does it -- the whole point of the fix. ~keep
    let go_version_ident = crate::core::version::to_go_version_ident(new_version);

    let sentinel = sync_go_native_setup_sentinel(&native_setup_at("3_11_1", "3.11.1"), &go_version_ident, new_version)
        .expect("the sentinel was stale, so a rewrite must have happened");
    let shim = sync_go_cmd_setup_version_ident(&cmd_setup_at("3_11_1", "3.11.1"), &go_version_ident)
        .expect("the versionIdent const was stale, so a rewrite must have happened");

    assert_eq!(
        sentinel_identifier(&sentinel).as_deref(),
        version_ident_const(&shim).as_deref(),
        "the shim references RequireNativeSetup_<versionIdent>; a mismatch is an undefined symbol"
    );
    assert_eq!(version_ident_const(&shim).as_deref(), Some("3_11_4"));
}

#[test]
fn the_pairing_assertion_detects_the_two_identifiers_drifting_apart() {
    // Negative control: reproduce alef#159 by rewriting only ONE side, and prove the
    // assertion above would have caught it. Without this, that test could pass vacuously. ~keep
    let sentinel = sync_go_native_setup_sentinel(&native_setup_at("3_11_1", "3.11.1"), "3_11_2", "3.11.2")
        .expect("the sentinel changed");
    let untouched_shim = cmd_setup_at("3_11_1", "3.11.1");

    assert_eq!(sentinel_identifier(&sentinel).as_deref(), Some("3_11_2"));
    assert_ne!(
        sentinel_identifier(&sentinel).as_deref(),
        version_ident_const(&untouched_shim).as_deref(),
        "this is the alef#159 defect itself; if these compare equal the pairing test proves nothing"
    );
}

#[test]
fn the_generated_shim_interpolates_the_same_const_the_rewriter_owns() {
    // The pairing only holds if the shim really builds its reference from `versionIdent`. Assert
    // against the REAL template text, so renaming the const there fails here instead of in a
    // consumer's Go build. ~keep
    let template = include_str!("../../../backends/go/templates/cmd_setup_main.go.jinja");
    assert!(
        template.contains(r#""var _ = %s.RequireNativeSetup_%s\n", bindingImportName, versionIdent"#),
        "cmd_setup_main.go.jinja no longer builds the sentinel reference from versionIdent"
    );
    let native = include_str!("../../../backends/go/templates/native_setup.go.jinja");
    assert!(
        native.contains("const RequireNativeSetup_{{ version_ident }}"),
        "native_setup.go.jinja no longer declares the sentinel the shim references"
    );
}
