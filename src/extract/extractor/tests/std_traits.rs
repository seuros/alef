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

#[test]
fn external_crate_trait_impl_is_not_exposed_as_binding_methods() {
    let source = r#"
        pub struct AttributeGroup {
            pub name: String,
        }

        impl schemagen::ToSchema for AttributeGroup {
            fn schema() -> Vec<(String, RefOr<Schema>)> {
                Vec::new()
            }
            fn schemas(items: &mut Vec<(String, RefOr<Schema>)>) {
                let _ = items;
            }
        }
    "#;

    let surface = extract_from_source(source);
    let group = surface
        .types
        .iter()
        .find(|item| item.name == "AttributeGroup")
        .expect("AttributeGroup should be extracted");

    assert!(
        group.methods.is_empty(),
        "a foreign crate's trait methods must not enter the binding surface, got: {:?}",
        group.methods.iter().map(|method| &method.name).collect::<Vec<_>>()
    );
}

#[test]
fn a_local_trait_impl_still_contributes_its_methods() {
    let source = r#"
        pub trait Summarize {
            fn summarize(&self) -> String;
        }

        pub struct Report {
            pub title: String,
        }

        impl crate::Summarize for Report {
            fn summarize(&self) -> String {
                self.title.clone()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let report = surface
        .types
        .iter()
        .find(|item| item.name == "Report")
        .expect("Report should be extracted");

    assert!(
        report.methods.iter().any(|method| method.name == "summarize"),
        "a crate-local trait impl must still contribute its methods, got: {:?}",
        report.methods.iter().map(|method| &method.name).collect::<Vec<_>>()
    );
}
