use super::*;
use crate::backends::java::JavaBackend;
use crate::core::backend::Backend as _;
use heck::ToPascalCase;
use std::collections::BTreeSet;

/// `[ffi] prefix` is deliberately unrelated to `name`: the docs emitter used to build the Java
/// `throws` clause out of the FFI symbol prefix while the Java backend named the class after the
/// crate, so a fixture where the two agree cannot fail no matter how the derivations drift. ~keep
fn agreement_config() -> ResolvedCrateConfig {
    config_from_toml(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample-multi-word"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "smw"
"#,
    )
}

fn agreement_api(config: &ResolvedCrateConfig) -> ApiSurface {
    let mut api = make_minimal_api("2.0.0");
    api.crate_name = config.name.clone();
    let mut engine = empty_type("Engine");
    engine.is_opaque = true;
    engine.methods = vec![make_method(
        "render",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        false,
        None,
    )];
    api.types = vec![engine];
    api.functions = vec![make_function(
        "convert",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::String,
        false,
        None,
    )];
    api
}

/// The single class name every generated Java file declares as `... extends Exception`.
///
/// `infrastructure_exception.jinja` extends *this* class rather than `Exception`, so the suffix
/// match picks out the base exception and nothing else. ~keep
fn declared_exception_classes(files: &[crate::core::backend::GeneratedFile]) -> Vec<String> {
    files
        .iter()
        .flat_map(|file| file.content.lines())
        .filter_map(|line| {
            line.trim()
                .strip_prefix("public class ")
                .and_then(|rest| rest.strip_suffix(" extends Exception {"))
                .map(str::to_string)
        })
        .collect()
}

/// Every `throws <Name>` spelling the generated Java reference page prints.
fn documented_throws_classes(page: &str) -> BTreeSet<String> {
    page.lines()
        .filter_map(|line| line.split_once(" throws "))
        .filter_map(|(_, rest)| rest.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The docs emitter and the Java binding emitter must not derive the exception class name
/// independently: `api-java.md` quotes `throws <Class>` verbatim, so any spelling the backend does
/// not declare names a class that occurs zero times in the generated package. ~keep
#[test]
fn java_docs_throws_clause_names_the_class_the_java_binding_declares() {
    let config = agreement_config();
    let api = agreement_api(&config);

    let binding_files = JavaBackend
        .generate_bindings(&api, &config)
        .expect("java bindings generate");
    let declared = declared_exception_classes(&binding_files);
    assert_eq!(
        declared.len(),
        1,
        "positive control: the Java backend must declare exactly one base exception class, got {declared:?}"
    );
    let exception_class = declared[0].clone();
    assert!(
        !exception_class.is_empty(),
        "the declared exception class name must not be empty"
    );
    assert_ne!(
        exception_class,
        format!("{}RsException", config.ffi_prefix().to_pascal_case()),
        "the fixture's [ffi] prefix must stay distinct from its crate name, otherwise this test \
         passes whether or not the two derivations agree"
    );

    let files = generate_docs(&api, &config, &[Language::Java], "out").expect("docs generate");
    let page = files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == "api-java.md")
        })
        .map(|file| file.content.as_str())
        .expect("missing generated api-java.md");

    let documented = documented_throws_classes(page);
    assert!(
        !documented.is_empty(),
        "positive control: the Java page must document at least one throws clause"
    );
    assert_eq!(
        documented,
        BTreeSet::from([exception_class.clone()]),
        "every documented throws clause must name the exception class the Java binding declares \
         ({exception_class})"
    );
}
