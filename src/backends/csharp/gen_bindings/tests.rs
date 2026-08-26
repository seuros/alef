//! Unit tests for the C# binding emitter, split out of `gen_bindings/mod.rs`.

use super::*;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, PrimitiveType, TypeDef};

fn make_method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        cfg: None,
        ..MethodDef::default()
    }
}

fn ocr_shaped_trait() -> TypeDef {
    TypeDef {
        name: "OcrBackend".to_string(),
        rust_path: "sample_core::OcrBackend".to_string(),
        is_trait: true,
        methods: vec![
            make_method("supports_language", TypeRef::Primitive(PrimitiveType::Bool)),
            make_method("backend_type", TypeRef::Named("OcrBackendType".to_string())),
            make_method("supported_languages", TypeRef::Vec(Box::new(TypeRef::String))),
        ],
        ..TypeDef::default()
    }
}

fn ocr_bridge_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

/// `ocr_bridge_config`, rooted at an absolute temp output directory so `base_path` lands
/// inside `temp` instead of resolving `packages/csharp/` against the test process's working
/// directory — which is the alef checkout itself, and is where the deleting version of this
/// code aimed `fs::remove_file` every time the existing suite ran. ~keep
fn temp_rooted_bridge_config(temp: &std::path::Path) -> ResolvedCrateConfig {
    let mut config = ocr_bridge_config();
    config.name = "sample".to_string();
    config.trait_bridges[0].context_type = Some("VisitContext".to_string());
    config.trait_bridges[0].result_type = Some("VisitOutcome".to_string());
    config.output_paths.insert("csharp".to_string(), temp.to_path_buf());
    config
}

/// Every file and directory under `root`, keyed by path; `None` marks a directory so a newly
/// created empty directory is as visible as a written file.
fn snapshot_tree(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut snapshot = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                snapshot.insert(path.clone(), None);
                stack.push(path);
            } else {
                snapshot.insert(path.clone(), Some(std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    snapshot
}

/// The load-bearing regression. `config.ffi` is an `Option`, and `generate_bindings` read an
/// absent `[ffi]` section as `visitor_callbacks == false`, taking an else-branch that
/// `fs::remove_file`d `IVisitor.cs`, `VisitorCallbacks.cs`, and a class per configured
/// bridge `context_type`/`result_type` — names that come out of the consumer's own config.
///
/// Asserting that the flag resolves to `false` would pass with the deletion still in place,
/// so this asserts on the filesystem. The seeded set is taken from `stale_visitor_filenames`
/// itself, deliberately: that is the exact blast radius the delete had, so the test cannot
/// drift narrower than the thing it guards, and a future entry added to that list is covered
/// the day it is added rather than the day someone remembers to extend a literal here. ~keep
#[test]
fn absent_ffi_section_deletes_no_visitor_files() {
    let temp = tempfile::tempdir().expect("temp output root");
    let config = temp_rooted_bridge_config(temp.path());
    assert!(
        config.ffi.is_none(),
        "sanity: this test is only about the absent-[ffi] branch"
    );

    let victims = files::stale_visitor_filenames(&config);
    assert!(
        victims.len() > 2,
        "sanity: the blast radius must include the consumer-named context/result classes, not \
         just the two hardcoded support files; got {victims:?}"
    );

    let base_path = temp.path().join(config.csharp_namespace());
    std::fs::create_dir_all(&base_path).expect("namespace directory");
    for filename in &victims {
        std::fs::write(base_path.join(filename), "// hand written\n").expect("seed victim file");
    }

    let api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![ocr_shaped_trait()],
        ..ApiSurface::default()
    };
    CsharpBackend
        .generate_bindings(&api, &config)
        .expect("C# bindings must render");

    for filename in &victims {
        let path = base_path.join(filename);
        assert_eq!(
            std::fs::read_to_string(&path).ok().as_deref(),
            Some("// hand written\n"),
            "{} must survive a render with no [ffi] section, byte for byte",
            path.display()
        );
    }
}

/// `bin_cli::helpers::collect_managed_surface` documents this stage as "a pure in-memory
/// render; nothing here writes to disk", and `alef verify`, `alef adopt`, and `alef diff` are
/// safe to run on a consumer's tree only because of that. The sibling test above checks the
/// visitor filenames specifically and would still pass if some other path in the backend
/// started writing or unlinking, so this one asserts the property the purity claim actually
/// makes: the output tree is bit-identical across the call. Scope is the configured output
/// root, which is where every path this backend constructs points. ~keep
#[test]
fn render_stage_writes_nothing_under_the_output_root() {
    let temp = tempfile::tempdir().expect("temp output root");
    let config = temp_rooted_bridge_config(temp.path());

    let base_path = temp.path().join(config.csharp_namespace());
    std::fs::create_dir_all(&base_path).expect("namespace directory");
    for filename in files::stale_visitor_filenames(&config) {
        std::fs::write(base_path.join(filename), "// hand written\n").expect("seed file");
    }
    std::fs::write(base_path.join("NativeMethods.cs"), "// stale generated\n").expect("seed emitted path");

    let before = snapshot_tree(temp.path());
    assert!(
        before.len() > 4,
        "sanity: an empty tree would make the comparison below vacuous; got {before:?}"
    );

    let api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![ocr_shaped_trait()],
        ..ApiSurface::default()
    };
    CsharpBackend
        .generate_bindings(&api, &config)
        .expect("C# bindings must render");

    assert_eq!(
        snapshot_tree(temp.path()),
        before,
        "generate_bindings must not create, modify, or remove anything under the output root"
    );
}

