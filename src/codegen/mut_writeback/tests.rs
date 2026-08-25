use super::*;
use crate::core::ir::ParamDef;

fn opaque(names: &[&str]) -> AHashSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

fn param(name: &str, ty: TypeRef, is_ref: bool, is_mut: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        is_ref,
        is_mut,
        ..ParamDef::default()
    }
}

fn mut_dto(name: &str, ty_name: &str) -> ParamDef {
    param(name, TypeRef::Named(ty_name.to_string()), true, true)
}

#[test]
fn should_classify_mut_dto_param_as_writeback() {
    assert!(is_writeback_param(&mut_dto("record", "Record"), &opaque(&[])));
}

#[test]
fn should_not_classify_immutable_borrow_as_writeback() {
    let p = param("record", TypeRef::Named("Record".into()), true, false);
    assert!(!is_writeback_param(&p, &opaque(&[])));
}

#[test]
fn should_not_classify_owned_param_as_writeback() {
    let p = param("record", TypeRef::Named("Record".into()), false, false);
    assert!(!is_writeback_param(&p, &opaque(&[])));
}

#[test]
fn should_not_classify_mut_opaque_param_as_writeback() {
    assert!(!is_writeback_param(&mut_dto("engine", "Engine"), &opaque(&["Engine"])));
}

#[test]
fn should_not_classify_mut_scalar_param_as_writeback() {
    let p = param(
        "count",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
        true,
        true,
    );
    assert!(!is_writeback_param(&p, &opaque(&[])));
}

#[test]
fn should_not_classify_optional_mut_dto_param_as_writeback() {
    let mut p = mut_dto("record", "Record");
    p.optional = true;
    assert!(!is_writeback_param(&p, &opaque(&[])));
}

#[test]
fn should_select_the_single_mut_dto_param_when_return_is_unit() {
    let params = vec![
        param("label", TypeRef::String, false, false),
        mut_dto("record", "Record"),
    ];
    let selected = writeback_param(&params, &TypeRef::Unit, &opaque(&[])).expect("writeback param");
    assert_eq!(selected.name, "record");
}

#[test]
fn should_select_no_param_when_function_already_returns_a_value() {
    let params = vec![mut_dto("record", "Record")];
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::U32);
    assert!(writeback_param(&params, &ty, &opaque(&[])).is_none());
}

#[test]
fn should_report_the_named_dto_type_to_return() {
    assert_eq!(writeback_type_name(&mut_dto("record", "Record")), Some("Record"));
}

#[test]
fn should_replace_unit_return_with_the_mut_param_type() {
    let params = vec![mut_dto("record", "Record")];
    let effective = effective_return_type(&params, &TypeRef::Unit, &opaque(&[])).expect("effective return");
    assert_eq!(effective, TypeRef::Named("Record".into()));
}

#[test]
fn should_leave_return_type_alone_when_no_mut_dto_param() {
    let params = vec![param("record", TypeRef::Named("Record".into()), true, false)];
    assert!(effective_return_type(&params, &TypeRef::Unit, &opaque(&[])).is_none());
}

#[test]
fn should_accept_a_signature_with_no_mut_dto_param() {
    let params = vec![param("record", TypeRef::Named("Record".into()), true, false)];
    reject_unsupported_writeback("read_record", &params, &TypeRef::Unit, &opaque(&[])).expect("accepted");
}

#[test]
fn should_accept_one_mut_dto_param_on_a_unit_returning_function() {
    let params = vec![mut_dto("record", "Record")];
    reject_unsupported_writeback("tag_record", &params, &TypeRef::Unit, &opaque(&[])).expect("accepted");
}

#[test]
fn should_reject_two_mut_dto_params_naming_the_function_and_params() {
    let params = vec![mut_dto("first", "Record"), mut_dto("second", "Record")];
    let error = reject_unsupported_writeback("tag_pair", &params, &TypeRef::Unit, &opaque(&[]))
        .expect_err("two `&mut` DTO params must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("`tag_pair`"),
        "diagnostic must name the function: {message}"
    );
    assert!(message.contains("first"), "diagnostic must name the params: {message}");
    assert!(message.contains("second"), "diagnostic must name the params: {message}");
}

#[test]
fn should_reject_a_mut_dto_param_on_a_value_returning_function() {
    let params = vec![mut_dto("record", "Record")];
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::U32);
    let error = reject_unsupported_writeback("tag_and_count", &params, &ty, &opaque(&[]))
        .expect_err("`&mut` param plus a return value must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("`tag_and_count`"),
        "diagnostic must name the function: {message}"
    );
    assert!(message.contains("record"), "diagnostic must name the param: {message}");
}

#[test]
fn should_accept_a_mut_opaque_param_on_a_value_returning_function() {
    let params = vec![mut_dto("engine", "Engine")];
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::U32);
    reject_unsupported_writeback("bump_engine", &params, &ty, &opaque(&["Engine"])).expect("accepted");
}
