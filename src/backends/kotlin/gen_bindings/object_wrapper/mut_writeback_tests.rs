//! Issue #380: `fn tag_record(record: &mut Record)` must not silently drop the mutation.
//!
//! The Kotlin JVM facade is a thin `object` wrapper that delegates every call to the generated
//! Java `Bridge` class. Java now rewrites the unsupported `&mut` DTO shape to return the updated
//! value instead of `void` (see `backends::java::gen_bindings::ffi_class::sync_functions`), so
//! this Kotlin wrapper must follow suit: it must return whatever `Bridge.tagRecord(...)` now
//! returns instead of treating the call as a `Unit`-returning statement.

use super::emit_function;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use std::collections::BTreeSet;

fn mut_dto_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: true,
        ..Default::default()
    }
}

fn tag_record_fn(is_ref: bool, is_mut: bool) -> FunctionDef {
    FunctionDef {
        name: "tag_record".to_string(),
        params: vec![ParamDef {
            is_ref,
            is_mut,
            ..mut_dto_param("record", "Record")
        }],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }
}

fn render(f: &FunctionDef, opaque_type_names: &ahash::AHashSet<String>) -> String {
    let mut out = String::new();
    let mut imports = BTreeSet::new();
    emit_function(
        f,
        &mut out,
        &mut imports,
        "io.test.krz",
        &std::collections::HashSet::new(),
        opaque_type_names,
    );
    out
}

/// The load-bearing check: signature grows a `Record` return, the call still delegates to
/// `Bridge.tagRecord`, and the pre-fix void/no-return shape is gone.
#[test]
fn mut_dto_param_writes_back_the_mutated_value() {
    let func = tag_record_fn(true, true);
    let out = render(&func, &ahash::AHashSet::new());

    assert!(
        out.contains("fun tagRecord(record: Record): Record {"),
        "expected the write-back signature (Record in, Record out), got:\n{out}"
    );
    assert!(
        out.contains("return Bridge.tagRecord(record)"),
        "must return whatever the Java Bridge method now returns, got:\n{out}"
    );
    assert!(
        !out.contains("): Unit {"),
        "must not regress to a Unit return, got:\n{out}"
    );
    assert!(
        !out.contains("Bridge.tagRecord(record)\n    }") || out.contains("return Bridge.tagRecord(record)"),
        "must not regress to the lossy bare-statement-then-implicit-return shape, got:\n{out}"
    );
}

/// Negative control: an immutable `&Record` DTO parameter must NOT gain a write-back return --
/// the pre-existing bare-statement `Unit` shape is correct for it.
#[test]
fn immutable_dto_param_gets_no_writeback() {
    let func = tag_record_fn(true, false);
    let out = render(&func, &ahash::AHashSet::new());

    assert!(
        out.contains("fun tagRecord(record: Record): Unit {"),
        "an immutable DTO param must keep the plain Unit signature, got:\n{out}"
    );
    assert!(
        out.contains("Bridge.tagRecord(record)") && !out.contains("return Bridge.tagRecord(record)"),
        "an immutable DTO param must call Bridge as a bare statement, not return its result, got:\n{out}"
    );
}

/// Negative control: an owned (by-value) DTO parameter must render the same as before this fix.
#[test]
fn owned_dto_param_is_unchanged() {
    let func = tag_record_fn(false, false);
    let out = render(&func, &ahash::AHashSet::new());

    assert!(
        out.contains("fun tagRecord(record: Record): Unit {"),
        "an owned DTO param must keep the plain Unit signature, got:\n{out}"
    );
    assert!(
        out.contains("Bridge.tagRecord(record)") && !out.contains("return Bridge.tagRecord(record)"),
        "an owned DTO param must call Bridge as a bare statement, not return its result, got:\n{out}"
    );
}

/// Negative control: a `&mut` parameter on a genuinely opaque (handle-backed) type must NOT be
/// treated as a write-back DTO -- the host already mutates through the live handle correctly,
/// so rewriting its Kotlin signature would be a regression, not a fix.
#[test]
fn mut_opaque_handle_param_gets_no_writeback() {
    let func = tag_record_fn(true, true);
    let mut opaque_type_names = ahash::AHashSet::default();
    opaque_type_names.insert("Record".to_string());
    let out = render(&func, &opaque_type_names);

    assert!(
        out.contains("fun tagRecord(record: Record): Unit {"),
        "an opaque &mut param must keep the plain Unit signature, got:\n{out}"
    );
}
