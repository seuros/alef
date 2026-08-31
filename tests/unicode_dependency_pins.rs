use std::path::Path;

fn dependency_requirement<'a>(dependencies: &'a toml::Table, name: &str) -> &'a str {
    let dependency = dependencies
        .get(name)
        .unwrap_or_else(|| panic!("Cargo.toml must declare {name}"));
    dependency
        .as_str()
        .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
        .unwrap_or_else(|| panic!("{name} must declare a version requirement"))
}

#[test]
fn unicode_tables_are_exactly_pinned() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read Alef Cargo.toml");
    let parsed: toml::Value = toml::from_str(&manifest).expect("parse Alef Cargo.toml");
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml has dependencies");

    for (name, expected) in [("icu_casemap", "=2.3.0"), ("unicode-general-category", "=1.1.0")] {
        let actual = semver::VersionReq::parse(dependency_requirement(dependencies, name))
            .unwrap_or_else(|error| panic!("parse {name} requirement: {error}"));
        let expected = semver::VersionReq::parse(expected).expect("parse expected exact requirement");
        assert_eq!(actual, expected, "{name} must remain reproducibly pinned");
    }
}
