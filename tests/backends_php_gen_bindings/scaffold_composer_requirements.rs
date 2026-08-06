use alef::core::config::Language;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;

/// Minimum PHP each PHPUnit major requires, per packagist. Used to check the scaffolded
/// `require-dev` constraint against the scaffolded `require.php` floor.
const PHPUNIT_MAJOR_MIN_PHP: &[(&str, (u32, u32))] =
    &[("^10", (8, 1)), ("^11", (8, 2)), ("^12", (8, 3)), ("^13", (8, 4))];

fn scaffolded_composer_json() -> String {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "demo_crate"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config must parse");
    let resolved = cfg.resolve().expect("config must resolve");
    let config = &resolved[0];

    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "1.0.0".into(),
        ..Default::default()
    };

    let files = scaffold(&api, config, &[Language::Php]).expect("PHP scaffold must succeed");
    files
        .iter()
        .find(|f| f.path.ends_with("composer.json"))
        .expect("composer.json must be scaffolded")
        .content
        .clone()
}

fn php_floor(composer: &serde_json::Value) -> (u32, u32) {
    let raw = composer["require"]["php"]
        .as_str()
        .expect("composer.json must declare require.php");
    let digits: Vec<u32> = raw
        .trim_start_matches(['>', '=', '^', '~', ' '])
        .split('.')
        .take(2)
        .map(|part| part.parse().expect("require.php must be numeric"))
        .collect();
    (digits[0], digits[1])
}

/// The scaffolded `require-dev` must be installable on the oldest PHP the scaffolded `require`
/// claims to support. PHPUnit 13 needs PHP >= 8.4.1, so pairing a bare `^13.1` with `"php": ">=8.2"`
/// is unsatisfiable: `composer install` hard-fails on 8.2 and 8.3, and Dependabot — which resolves
/// Composer against the declared platform floor rather than the runtime PHP — failed every run on
/// html-to-markdown with "requirements could not be resolved".
#[test]
fn scaffolded_phpunit_constraint_is_installable_at_the_declared_php_floor() {
    let content = scaffolded_composer_json();
    let composer: serde_json::Value = serde_json::from_str(&content).expect("composer.json must be valid JSON");

    let floor = php_floor(&composer);
    let phpunit = composer["require-dev"]["phpunit/phpunit"]
        .as_str()
        .expect("composer.json must declare a phpunit dev dependency");

    let satisfiable = PHPUNIT_MAJOR_MIN_PHP
        .iter()
        .any(|(major, min_php)| phpunit.contains(major) && *min_php <= floor);

    assert!(
        satisfiable,
        "phpunit constraint {phpunit:?} has no major installable on PHP {}.{} (the declared require.php floor)\n{content}",
        floor.0, floor.1
    );
}
