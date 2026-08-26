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

fn test_backend_arg(trait_name: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "backend".into(),
        field: "backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some(trait_name.to_string()),
    }
}

/// Pin: a `test_backend` arg whose trait IS registered still panics today,
/// because `c::emit_test_backend` (`trait_bridge_snippet.rs`) is unimplemented —
/// see its doc comment for why. `emit_test_backend` panics before ever handing
/// `build_args_string_c` a value, so there is no sentinel left to accidentally
/// splice into the call's argument list. This is the regression guard: it fails
/// if that panic is ever replaced with a placeholder return and the call site
/// stops checking it.
#[test]
#[should_panic(expected = "test-backend emitter is unimplemented")]
fn registered_test_backend_trait_panics_because_c_backend_is_unimplemented() {
    use crate::core::config::TraitBridgeConfig;

    let bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".into(),
        ..TraitBridgeConfig::default()
    };
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge],
        ..ResolvedCrateConfig::default()
    };
    let fixture = Fixture {
        id: "register_sample_backend".into(),
        ..Fixture::default()
    };
    let args = vec![test_backend_arg("SampleBackend")];

    let _ = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "register_sample_backend",
        TargetParams::IrAbsent,
    );
}

/// An unregistered trait (no matching `[[crates.trait_bridges]]` entry) has no
/// vtable to point at — generation must fail loudly instead of falling back to
/// `NULL`. Unlike Kotlin's non-null interface parameter, nothing in C's type
/// system would catch a bad `NULL` default at compile time, so this loud check
/// is the only thing standing between a misconfigured `alef.toml` and either an
/// uncompilable comment or a `NULL` vtable pointer reaching generated C.
#[test]
#[should_panic(expected = "no `[[crates.trait_bridges]]` entry")]
fn unregistered_test_backend_trait_panics_instead_of_falling_back_to_null() {
    let config = ResolvedCrateConfig::default();
    let fixture = Fixture {
        id: "register_sample_backend".into(),
        ..Fixture::default()
    };
    let args = vec![test_backend_arg("SampleBackend")];

    let _ = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "register_sample_backend",
        TargetParams::IrAbsent,
    );
}

/// Regression for the bug that shipped a `char[37]` literal against a
/// `TS_PACKAlefHandle` (an `int32_t`) parameter: with no `args` configured, alef
/// used to splice the fixture's whole `input` JSON as a single C string literal
/// regardless of the target's real parameters, which cannot compile against
/// anything the target actually takes. A genuinely zero-argument target
/// (`TargetParams::Known(&[])`) is the one case that must keep emitting an empty
/// argument list rather than refuse. ~keep
#[test]
fn should_emit_empty_parens_when_args_unconfigured_and_target_takes_no_parameters() {
    let fixture = Fixture {
        id: "list_ocr_backends".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let result = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "list_ocr_backends",
        TargetParams::Known(&[]),
    )
    .expect("a genuinely zero-argument target must not fail generation");

    assert_eq!(
        result, "",
        "a zero-argument call must emit `()`, not a fabricated literal"
    );
}

/// The actual defect this guards: `ts_pack_configure` takes one typed parameter
/// (`config`, an opaque handle), but the fixture configured no `args`. Splicing the
/// whole fixture `input` JSON as one C string literal produced
/// `ts_pack_configure("{\"cache_dir\":...}")` against `int32_t
/// ts_pack_configure(TS_PACKAlefHandle config)` -- an incompatible
/// pointer-to-integer conversion that does not compile. The emitter must refuse
/// with a diagnostic instead of guessing an argument it cannot construct. ~keep
#[test]
fn should_refuse_when_args_unconfigured_and_target_takes_a_typed_parameter() {
    let fixture = Fixture {
        id: "pack_configure_defaults".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let params = [ParamDef {
        name: "config".into(),
        ..ParamDef::default()
    }];

    let error = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "ts_pack_configure",
        TargetParams::Known(&params),
    )
    .expect_err("a known non-empty parameter list must not be papered over with a JSON literal")
    .to_string();

    assert!(
        !error.contains("cache_dir"),
        "must not leak the fixture JSON into a diagnostic that replaces splicing it: {error}"
    );
    assert!(error.contains("ts_pack_configure"), "must name the call: {error}");
    assert!(error.contains("config"), "must name the unfilled parameter: {error}");
    assert!(error.contains("args"), "must point at the `args` config knob: {error}");
}

/// When the IR signature cannot be resolved at all, the emitter has no basis to
/// tell a genuine zero-argument call from an authoring gap -- refuse rather than
/// guess, per the same principle `ResultTypeName::require` applies to result types.
#[test]
fn should_refuse_when_args_unconfigured_and_target_signature_is_unresolvable() {
    let fixture = Fixture {
        id: "mystery_call".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let error = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "mystery_fn",
        TargetParams::Unresolvable,
    )
    .expect_err("an unresolvable signature must not fall back to guessing")
    .to_string();

    assert!(error.contains("mystery_fn"), "must name the call: {error}");
    assert!(error.contains("args"), "must point at the `args` config knob: {error}");
}

/// The boundary between the two refusing cases and the one that must not refuse.
///
/// `IrAbsent` means no IR was consulted at all -- the main e2e test-file emitter has no
/// `CallIr`, and several snippet entry points render without one. Nothing was learned, so
/// nothing can be concluded, and this keeps the pre-existing behaviour instead of failing.
/// Collapsing it back into `Unresolvable` would fail generation for every IR-less caller,
/// which is a far wider blast radius than the defect this guards, and it would put this
/// half of the fix in direct contradiction with `unresolved_result_type_name`, which
/// classifies an absent IR as `Unverified` for exactly the same reason. Both halves must
/// agree on what an absent IR licenses, or one of them is wrong. ~keep
#[test]
fn should_keep_prior_behaviour_when_there_is_no_ir_to_consult() {
    let fixture = Fixture {
        id: "no_ir".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let rendered = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "sample_fn",
        TargetParams::IrAbsent,
    )
    .expect("an absent IR must not fail generation on a path that never had a signature");

    assert_eq!(
        rendered,
        json_to_c(&fixture.input),
        "with no IR consulted the emitter must render exactly what it rendered before"
    );
}

