use crate::e2e::fixture::Fixture;

pub(super) fn extension_owned_recipe_kind(
    fixture: &Fixture,
    resolved_args: &[crate::core::config::e2e::ArgMapping],
) -> Option<&'static str> {
    if fixture.http.is_some() {
        return Some("HTTP");
    }
    if fixture.asyncapi.is_some() {
        return Some("AsyncAPI");
    }
    if fixture.websocket.is_some() {
        return Some("WebSocket");
    }
    resolved_args
        .iter()
        .any(|argument| argument.arg_type == "test_backend")
        .then_some("test-backend")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::ArgMapping;

    #[test]
    fn test_backend_fixture_requires_public_extension_recipe() {
        let fixture = Fixture {
            id: "register_processor".into(),
            description: "Register a processor".into(),
            args: vec![ArgMapping {
                name: "processor".into(),
                field: "processor".into(),
                arg_type: "test_backend".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: Some("Processor".into()),
            }],
            ..Fixture::default()
        };

        assert_eq!(
            extension_owned_recipe_kind(&fixture, &fixture.args),
            Some("test-backend")
        );
    }

    #[test]
    fn ordinary_call_uses_builtin_recipe() {
        let fixture = Fixture {
            id: "process_document".into(),
            description: "Process a document".into(),
            ..Fixture::default()
        };

        assert_eq!(extension_owned_recipe_kind(&fixture, &[]), None);
    }

    #[test]
    fn call_level_test_backend_requires_public_extension_recipe() {
        let fixture = Fixture {
            id: "register_processor".into(),
            description: "Register a processor".into(),
            ..Fixture::default()
        };
        let call_args = vec![ArgMapping {
            name: "processor".into(),
            field: "processor".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("Processor".into()),
        }];

        assert_eq!(extension_owned_recipe_kind(&fixture, &call_args), Some("test-backend"));
    }
}