/// The report replacing the delete has to name the paths a human is being asked to check, and
/// has to report only what is actually there — a candidate list is not a finding. ~keep
#[test]
fn unemitted_visitor_files_are_reported_not_removed() {
    let temp = tempfile::tempdir().expect("temp output root");
    let base_path = temp.path();
    std::fs::write(base_path.join("IVisitor.cs"), "// hand written\n").expect("seed present file");

    let reported = files::report_unemitted_visitor_files(
        base_path,
        &["IVisitor.cs".to_string(), "VisitorCallbacks.cs".to_string()],
        &std::collections::HashSet::new(),
    );

    assert_eq!(
        reported,
        vec![base_path.join("IVisitor.cs")],
        "only the file that exists is reported"
    );
    assert!(
        base_path.join("IVisitor.cs").is_file(),
        "reporting must leave the file on disk"
    );
}

/// A file this very run is emitting is not an unemitted file.
///
/// ~keep The check was `path.is_file()` alone, evaluated before the type and enum emitters had
/// pushed anything. In the branch where visitor callbacks are off -- which includes a consumer
/// simply having no `[ffi]` section, since `unwrap_or(false)` cannot distinguish that from an
/// explicit `false` -- the candidates are `{context_type}.cs` and `{result_type}.cs` from
/// `[[trait_bridges]]`, and those emitters go on to write exactly those files. So every
/// generate, adopt, verify and diff on such a repo reported files the same run had emitted.
#[test]
fn a_file_this_run_emits_is_not_reported_as_unemitted() {
    let temp = tempfile::tempdir().expect("temp output root");
    let base_path = temp.path();
    std::fs::write(base_path.join("NodeContext.cs"), "// emitted last run\n").expect("seed");
    std::fs::write(base_path.join("IVisitor.cs"), "// hand written\n").expect("seed");

    let emitted = std::collections::HashSet::from([base_path.join("NodeContext.cs")]);
    let reported = files::report_unemitted_visitor_files(
        base_path,
        &["NodeContext.cs".to_string(), "IVisitor.cs".to_string()],
        &emitted,
    );

    assert_eq!(
        reported,
        vec![base_path.join("IVisitor.cs")],
        "a path this run is writing must be excluded; only the genuinely unemitted one remains"
    );
}

/// Ordered slot-comment identities the bridge class writes, e.g. `["name_fn", "backend_type_fn"]`.
fn emitted_slot_comments(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("// Slot "))
        .filter_map(|rest| rest.split_once(": "))
        .map(|(_, name)| name.to_string())
        .collect()
}

