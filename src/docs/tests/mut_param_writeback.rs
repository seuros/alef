//! Docs must document the signature the backends actually emit for a `&mut T` DTO parameter.
//!
//! The binding returns the updated `T` in place of `()` (see `codegen::mut_writeback`). A docs
//! page that still prints the unit return tells the reader to drop a value the binding hands
//! back, which is how the silent-no-op bug reaches the reader a second time.

use super::*;
use crate::core::ir::ParamDef;

const RECORD: &str = "Record";

fn record_type() -> TypeDef {
    let mut ty = empty_type(RECORD);
    ty.doc = "A record.".to_string();
    ty.fields = vec![make_field("label", TypeRef::String, false, None)];
    ty
}

fn mut_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: true,
        ..ParamDef::default()
    }
}

fn ref_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: false,
        ..ParamDef::default()
    }
}

fn api_with(function: FunctionDef) -> ApiSurface {
    let mut api = make_minimal_api("1.0.0");
    api.types = vec![record_type()];
    api.enums = vec![];
    api.errors = vec![];
    api.functions = vec![function];
    api
}

fn function_with(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> FunctionDef {
    let mut func = make_function(name, vec![], return_type, false, None);
    func.params = params;
    func.doc = "Tags the record.".to_string();
    func
}

fn python_page(api: &ApiSurface) -> String {
    let files = generate_docs(api, &make_test_config(), &[Language::Python], "docs").expect("docs render");
    files
        .iter()
        .find(|f| f.path.to_str().unwrap_or_default().contains("api-python"))
        .expect("python page")
        .content
        .clone()
}

#[test]
fn should_document_a_mut_dto_param_as_returning_the_updated_value() {
    let api = api_with(function_with(
        "tag_record",
        vec![mut_param("record", RECORD)],
        TypeRef::Unit,
    ));
    let page = python_page(&api);

    assert!(
        page.contains("def tag_record(record: Record) -> Record"),
        "docs must show the write-back return the binding emits; got:\n{page}"
    );
    assert!(
        !page.contains("def tag_record(record: Record) -> None"),
        "docs must not keep printing the unit return; got:\n{page}"
    );
}

#[test]
fn should_not_document_a_write_back_for_an_immutable_borrow_param() {
    let api = api_with(function_with(
        "read_record",
        vec![ref_param("record", RECORD)],
        TypeRef::Unit,
    ));
    let page = python_page(&api);

    assert!(
        page.contains("def read_record(record: Record) -> None"),
        "a `&T` param must not gain a write-back return; got:\n{page}"
    );
}

#[test]
fn should_not_document_a_write_back_for_an_owned_param() {
    let mut owned = mut_param("record", RECORD);
    owned.is_ref = false;
    owned.is_mut = false;
    let api = api_with(function_with("consume_record", vec![owned], TypeRef::Unit));
    let page = python_page(&api);

    assert!(
        page.contains("def consume_record(record: Record) -> None"),
        "an owned `T` param must be unchanged; got:\n{page}"
    );
}

#[test]
fn should_not_document_a_write_back_for_a_mut_opaque_param() {
    let mut opaque = record_type();
    opaque.is_opaque = true;
    let mut api = api_with(function_with(
        "bump_record",
        vec![mut_param("record", RECORD)],
        TypeRef::Unit,
    ));
    api.types = vec![opaque];
    let page = python_page(&api);

    assert!(
        page.contains("def bump_record(record: Record) -> None"),
        "an opaque handle mutates through the handle and must keep its unit return; got:\n{page}"
    );
}
