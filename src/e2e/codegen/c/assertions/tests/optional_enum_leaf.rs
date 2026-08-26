use super::*;

/// The full `ProcessResult.data -> DataNode.kind` shape once `data` is correctly
/// registered in `result_fields` and `fields_c_types` names both hops (`data` ->
/// `DataNode`, and the enum leaf `kind` -> `DataNodeKind`) — the "config already correct
/// and complete" state a fixture author reaches after following `ts_pack_types`'s
/// diagnostic. `data` is `Optional<Named>` here, matching the real IR (`pub data:
/// Option<DataNode>`), not the bare `Named` `ts_pack_types` uses — this is the actual
/// shape `emit_nested_accessor` must walk through the `Option`. ~keep
fn ts_pack_types_with_optional_data_and_enum_kind() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ProcessResult".into(),
            fields: vec![FieldDef {
                name: "data".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("DataNode".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "DataNode".into(),
            fields: vec![
                FieldDef {
                    name: "kind".into(),
                    ty: TypeRef::Named("DataNodeKind".into()),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "children".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("DataNode".into()))),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
    ]
}

/// Both halves of the ts-pack fix at once: the walk must go through the `Option<DataNode>`
/// hop AND land on the enum branch, not the opaque-struct branch, for the `DataNodeKind`
/// leaf. Before the branch-ordering fix, this leaf matched the opaque-struct filter first
/// (`DataNodeKind` is PascalCase, non-primitive, not `char*`/`skip`) and emitted a bare
/// handle the caller would `strcmp` against instead of a `_to_string`-converted `char*`.
#[test]
fn dotted_path_through_optional_field_reaches_enum_leaf() {
    let types = ts_pack_types_with_optional_data_and_enum_kind();
    let fields_c_types = HashMap::from([
        ("process_result.data".to_string(), "DataNode".to_string()),
        ("data_node.kind".to_string(), "DataNodeKind".to_string()),
    ]);
    let fields_enum: HashSet<String> = ["data.kind".to_string()].into_iter().collect();
    let mut output = String::new();
    let mut handles = Vec::new();

    let result = emit_nested_accessor(
        &mut output,
        "ts_pack",
        "data.kind",
        "data_kind",
        "result",
        &fields_c_types,
        &fields_enum,
        &mut handles,
        "ProcessResult",
        "data.kind",
        &types,
        &global_sources(),
    )
    .expect("the Option<DataNode> hop and the enum leaf both resolve");

    assert_eq!(
        result, None,
        "an enum leaf returns Ok(None) (render_assertion reads it as a plain char*), not \
         Ok(Some(opaque_type)) -- a Some here would mean the opaque-struct branch fired instead"
    );
    assert!(
        output.contains("data_handle = ts_pack_process_result_data(result)"),
        "must walk into the Option<DataNode> field via the FFI accessor: {output}"
    );
    assert!(
        output.contains("ts_pack_data_node_kind_to_string("),
        "must convert the enum leaf via its _to_string accessor, proving the enum branch \
         (not the opaque-struct branch) fired: {output}"
    );
    assert!(
        !output.contains("AlefHandle data_kind = kind_handle"),
        "must not fall through to the opaque-struct branch's bare handle assignment: {output}"
    );
}

/// Two unrelated types below the same result type declaring a field with the same name
/// (`DataNode.kind`, values object/array/scalar, vs `StructureItem.kind`, values
/// function/class) must not collapse into a single confident alias suggestion — this is
/// the tslp scenario that motivated the fix: the pre-fix diagnostic would have proposed
/// exactly `"data.kind" = "structure.kind"`, silently rebinding the assertion to the
/// wrong field.
#[test]
fn ambiguous_leaf_field_name_does_not_suggest_a_specific_alias() {
    let types = vec![
        TypeDef {
            name: "ProcessResult".into(),
            fields: vec![
                FieldDef {
                    name: "data".into(),
                    ty: TypeRef::Named("DataNode".into()),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "structure".into(),
                    ty: TypeRef::Named("StructureItem".into()),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "DataNode".into(),
            fields: vec![FieldDef {
                name: "kind".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "StructureItem".into(),
            fields: vec![FieldDef {
                name: "kind".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];

    let message = ensure_leaf_field_exists(LeafFieldCheck {
        prefix: "ts_pack",
        accessor_fn: "ts_pack_process_result_kind",
        resolved: "kind",
        raw_field: "data.kind",
        segment: "kind",
        parent_snake_type: "process_result",
        parent_is_ir_type: true,
        declared_in_fields_c_types: false,
        result_type_name: "ProcessResult",
        type_defs: &types,
        result_fields_source: &EffectiveConfigSource::Global,
        fields_source: &EffectiveConfigSource::Global,
    })
    .expect_err("`kind` is not a field of `ProcessResult` itself")
    .to_string();

    assert!(
        !message.contains("\"data.kind\" = \"structure.kind\""),
        "must never suggest binding DataNode.kind's field onto the unrelated \
         StructureItem.kind: {message}"
    );
    assert!(
        message.contains("\"data.kind\""),
        "must still name the ambiguous candidate chain rooted at `data`: {message}"
    );
    assert!(
        message.contains("\"structure.kind\""),
        "must still name the ambiguous candidate chain rooted at `structure`: {message}"
    );
    assert!(
        message.contains("DataNode") && message.contains("StructureItem"),
        "must name both declaring types so the operator can tell them apart: {message}"
    );
}
