use alef::e2e::codegen::rust::{CargoTomlInputs, render_cargo_toml};

/// The three cases below differ only in dependency mode and version, so they share this base.
fn inputs() -> CargoTomlInputs<'static> {
    CargoTomlInputs {
        crate_name: "my-lib",
        dep_name: "my_lib",
        crate_path: "../../crates/my-lib",
        ..Default::default()
    }
}

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_local_mode() {
    let result = render_cargo_toml(&CargoTomlInputs {
        dep_mode: alef::e2e::config::DependencyMode::Local,
        ..inputs()
    });

    assert!(
        result.contains("[workspace]"),
        "Local mode e2e Cargo.toml must contain an empty [workspace] section so it stands alone"
    );
}

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_registry_mode() {
    let result = render_cargo_toml(&CargoTomlInputs {
        dep_mode: alef::e2e::config::DependencyMode::Registry,
        version: Some("0.1.0"),
        ..inputs()
    });

    assert!(
        result.contains("[workspace]"),
        "Registry mode e2e Cargo.toml must contain an empty [workspace] section so it stands alone"
    );
}

#[test]
fn test_cargo_toml_contains_package_name() {
    let result = render_cargo_toml(&CargoTomlInputs {
        dep_mode: alef::e2e::config::DependencyMode::Local,
        ..inputs()
    });

    assert!(
        result.contains("name = \"my_lib-e2e-rust\""),
        "Cargo.toml should contain the correct package name"
    );
}
