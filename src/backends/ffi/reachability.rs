use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};

pub(crate) fn type_has_generated_exports(api: &ApiSurface, config: &ResolvedCrateConfig, typ: &TypeDef) -> bool {
    if typ.is_trait || typ.has_lifetime_params || typ.binding_excluded {
        return false;
    }
    if config
        .ffi
        .as_ref()
        .is_some_and(|ffi| ffi.exclude_types.iter().any(|name| name == &typ.name))
    {
        return false;
    }
    if config
        .opaque_types
        .get(&typ.name)
        .is_some_and(|path| path.contains('<'))
    {
        return false;
    }
    let Some(ffi) = &config.ffi else {
        return true;
    };
    if !ffi.capsule_types.contains_key(&typ.name) {
        return true;
    }
    api.types.iter().flat_map(|candidate| &candidate.methods).any(|method| {
        matches!(&method.return_type, TypeRef::Named(name) if name == &typ.name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_types_have_no_generated_ffi_exports() {
        let borrowed = TypeDef {
            name: "BorrowedNode".into(),
            has_lifetime_params: true,
            ..TypeDef::default()
        };
        let api = ApiSurface {
            types: vec![borrowed.clone()],
            ..ApiSurface::default()
        };

        assert!(!type_has_generated_exports(
            &api,
            &ResolvedCrateConfig::default(),
            &borrowed
        ));
    }

    #[test]
    fn owned_types_keep_generated_ffi_exports() {
        let owned = TypeDef {
            name: "OwnedNode".into(),
            ..TypeDef::default()
        };
        let api = ApiSurface {
            types: vec![owned.clone()],
            ..ApiSurface::default()
        };

        assert!(type_has_generated_exports(
            &api,
            &ResolvedCrateConfig::default(),
            &owned
        ));
    }
}
