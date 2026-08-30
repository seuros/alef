use std::fmt::Write;

pub(super) fn string_value_expression(field: &str, is_pointer: bool, is_data_interface: bool) -> String {
    if is_data_interface {
        format!("jsonString(t, {field})")
    } else if is_pointer {
        format!("string(*{field})")
    } else {
        format!("string({field})")
    }
}

pub(super) fn contains_value_expression(
    field: &str,
    is_pointer: bool,
    is_array: bool,
    is_data_interface: bool,
) -> String {
    if is_data_interface || is_array {
        format!("jsonString(t, {field})")
    } else {
        string_value_expression(field, is_pointer, false)
    }
}

pub(super) fn render_guarded_scalar_comparison(
    out: &mut String,
    guard: Option<&str>,
    field_expr: &str,
    operator: &str,
    comparison_value: &str,
    expected_message: &str,
) -> bool {
    let Some(guard) = guard else {
        return false;
    };
    let _ = writeln!(out, "\tif {guard} != nil {{");
    let _ = writeln!(out, "\t\tif {field_expr} {operator} {comparison_value} {{");
    let _ = writeln!(
        out,
        "\t\t\tt.Errorf(\"expected {expected_message}, got %v\", {field_expr})"
    );
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
    true
}

pub(super) fn render_count_assertion(
    out: &mut String,
    field: &str,
    count: u64,
    nullable_guard: Option<&str>,
    is_slice: bool,
    exact: bool,
) {
    let (method, message) = if exact {
        ("Equal", format!("expected exactly {count} elements"))
    } else {
        ("GreaterOrEqual", format!("expected at least {count} elements"))
    };
    let is_length = field.starts_with("len(");
    let length = if is_length {
        field.to_string()
    } else if nullable_guard.is_some() && !is_slice {
        format!("len(*{field})")
    } else {
        format!("len({field})")
    };
    if let Some(guard) = nullable_guard {
        let _ = writeln!(out, "\tif {guard} != nil {{");
        let _ = writeln!(out, "\t\tassert.{method}(t, {length}, {count}, \"{message}\")");
        let _ = writeln!(out, "\t}}");
    } else {
        let _ = writeln!(out, "\tassert.{method}(t, {length}, {count}, \"{message}\")");
    }
}

pub(super) fn render_length_assertion(
    out: &mut String,
    field: &str,
    length: u64,
    nullable_guard: Option<&str>,
    is_pointer: bool,
    minimum: bool,
) {
    let (method, relation) = if minimum {
        ("GreaterOrEqual", ">=")
    } else {
        ("LessOrEqual", "<=")
    };
    let expression = if field.starts_with("len(") {
        field.to_string()
    } else if is_pointer {
        format!("len(*{field})")
    } else {
        format!("len({field})")
    };
    if let Some(guard) = nullable_guard {
        let _ = writeln!(out, "\tif {guard} != nil {{");
        let _ = writeln!(
            out,
            "\t\tassert.{method}(t, {expression}, {length}, \"expected length {relation} {length}\")"
        );
        let _ = writeln!(out, "\t}}");
    } else {
        let _ = writeln!(
            out,
            "\tassert.{method}(t, {expression}, {length}, \"expected length {relation} {length}\")"
        );
    }
}