/// Regression: `[crates.csharp].exclude_types` (and every other source feeding
/// `effective_exclude_types`, here `binding_excluded`) must not delete a trait method from
/// the bridge. The method keeps a slot in the Rust vtable struct, so deleting it here
/// leaves C# allocating and writing N-1 function pointers into an N-slot struct.
#[test]
fn excluded_return_type_does_not_remove_a_vtable_slot() {
    let api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![
            ocr_shaped_trait(),
            TypeDef {
                name: "OcrBackendType".to_string(),
                binding_excluded: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let files = CsharpBackend
        .generate_bindings(&api, &ocr_bridge_config())
        .expect("C# bindings");
    let bridges = files
        .iter()
        .find(|file| file.path.ends_with("TraitBridges.cs"))
        .expect("TraitBridges.cs");

    assert_eq!(
        emitted_slot_comments(&bridges.content),
        vec![
            "name_fn",
            "version_fn",
            "initialize_fn",
            "shutdown_fn",
            "supports_language_fn",
            "backend_type_fn",
            "supported_languages_fn",
            "free_string",
            "free_user_data",
        ],
        "every Rust vtable field must get a slot, at its own index"
    );
    assert!(
        bridges.content.contains("Marshal.AllocHGlobal(IntPtr.Size * 9)"),
        "the block must stay as wide as the Rust vtable struct;\nactual:\n{}",
        bridges.content
    );
    assert!(
        bridges.content.contains("string BackendType { get; }"),
        "an excluded return type degrades to a JSON string rather than removing the method;\nactual:\n{}",
        bridges.content
    );
}

#[test]
fn vtable_slot_check_accepts_a_faithful_bridge() {
    let trait_def = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![trait_def.clone()],
        ..ApiSurface::default()
    };
    let emitted = crate::codegen::generators::trait_bridge::vtable_slot_names(&trait_def, true, &[]);

    assert_vtable_matches_rust_struct(&api, &trait_def, true, &[], &emitted).expect("matching slot lists must pass");
}

#[test]
fn vtable_slot_check_rejects_a_dropped_slot() {
    let source_trait = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![source_trait.clone()],
        ..ApiSurface::default()
    };
    let mut pruned_trait = source_trait.clone();
    pruned_trait.methods.retain(|method| method.name != "backend_type");
    let emitted = crate::codegen::generators::trait_bridge::vtable_slot_names(&pruned_trait, true, &[]);

    let error = assert_vtable_matches_rust_struct(&api, &pruned_trait, true, &[], &emitted)
        .expect_err("a bridge missing a slot must fail generation");
    let message = error.to_string();
    assert!(
        message.contains("Rust slots (9)") && message.contains("C# slots (8)"),
        "the failure must report both slot counts;\nactual:\n{message}"
    );
    assert!(
        message.contains("backend_type"),
        "the failure must name the slot that disagrees;\nactual:\n{message}"
    );
}

#[test]
fn vtable_slot_check_rejects_a_reordered_slot() {
    let trait_def = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![trait_def.clone()],
        ..ApiSurface::default()
    };
    let mut reordered = crate::codegen::generators::trait_bridge::vtable_slot_names(&trait_def, true, &[]);
    reordered.swap(5, 6);

    let error = assert_vtable_matches_rust_struct(&api, &trait_def, true, &[], &reordered)
        .expect_err("a bridge with the right slot count in the wrong order must fail generation");
    let message = error.to_string();
    assert!(
        message.contains("Rust slots (9)") && message.contains("C# slots (9)"),
        "a reordering keeps the count, so the counts alone must not be what fails;\nactual:\n{message}"
    );
    assert!(
        message.contains("backend_type, supported_languages") && message.contains("supported_languages, backend_type"),
        "the failure must show both orders so the swapped pair is identifiable;\nactual:\n{message}"
    );
}

/// A skipped method is absent from the Rust vtable struct, so an emitter that still writes
/// a slot for it must fail generation rather than shift every later function pointer.
#[test]
fn vtable_slot_check_rejects_a_slot_for_a_skipped_method() {
    let trait_def = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![trait_def.clone()],
        ..ApiSurface::default()
    };
    let skip = vec!["backend_type".to_string()];
    let over_counted = crate::codegen::generators::trait_bridge::vtable_slot_names(&trait_def, true, &[]);

    let error = assert_vtable_matches_rust_struct(&api, &trait_def, true, &skip, &over_counted)
        .expect_err("an extra slot must fail generation");
    let message = error.to_string();
    assert!(
        message.contains("Rust slots (8)") && message.contains("C# slots (9)"),
        "the failure must report both slot counts;\nactual:\n{message}"
    );
}
