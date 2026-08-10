use super::*;

fn boxed_field(optional: bool) -> FieldDef {
    FieldDef {
        name: "settings".to_string(),
        ty: TypeRef::Named("Settings".to_string()),
        optional,
        is_boxed: true,
        ..Default::default()
    }
}

fn render(field: FieldDef) -> String {
    let typ = TypeDef {
        name: "Request".to_string(),
        rust_path: "sample::Request".to_string(),
        fields: vec![field],
        ..Default::default()
    };

    binding_helpers::gen_lossy_binding_to_core_fields(&typ, "sample", false, &AHashSet::new(), false, false, &[])
}

#[test]
fn boxes_required_named_field_after_conversion() {
    let output = render(boxed_field(false));

    assert!(output.contains("settings: Box::new(self.settings.clone().into()),"));
}

#[test]
fn boxes_optional_named_field_after_conversion() {
    let output = render(boxed_field(true));

    assert!(output.contains("settings: self.settings.clone().map(|v| Box::new(v.into())),"));
}

#[test]
fn leaves_non_boxed_named_field_unwrapped() {
    let mut field = boxed_field(false);
    field.is_boxed = false;
    let output = render(field);

    assert!(output.contains("settings: self.settings.clone().into(),"));
    assert!(!output.contains("Box::new"));
}
