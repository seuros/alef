use super::*;
use crate::core::ir::{FieldDef, ParamDef, TypeDef, TypeRef};

/// The neutral `FieldConfigSources` most tests want: neither `result_fields` nor
/// `fields` has a per-call override in effect, so every diagnostic falls back to
/// naming the global keys — the shape every test that isn't specifically exercising
/// the per-call branch expects.
fn global_sources() -> FieldConfigSources {
    FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::Global,
    }
}

/// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
/// (present, non-`binding_excluded`, on some IR type) but missing from the
/// hand-maintained `result_fields` config must still render a real assertion,
/// not a "skipped: field not available" comment — `c.rs` (both the main-suite
/// and snippet resolver construction sites) now threads
/// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
#[test]
fn c_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
    let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("data".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &resolver,
        &[],
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert!(!out.contains("skipped"), "got: {out}");
}

/// The negative-control half of the same regression: `internal_diagnostics`
/// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
/// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
/// NOT `#[serde(skip)]`, which alone does not exclude a field from the
/// binding surface. Even though it is listed in `result_fields` (a stale/
/// wrong config entry), the IR must still win and reject it. ~keep
#[test]
fn c_ir_excluded_field_present_in_result_fields_is_still_skipped() {
    let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), excluded, HashSet::new());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("internal_diagnostics".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &resolver,
        &[],
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert!(out.contains("skipped"), "got: {out}");
}

/// Task 1c backstop: even after the enum-vs-opaque-handle classification gap is
/// fixed elsewhere, a field `render_assertion` is told is a genuine opaque handle
/// must never be compared via `strcmp` — the ABI carries it as a scalar `uint64_t`
/// `AlefHandle`, and `strcmp` on that is undefined behavior, not merely wrong. A
/// numeric `equals` value must compare exactly instead.
#[test]
fn equals_assertion_on_opaque_handle_compares_numerically_not_via_strcmp() {
    let reachable: HashSet<String> = ["status".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("status".to_string()),
        value: Some(serde_json::json!(2)),
        ..Default::default()
    };
    let accessed_fields = [("status".to_string(), "status".to_string(), false)];
    let mut opaque_handle_locals = HashMap::new();
    opaque_handle_locals.insert("status".to_string(), "batch_status".to_string());

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &resolver,
        &accessed_fields,
        &HashMap::new(),
        &opaque_handle_locals,
        &HashMap::new(),
    );

    assert!(out.contains("status == 2"), "got: {out}");
    assert!(!out.contains("strcmp"), "must not strcmp a uint64_t handle: {out}");
}

/// Negative control / companion: a string expected value against an opaque handle
/// means the field should have matched `try_emit_enum_accessor` and didn't. Rather
/// than emit `status == "completed"` — a pointer comparison against a string literal
/// that compiles cleanly and always lies — this weakens to an honest existence check,
/// mirroring the precedent already established for `not_empty`/`is_empty`.
#[test]
fn equals_assertion_on_opaque_handle_with_string_value_falls_back_to_existence_check() {
    let reachable: HashSet<String> = ["status".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("status".to_string()),
        value: Some(serde_json::Value::String("completed".to_string())),
        ..Default::default()
    };
    let accessed_fields = [("status".to_string(), "status".to_string(), false)];
    let mut opaque_handle_locals = HashMap::new();
    opaque_handle_locals.insert("status".to_string(), "batch_status".to_string());

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &resolver,
        &accessed_fields,
        &HashMap::new(),
        &opaque_handle_locals,
        &HashMap::new(),
    );

    assert!(out.contains("status != 0"), "got: {out}");
    assert!(
        !out.contains("strcmp"),
        "must not compare a uint64_t handle to a string literal: {out}"
    );
}

