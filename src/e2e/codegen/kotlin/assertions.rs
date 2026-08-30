//! Kotlin assertion rendering helpers.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification (routing `not_error` through
//! `not_error_presence::may_assert_presence`) added one parameter to `render_assertion`,
//! required at every call site — the small net growth here is that mechanical churn plus the
//! `not_error` arm's updated doc comment, not new unrelated functionality.

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_option: bool,
    enum_fields: &std::collections::HashSet<String>,
    json_scalar_fields: &std::collections::HashSet<String>,
    fields_c_types: &std::collections::HashMap<String, String>,
    is_streaming: bool,
    kotlin_android_style: bool,
    not_error_may_assert_presence: bool,
) {
    if super::assertion_field_gates::try_render_field_shape_gates(
        out,
        assertion,
        field_resolver,
        result_var,
        result_is_simple,
        is_streaming,
        kotlin_android_style,
        fields_c_types,
    ) {
        return;
    }

    super::assertion_scalar_dispatch::render_scalar_pipeline(
        out,
        assertion,
        field_resolver,
        result_var,
        result_is_simple,
        result_is_option,
        enum_fields,
        json_scalar_fields,
        fields_c_types,
        kotlin_android_style,
        is_streaming,
        not_error_may_assert_presence,
    );
}

#[cfg(test)]
mod tests;
