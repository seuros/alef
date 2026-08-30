use std::collections::HashMap;

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

pub(super) struct AssertionFieldShape {
    pub is_optional: bool,
    pub is_pointer: bool,
    pub is_nullable: bool,
    pub is_array_for_len: bool,
    pub is_slice: bool,
}

pub(super) fn resolve_assertion_field_shape(
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    optional_locals: &HashMap<String, String>,
) -> AssertionFieldShape {
    let Some(field) = assertion.field.as_deref() else {
        return AssertionFieldShape {
            is_optional: false,
            is_pointer: false,
            is_nullable: false,
            is_array_for_len: false,
            is_slice: false,
        };
    };
    let resolved = field_resolver.resolve(field);
    let check_path = resolved
        .strip_suffix(".length")
        .or_else(|| resolved.strip_suffix(".count"))
        .or_else(|| resolved.strip_suffix(".size"))
        .unwrap_or(resolved);
    let uses_plain_local = optional_locals.contains_key(field);
    let is_optional = field_resolver.is_optional(check_path) && !uses_plain_local;
    let is_array_for_len = field_resolver.is_array(check_path);
    let is_slice = field_resolver.is_array(resolved);
    let is_pointer = !uses_plain_local
        && field_resolver
            .target_field_is_pointer(check_path)
            .unwrap_or(is_optional && !is_array_for_len);

    AssertionFieldShape {
        is_optional,
        is_pointer,
        is_nullable: is_optional || is_pointer,
        is_array_for_len,
        is_slice,
    }
}