/// The load-bearing control: a call WITH properly configured `args` must keep
/// emitting them, unchanged, real typed literal and all. Without this test, a fix
/// that makes the empty-`args` path refuse (or always emit `()`) everywhere would
/// pass the two tests above and look correct while quietly breaking every snippet
/// that already configures `args` correctly -- the two failure modes above only
/// ever trigger on `args.is_empty()`, so nothing else in this suite would catch a
/// regression that clobbers the non-empty path too. ~keep
#[test]
fn should_still_emit_configured_args_unchanged_when_args_are_present() {
    let fixture = Fixture {
        id: "chat_basic".into(),
        input: serde_json::json!({"text": "hello"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![crate::e2e::config::ArgMapping {
        name: "text".into(),
        field: "text".into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];

    // `TargetParams::Unresolvable` on purpose: an unresolved signature licenses no claim
    // about any parameter's type, so a configured `args` list must render exactly as it
    // always did. (A resolved signature does license one -- see
    // `should_refuse_a_string_literal_configured_against_a_handle_parameter` and its
    // correctly-typed control below.)
    let result = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "chat",
        TargetParams::Unresolvable,
    )
    .expect("configured args must still render");

    assert_eq!(
        result, "\"hello\"",
        "a configured string arg must still emit its real typed literal"
    );
}

fn string_arg(name: &str, field: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// The other half of the same defect. The refusals above all key on `args.is_empty()` --
/// "no args configured, do not fabricate an argument list". This is the opposite case:
/// `args` are present, so the arity is satisfied and nothing refuses, but the entry's type
/// contradicts the parameter's. `json_to_c` stringifies the JSON object and the emitter
/// splices a `char[]` literal into a parameter the C ABI exports as `AlefHandle` -- the
/// same `-Wint-conversion` failure, reached without ever passing through the empty-`args`
/// guard. ~keep
#[test]
fn should_refuse_a_string_literal_configured_against_a_handle_parameter() {
    let fixture = Fixture {
        id: "configure_cache_dir".into(),
        input: serde_json::json!({"config": {"cache_dir": "/tmp/sample_cache"}}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("config", "config")];
    let params = [ParamDef {
        name: "config".into(),
        ty: TypeRef::Named("SampleConfig".into()),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "SampleConfig".into(),
        has_serde: true,
        ..TypeDef::default()
    }];

    let error = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &type_defs,
        &fixture,
        "sample_configure",
        TargetParams::Known(&params),
    )
    .expect_err("a JSON object must not be lowered into a handle parameter")
    .to_string();

    assert!(error.contains("sample_configure"), "must name the call: {error}");
    assert!(error.contains("`config`"), "must name the parameter: {error}");
    assert!(
        error.contains("AlefHandle"),
        "must name the parameter's C type: {error}"
    );
    assert!(
        error.contains("cache_dir"),
        "must quote the offending value so the operator can find the entry: {error}"
    );
    assert!(
        error.contains("json_object"),
        "must name the configuration that constructs the handle: {error}"
    );
}

/// The false-refusal boundary, and the reason this check cannot simply reject every JSON
/// object. A `Vec<Named>` parameter does NOT cross the C ABI as a handle -- `type_map`'s
/// `c_param_type` maps it to `*const c_char`, a JSON string -- so the stringified literal
/// is exactly the right lowering there. Refusing it would delete correct, compiling
/// documentation, which is why `handle_param_type_name` deliberately does not unwrap
/// through `Vec` the way `c.rs`'s `named_type` does. ~keep
#[test]
fn should_not_refuse_a_json_literal_against_a_vec_parameter() {
    let fixture = Fixture {
        id: "rank_documents".into(),
        input: serde_json::json!({"documents": ["alpha", "beta"]}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("documents", "documents")];
    let params = [ParamDef {
        name: "documents".into(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".into()))),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "Document".into(),
        has_serde: true,
        ..TypeDef::default()
    }];

    let rendered = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &type_defs,
        &fixture,
        "sample_rank",
        TargetParams::Known(&params),
    )
    .expect("a JSON-string parameter must keep rendering its literal");

    assert_eq!(
        rendered,
        json_to_c(&fixture.input["documents"]),
        "a `Vec<T>` parameter crosses as a JSON `const char *`, so the literal is correct"
    );
}

/// A parameter type the IR names but carries no `TypeDef` for cannot be proven to be a
/// handle: an IR enum is an `EnumDef`, never a `TypeDef`, and enum-typed `Named` parameters
/// cross as `i32`. Refusing on the name alone would reject every enum argument on evidence
/// the emitter does not have, so an unmatched name leaves the rendering untouched. ~keep
#[test]
fn should_not_refuse_a_named_parameter_the_ir_carries_no_type_def_for() {
    let fixture = Fixture {
        id: "set_level".into(),
        input: serde_json::json!({"level": "debug"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("level", "level")];
    let params = [ParamDef {
        name: "level".into(),
        ty: TypeRef::Named("LogLevel".into()),
        ..ParamDef::default()
    }];

    let rendered = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "sample_set_level",
        TargetParams::Known(&params),
    )
    .expect("a name with no `TypeDef` behind it licenses no claim about the C type");

    assert_eq!(rendered, "\"debug\"", "the rendering must be left exactly as it was");
}
