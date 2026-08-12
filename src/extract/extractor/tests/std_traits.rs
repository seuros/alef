use super::*;

#[test]
fn manual_debug_impl_is_not_exposed_as_binding_methods() {
    let source = r#"
        pub struct ServiceConfig {
            pub endpoint: String,
        }

        impl std::fmt::Debug for ServiceConfig {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.debug_struct("ServiceConfig").finish_non_exhaustive()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface
        .types
        .iter()
        .find(|item| item.name == "ServiceConfig")
        .expect("ServiceConfig should be extracted");

    assert!(
        config.methods.is_empty(),
        "Debug::fmt must not become a public binding method"
    );
}