#[test]
fn nested_optional_handle_type_comes_from_ir_when_config_mapping_is_absent() {
    let types = [
        TypeDef {
            name: "ExtractionResult".into(),
            fields: vec![FieldDef {
                name: "summary".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionSummary".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ExtractionSummary".into(),
            fields: vec![FieldDef {
                name: "processed".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];
    let mut output = String::new();
    let mut handles = Vec::new();

    emit_nested_accessor(
        &mut output,
        "sample",
        "summary.processed",
        "summary_processed",
        "result",
        &HashMap::from([("extraction_summary.processed".into(), "uint64_t".into())]),
        &HashSet::new(),
        &mut handles,
        "ExtractionResult",
        "summary.processed",
        &types,
        &global_sources(),
    )
    .expect("every hop resolves");

    assert!(output.contains("SAMPLEAlefHandle summary_handle"), "{output}");
    assert!(output.contains("sample_extraction_result_summary(result)"), "{output}");
    assert!(output.contains("uint64_t summary_processed"), "{output}");
}

/// The crawlberg shape: `ScrapeResult.metadata -> PageMetadata.article ->
/// ArticleMetadata.tags`, asserted by a fixture as `article.tags.length`. With no
/// `article.*` alias configured, `article` is stripped as a virtual namespace before
/// this function is called, so the walk starts on `ScrapeResult` and looks for a field
/// `tags` that lives two hops further down. ~keep
fn crawlberg_article_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ScrapeResult".into(),
            fields: vec![FieldDef {
                name: "metadata".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("PageMetadata".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "PageMetadata".into(),
            fields: vec![FieldDef {
                name: "article".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ArticleMetadata".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ArticleMetadata".into(),
            fields: vec![FieldDef {
                name: "tags".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn walk_crawlberg_article_tags() -> anyhow::Error {
    walk_crawlberg_article_tags_with_sources(&global_sources())
}

fn walk_crawlberg_article_tags_with_sources(config_sources: &FieldConfigSources) -> anyhow::Error {
    let mut output = String::new();
    let mut handles = Vec::new();
    emit_nested_accessor(
        &mut output,
        "cberg",
        "tags.length",
        "article_tags_length",
        "result",
        &HashMap::new(),
        &HashSet::new(),
        &mut handles,
        "ScrapeResult",
        "article.tags.length",
        &crawlberg_article_types(),
        config_sources,
    )
    .expect_err("`tags` is not a field of ScrapeResult")
}

/// A consumer config gap must surface as an error, not a process-killing panic.
#[test]
fn missing_intermediate_type_returns_an_error_instead_of_panicking() {
    let message = walk_crawlberg_article_tags().to_string();
    assert!(message.contains("fields_c_types"), "{message}");
    assert!(message.contains("scrape_result.tags"), "{message}");
    assert!(message.contains("tags.length"), "{message}");
}

/// Every fact the old panic carried must survive the conversion.
#[test]
fn missing_intermediate_type_keeps_the_original_panic_facts() {
    let message = walk_crawlberg_article_tags().to_string();
    assert!(message.contains("path \"tags.length\""), "{message}");
    assert!(message.contains("segment \"tags\""), "{message}");
    assert!(message.contains("`Tags`"), "guessed-name rationale is gone: {message}");
    assert!(message.contains("`DataNode` vs `Data`"), "{message}");
}

/// The point of the rewrite. The message must not leave "add the key it named" as the
/// obvious remedy, because that key would emit `cberg_scrape_result_tags()` -- a symbol
/// no backend generates. It has to name the stripped namespace, the real chain, and the
/// alias that reconnects them.
#[test]
fn missing_intermediate_type_names_the_real_chain_not_the_phantom_key() {
    let message = walk_crawlberg_article_tags().to_string();

    assert!(
        message.contains("Type `ScrapeResult` has no field `tags`"),
        "must say why the key is missing: {message}"
    );
    assert!(
        message.contains("stripped the leading \"article\""),
        "must name the namespace stripping that produced the path: {message}"
    );
    assert!(
        message.contains("cberg_scrape_result_tags()"),
        "must name the C symbol declaring the key would conjure: {message}"
    );
    assert!(
        message.contains("cberg_article_metadata_tags()"),
        "must name the C symbol that really exists: {message}"
    );
    assert!(
        message.contains("\"metadata.article.tags\""),
        "must name the real resolved chain: {message}"
    );
    assert!(
        message.contains("\"article.tags\" = \"metadata.article.tags\""),
        "must spell the alias that fixes it: {message}"
    );
    assert!(
        message.contains("[crates.e2e.fields]"),
        "must name the alias table, not just fields_c_types: {message}"
    );
}

/// The `fields` sibling of the `result_fields` fix: a non-empty per-call `fields`
/// override REPLACES the global alias table outright (`E2eConfig::effective_fields`),
/// so when a per-call override is what's in effect, the alias-fix must name that
/// call's own key -- never the global one, which an edit would not reach.
#[test]
fn missing_intermediate_type_names_the_per_call_fields_when_that_is_what_shadows() {
    let sources = FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::PerCall("[crates.e2e.calls.scrape]".to_string()),
    };
    let message = walk_crawlberg_article_tags_with_sources(&sources).to_string();

    assert!(
        message.contains("\"article.tags\" = \"metadata.article.tags\" under `[crates.e2e.calls.scrape].fields`"),
        "must name the per-call key that actually governs this call: {message}"
    );
    assert!(
        !message.contains("under `[crates.e2e.fields]`"),
        "must not point at the global key when a per-call override shadows it: {message}"
    );
}

/// The other half of the diagnostic: when the field genuinely does not exist anywhere
/// under the result type, there is no alias to suggest and the message must say so
/// rather than inventing a chain.
#[test]
fn missing_intermediate_type_says_so_when_no_type_carries_the_field() {
    let mut output = String::new();
    let mut handles = Vec::new();
    let error = emit_nested_accessor(
        &mut output,
        "cberg",
        "nowhere.length",
        "nowhere_length",
        "result",
        &HashMap::new(),
        &HashSet::new(),
        &mut handles,
        "ScrapeResult",
        "nowhere.length",
        &crawlberg_article_types(),
        &global_sources(),
    )
    .expect_err("`nowhere` is not a field of anything");

    let message = error.to_string();
    assert!(
        message.contains("No type reachable from `ScrapeResult` has a field named `nowhere`"),
        "{message}"
    );
    assert!(
        !message.contains("under `[crates.e2e.fields]`"),
        "must not suggest an alias it cannot spell: {message}"
    );
    assert!(
        !message.contains("stripped the leading"),
        "nothing was stripped here: {message}"
    );
}

#[test]
fn stripped_namespace_prefix_recovers_only_a_real_stripped_prefix() {
    assert_eq!(
        stripped_namespace_prefix("article.tags.length", "tags.length"),
        Some("article")
    );
    assert_eq!(
        stripped_namespace_prefix("interaction.action_results[0].x", "action_results[0].x"),
        Some("interaction")
    );
    assert_eq!(stripped_namespace_prefix("tags.length", "tags.length"), None);
    assert_eq!(
        stripped_namespace_prefix("metadata.title", "something.else"),
        None,
        "a raw field that does not end with the resolved path was not produced by stripping"
    );
}

#[test]
fn find_field_path_returns_the_shallowest_chain_and_its_declaring_type() {
    let types = crawlberg_article_types();

    let tags = find_field_path("ScrapeResult", "tags", &types).expect("tags is reachable");
    assert_eq!(tags.path, "metadata.article.tags");
    assert_eq!(
        tags.owner_type, "ArticleMetadata",
        "the C accessor symbol is built from the declaring type, not the root"
    );

    let metadata = find_field_path("ScrapeResult", "metadata", &types).expect("metadata is a direct field");
    assert_eq!(metadata.path, "metadata");
    assert_eq!(metadata.owner_type, "ScrapeResult");

    assert!(find_field_path("ScrapeResult", "nowhere", &types).is_none());
}

/// The `pipeline_regeneration_gate` shape: `CompletionResponse.metadata -> Metadata`,
/// `Metadata.document -> Document`, `Document.title`. `Metadata` deliberately has NO
/// `title` field, so `metadata.title` only resolves through the
/// `"metadata.title" = "metadata.document.title"` alias. ~keep
fn completion_response_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "CompletionResponse".into(),
            fields: vec![
                FieldDef {
                    name: "id".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "metadata".into(),
                    ty: TypeRef::Named("Metadata".into()),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metadata".into(),
            fields: vec![FieldDef {
                name: "document".into(),
                ty: TypeRef::Named("Document".into()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".into(),
            fields: vec![FieldDef {
                name: "title".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn completion_response_c_types() -> HashMap<String, String> {
    HashMap::from([
        ("completion_response.metadata".to_string(), "Metadata".to_string()),
        ("metadata.document".to_string(), "Document".to_string()),
    ])
}

fn walk_completion_response(
    resolved: &str,
    raw_field: &str,
    fields_c_types: &HashMap<String, String>,
) -> anyhow::Result<(String, Option<NestedLeafOutcome>)> {
    walk_completion_response_with_sources(resolved, raw_field, fields_c_types, &global_sources())
}

fn walk_completion_response_with_sources(
    resolved: &str,
    raw_field: &str,
    fields_c_types: &HashMap<String, String>,
    config_sources: &FieldConfigSources,
) -> anyhow::Result<(String, Option<NestedLeafOutcome>)> {
    let mut output = String::new();
    let mut handles = Vec::new();
    let leaf = emit_nested_accessor(
        &mut output,
        "gatelib",
        resolved,
        "metadata_title",
        "result",
        fields_c_types,
        &HashSet::new(),
        &mut handles,
        "CompletionResponse",
        raw_field,
        &completion_response_types(),
        config_sources,
    )?;
    Ok((output, leaf))
}

/// The decisive case. Dropping the `[crates.e2e.fields]` alias leaves the fixture
/// asserting `metadata.title`, whose leaf names no field of `Metadata`. Before this
/// check the walk emitted `gatelib_metadata_title(metadata_handle)` — a symbol cbindgen
/// never generates — and generation reported success, so the assertion was lost with no
/// error, no warning and no skip comment for the
/// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` scan to find. ~keep
#[test]
fn unknown_leaf_field_is_an_error_not_a_phantom_accessor() {
    let error = walk_completion_response("metadata.title", "metadata.title", &completion_response_c_types())
        .expect_err("`title` is not a field of `Metadata`");

    let message = error.to_string();
    assert!(
        message.contains("IR type `Metadata` has no field `title`"),
        "must name the type and the field it lacks: {message}"
    );
    assert!(
        message.contains("gatelib_metadata_title()"),
        "must name the phantom symbol it refused to emit: {message}"
    );
    assert!(
        message.contains("only inspects a path's FIRST segment"),
        "must say why nothing upstream caught it: {message}"
    );
}

/// The remedy has to be spelled out, not implied: the fix for this shape is the alias,
/// and the message must carry both sides of it.
#[test]
fn unknown_leaf_field_diagnostic_spells_the_alias_that_fixes_it() {
    let message = walk_completion_response("metadata.title", "metadata.title", &completion_response_c_types())
        .expect_err("`title` is not a field of `Metadata`")
        .to_string();

    assert!(
        message.contains("\"metadata.title\" = \"metadata.document.title\""),
        "must spell the alias that reconnects the fixture path: {message}"
    );
    assert!(
        message.contains("`[crates.e2e.fields]`"),
        "must name the table the alias goes in: {message}"
    );
    assert!(
        message.contains("gatelib_document_title()"),
        "must name the accessor that really exists: {message}"
    );
}

/// The `fields` sibling of the per-call `result_fields` test above: a per-call `fields`
/// override REPLACES the global alias table outright, so the leaf diagnostic's alias-fix
/// branch must name that call's own key too -- not just the intermediate-hop diagnostic's
/// identical branch tested above.
#[test]
fn unknown_leaf_field_diagnostic_names_the_per_call_fields_when_that_is_what_shadows() {
    let sources = FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::PerCall("[crates.e2e.calls.complete]".to_string()),
    };
    let message = walk_completion_response_with_sources(
        "metadata.title",
        "metadata.title",
        &completion_response_c_types(),
        &sources,
    )
    .expect_err("`title` is not a field of `Metadata`")
    .to_string();

    assert!(
        message.contains("\"metadata.title\" = \"metadata.document.title\" under `[crates.e2e.calls.complete].fields`"),
        "must name the per-call key that actually governs this call: {message}"
    );
    assert!(
        !message.contains("`[crates.e2e.fields]`"),
        "must not point at the global key when a per-call override shadows it: {message}"
    );
}

/// Positive control: with the alias in place the very same fixture field resolves, and
/// the leaf still renders its accessor. The fix must not turn every nested assertion
/// into a failure.
#[test]
fn resolvable_leaf_still_renders_its_accessor() {
    let (output, leaf) = walk_completion_response(
        "metadata.document.title",
        "metadata.title",
        &completion_response_c_types(),
    )
    .expect("every hop and the leaf resolve");

    assert_eq!(
        leaf, None,
        "a plain string leaf is a char*, not a primitive or a handle"
    );
    assert!(
        output.contains("char* metadata_title = gatelib_document_title(document_handle);"),
        "{output}"
    );
}

/// A leaf the operator declared in `[crates.e2e.fields_c_types]` is an explicit claim
/// that the accessor exists, and stays authoritative — the IR check only governs the
/// undeclared default. Without this escape hatch a field reached through a C type the
/// IR does not model would become ungeneratable.
#[test]
fn explicitly_declared_leaf_type_overrides_the_ir_check() {
    let mut fields_c_types = completion_response_c_types();
    fields_c_types.insert("metadata.title".to_string(), "char*".to_string());

    let (output, _) = walk_completion_response("metadata.title", "metadata.title", &fields_c_types)
        .expect("an explicit fields_c_types declaration is authoritative");

    assert!(
        output.contains("char* metadata_title = gatelib_metadata_title(metadata_handle);"),
        "{output}"
    );
}

/// Default-allow guard: when the walk is standing on a type the IR does not declare,
/// the IR cannot say whether the leaf exists, and silence must not be read as absence.
#[test]
fn leaf_on_a_type_the_ir_does_not_declare_is_not_rejected() {
    let mut output = String::new();
    let mut handles = Vec::new();
    emit_nested_accessor(
        &mut output,
        "gatelib",
        "metadata.title",
        "metadata_title",
        "result",
        &HashMap::from([("unmodelled_result.metadata".to_string(), "AlsoUnmodelled".to_string())]),
        &HashSet::new(),
        &mut handles,
        "UnmodelledResult",
        "metadata.title",
        &completion_response_types(),
        &global_sources(),
    )
    .expect("an unmodelled parent type must not be treated as proof the leaf is absent");

    assert!(
        output.contains("char* metadata_title = gatelib_also_unmodelled_title(metadata_handle);"),
        "{output}"
    );
}

/// The shape found shipped in `tree-sitter-language-pack/e2e/c/test_data_extraction.c`:
/// `ProcessResult.data -> DataNode.kind`, asserted as `data.kind`, with `data` absent
/// from `result_fields`. Stripping reduces the path to the bare leaf `kind`, which the
/// availability oracle accepts because `kind` is IR-reachable on *some* type, and the
/// flat branch then emits `ts_pack_process_result_kind()` — a symbol the generated
/// header does not declare. ~keep
fn ts_pack_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ProcessResult".into(),
            fields: vec![
                FieldDef {
                    name: "language".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "data".into(),
                    ty: TypeRef::Named("DataNode".into()),
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
    ]
}

fn check_ts_pack_stripped_leaf(
    declared_in_fields_c_types: bool,
    result_fields_source: &EffectiveConfigSource,
) -> anyhow::Result<()> {
    let types = ts_pack_types();
    ensure_leaf_field_exists(LeafFieldCheck {
        prefix: "ts_pack",
        accessor_fn: "ts_pack_process_result_kind",
        resolved: "kind",
        raw_field: "data.kind",
        segment: "kind",
        parent_snake_type: "process_result",
        parent_is_ir_type: true,
        declared_in_fields_c_types,
        result_type_name: "ProcessResult",
        type_defs: &types,
        result_fields_source,
        // Irrelevant to what this helper's callers assert on -- all of them exercise
        // the namespace-stripped-identity branch, which only reads
        // `result_fields_source`. Global is the neutral default. ~keep
        fields_source: &EffectiveConfigSource::Global,
    })
}

#[test]
fn namespace_stripped_leaf_that_is_not_a_result_type_field_is_rejected() {
    let message = check_ts_pack_stripped_leaf(false, &EffectiveConfigSource::Global)
        .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
        .to_string();

    assert!(
        message.contains("IR type `ProcessResult` has no field `kind`"),
        "must name the type the accessor would have been called on: {message}"
    );
    assert!(
        message.contains("stripped the leading \"data\""),
        "must name the stripping that produced the bare leaf: {message}"
    );
    assert!(
        message.contains("ts_pack_data_node_kind()"),
        "must name the accessor that really exists: {message}"
    );
}

/// The remedy differs from the aliasable case and the message must not confuse them: an
/// alias here would be `"data.kind" = "data.kind"`, an identity mapping that leaves
/// `namespace_stripped_path` (which reads `result_fields`, not the alias table) stripping
/// exactly as before. This is the global-in-effect case: no per-call override, so the
/// global key really is the one an edit reaches.
#[test]
fn stripped_leaf_diagnostic_names_result_fields_not_an_identity_alias() {
    let message = check_ts_pack_stripped_leaf(false, &EffectiveConfigSource::Global)
        .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
        .to_string();

    assert!(
        message.contains("add \"data\" to `[crates.e2e].result_fields`"),
        "must name the config entry that stops the stripping: {message}"
    );
    assert!(
        !message.contains("\"data.kind\" = \"data.kind\""),
        "must not suggest an identity alias that changes nothing: {message}"
    );
}

/// The defect this type exists to prevent: a per-call `result_fields` override
/// REPLACES the global default outright (`E2eConfig::effective_result_fields`), so
/// when a per-call override is what's in effect, the "Fix:" must name that call's own
/// key -- never the global one, which a consumer reported editing to no effect
/// because their call's per-call list is what actually governed the walk.
#[test]
fn stripped_leaf_diagnostic_names_the_per_call_result_fields_when_that_is_what_shadows() {
    let source = EffectiveConfigSource::PerCall("[crates.e2e.calls.crawl]".to_string());
    let message = check_ts_pack_stripped_leaf(false, &source)
        .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
        .to_string();

    assert!(
        message.contains("add \"data\" to `[crates.e2e.calls.crawl].result_fields`"),
        "must name the per-call key that actually governs this call: {message}"
    );
    assert!(
        !message.contains("`[crates.e2e].result_fields`"),
        "must not point at the global key when a per-call override shadows it: {message}"
    );
}

/// The unnamed default call (`[crates.e2e.call]`) can also carry its own
/// `result_fields` override -- it is looked up the same way a named call is, just
/// with no entry in `e2e_config.calls` to match by pointer. The message must still
/// name it, not fall back to claiming it's the global key.
#[test]
fn describe_effective_config_source_names_the_unnamed_default_call() {
    let e2e_config = E2eConfig::default();
    let call = CallConfig {
        result_fields: HashSet::from(["pages".to_string()]),
        ..CallConfig::default()
    };

    let source = describe_effective_config_source(&e2e_config, &call, !call.result_fields.is_empty());

    match source {
        EffectiveConfigSource::PerCall(label) => assert_eq!(label, "[crates.e2e.call]"),
        EffectiveConfigSource::Global => panic!("call_has_override == true must never resolve to Global"),
    }
}

/// The common case: a named call in `[crates.e2e.calls]` with its own override must be
/// identified by that name, so the operator can find the exact TOML table to edit.
#[test]
fn describe_effective_config_source_names_a_call_matched_by_pointer_identity() {
    let mut e2e_config = E2eConfig::default();
    let crawl_call = CallConfig {
        result_fields: HashSet::from(["pages".to_string()]),
        ..CallConfig::default()
    };
    e2e_config.calls.insert("crawl".to_string(), crawl_call);

    let source = describe_effective_config_source(&e2e_config, &e2e_config.calls["crawl"], true);

    match source {
        EffectiveConfigSource::PerCall(label) => assert_eq!(label, "[crates.e2e.calls.crawl]"),
        EffectiveConfigSource::Global => panic!("call_has_override == true must never resolve to Global"),
    }
}

/// `call_has_override == false` always resolves to the global default, regardless of
/// whether `call` is named or the unnamed default call -- the caller-computed
/// emptiness check is authoritative, the function never re-derives it.
#[test]
fn describe_effective_config_source_is_global_when_the_caller_says_there_is_no_override() {
    let e2e_config = E2eConfig::default();
    let call = CallConfig {
        result_fields: HashSet::from(["pages".to_string()]),
        ..CallConfig::default()
    };

    assert!(matches!(
        describe_effective_config_source(&e2e_config, &call, false),
        EffectiveConfigSource::Global
    ));
}

/// `FieldConfigSources::resolve` is the one place production code should call this
/// from: it derives `call_has_override` itself, once per collection, so the two
/// checks (`result_fields`, `fields`) cannot drift onto different emptiness logic.
#[test]
fn field_config_sources_resolve_derives_each_collection_independently() {
    let mut e2e_config = E2eConfig::default();
    let call = CallConfig {
        result_fields: HashSet::from(["pages".to_string()]),
        // `fields` left empty: only `result_fields` has a per-call override.
        ..CallConfig::default()
    };
    e2e_config.calls.insert("crawl".to_string(), call);

    let sources = FieldConfigSources::resolve(&e2e_config, &e2e_config.calls["crawl"]);

    assert!(
        matches!(sources.result_fields, EffectiveConfigSource::PerCall(ref label) if label == "[crates.e2e.calls.crawl]")
    );
    assert!(matches!(sources.fields, EffectiveConfigSource::Global));
}

#[test]
fn explicitly_declared_flat_leaf_type_overrides_the_ir_check() {
    check_ts_pack_stripped_leaf(true, &EffectiveConfigSource::Global)
        .expect("an explicit fields_c_types declaration is authoritative");
}

mod args;
mod optional_enum_leaf;
