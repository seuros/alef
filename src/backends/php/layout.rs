//! Where alef writes the PHP userland classes, and how every manifest names that directory.
//!
//! Three separate stages have to agree on one directory: the backend writes the classes there,
//! the scaffolded root `composer.json` autoloads it, and the generated `e2e/php/composer.json`
//! autoloads it again through a relative prefix. They used to derive it independently and the
//! e2e stage appended a fixed `/src/` to the resolved package root, so every layout whose output
//! path did not already end in `src` sent Composer to a directory no stage ever writes — which
//! only resolves while an unmanaged duplicate of the class tree is kept beside the managed one.
//! Read the directory from here instead of re-deriving it. ~keep

use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};

/// Class directory for the historical split layout, used when no `[crates.output] php` entry
/// and no `[crates.php.stubs] output` name one. The sibling `packages/php/composer.json` the
/// scaffolder emits for that layout autoloads `src/` relative to itself, which is this path.
const DEFAULT_SPLIT_LAYOUT_CLASS_DIR: &str = "packages/php/src/";

/// The directory `alef generate` writes the PHP userland classes into, relative to the
/// repository root.
///
/// `[crates.php.stubs] output` wins, then the resolved `[crates.output] php` path, then the
/// split-layout default — the same precedence `generate_public_api` and `generate_bindings`
/// apply when they place the emitted files.
pub fn php_class_output_dir(config: &ResolvedCrateConfig) -> String {
    config
        .php
        .as_ref()
        .and_then(|php| php.stubs.as_ref())
        .map(|stubs| stubs.output.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            resolve_output_dir(
                config.output_paths.get("php"),
                &config.name,
                DEFAULT_SPLIT_LAYOUT_CLASS_DIR,
            )
        })
}

/// [`php_class_output_dir`] spelled as a Composer PSR-4 target: exactly one trailing slash,
/// because Composer resolves a PSR-4 value as a directory prefix and concatenates the relative
/// class path onto it verbatim.
pub fn php_psr4_target(config: &ResolvedCrateConfig) -> String {
    format!("{}/", php_class_output_dir(config).trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn resolve(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    #[test]
    fn unconfigured_php_output_resolves_to_the_co_located_binding_crate() {
        let config = resolve(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
"#,
        );
        assert_eq!(php_class_output_dir(&config), "crates/my-lib-php/src");
        assert_eq!(php_psr4_target(&config), "crates/my-lib-php/src/");
    }

    #[test]
    fn php_disabled_falls_back_to_the_split_layout_class_directory() {
        let config = resolve(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "my-lib"
sources = []
"#,
        );
        assert_eq!(php_psr4_target(&config), "packages/php/src/");
    }

    #[test]
    fn configured_output_without_a_src_segment_is_used_verbatim() {
        let config = resolve(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "packages/php"
"#,
        );
        assert_eq!(php_psr4_target(&config), "packages/php/");
    }

    #[test]
    fn stubs_output_wins_over_the_resolved_output_path() {
        let config = resolve(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "packages/php"
[crates.php.stubs]
output = "src/php-classes"
"#,
        );
        assert_eq!(php_psr4_target(&config), "src/php-classes/");
    }

    #[test]
    fn a_trailing_slash_is_never_doubled() {
        let config = resolve(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "crates/my-lib-php/src/"
"#,
        );
        assert_eq!(php_psr4_target(&config), "crates/my-lib-php/src/");
    }
}
