use crate::e2e::field_access::FieldResolver;

pub(super) fn classify(field: Option<&str>, resolver: &FieldResolver) -> (bool, bool) {
    let Some(field) = field else {
        return (false, false);
    };
    let resolved = resolver.resolve(field);
    let is_array = resolver.is_array(resolved);
    let is_object = !is_array && !resolver.is_optional(resolved) && resolver.is_display_unsafe(field);
    (is_array, is_object)
}
