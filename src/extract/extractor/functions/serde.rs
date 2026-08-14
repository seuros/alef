use super::super::helpers::has_derive;
use ahash::AHashSet;

fn record_derived_serde_traits(
    items: &[syn::Item],
    has_serialize: &mut AHashSet<String>,
    has_deserialize: &mut AHashSet<String>,
) {
    for item in items {
        let (name, attrs) = match item {
            syn::Item::Struct(item) => (&item.ident, item.attrs.as_slice()),
            syn::Item::Enum(item) => (&item.ident, item.attrs.as_slice()),
            _ => continue,
        };
        if has_derive(attrs, "Serialize") {
            has_serialize.insert(name.to_string());
        }
        if has_derive(attrs, "Deserialize") {
            has_deserialize.insert(name.to_string());
        }
    }
}

fn record_manual_serde_traits(
    items: &[syn::Item],
    has_serialize: &mut AHashSet<String>,
    has_deserialize: &mut AHashSet<String>,
) {
    for item in items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let Some((trait_path, _)) = &item_impl.trait_ else {
            continue;
        };
        let syn::Type::Path(type_path) = &*item_impl.self_ty else {
            continue;
        };
        let Some(type_name) = type_path.path.segments.last().map(|segment| segment.ident.to_string()) else {
            continue;
        };
        match trait_path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .as_deref()
        {
            Some("Serialize") => {
                has_serialize.insert(type_name);
            }
            Some("Deserialize") => {
                has_deserialize.insert(type_name);
            }
            _ => {}
        }
    }
}

/// Return types that implement both serde directions through derives, manual impls,
/// or one of each.
pub(crate) fn collect_complete_serde_type_names(items: &[syn::Item]) -> AHashSet<String> {
    let mut has_serialize = AHashSet::new();
    let mut has_deserialize = AHashSet::new();
    record_derived_serde_traits(items, &mut has_serialize, &mut has_deserialize);
    record_manual_serde_traits(items, &mut has_serialize, &mut has_deserialize);
    has_serialize
        .into_iter()
        .filter(|name| has_deserialize.contains(name))
        .collect()
}
