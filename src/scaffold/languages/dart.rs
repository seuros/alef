use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::naming::{dart_frb_version, dart_style};
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{DartStyle, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, PrimitiveType, TypeDef, TypeRef};
use crate::core::template_versions::{pub_dev, toolchain};
use crate::scaffold::{readme_language_configured, scaffold_meta};
use heck::ToLowerCamelCase;
use std::collections::HashSet;
use std::path::PathBuf;

pub(crate) fn scaffold_dart(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let pubspec_name = config.dart_pubspec_name();
    let module_name = api.crate_name.replace('-', "_");

    let flutter_rust_bridge = dart_frb_version(config);
    let dart_sdk = toolchain::DART_SDK_CONSTRAINT;
    let test_package = pub_dev::TEST_PACKAGE;
    let lints = pub_dev::LINTS;
    let ffi_package = pub_dev::FFI_PACKAGE;
    let freezed_annotation = pub_dev::FREEZED_ANNOTATION;
    let json_annotation = pub_dev::JSON_ANNOTATION;
    let freezed = pub_dev::FREEZED;
    let build_runner = pub_dev::BUILD_RUNNER;
    let json_serializable = pub_dev::JSON_SERIALIZABLE;
    let native_assets_cli = pub_dev::NATIVE_ASSETS_CLI;
    let http_package = pub_dev::HTTP_PACKAGE;
    let crypto_package = pub_dev::CRYPTO;
    let style = dart_style(config);

    let dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # FRB runtime is pure-Dart; works in both Flutter and server-Dart contexts.
  flutter_rust_bridge: ^{flutter_rust_bridge}
  # FRB codegen-2.x emits `@freezed` sealed classes annotated with these.
  freezed_annotation: '{freezed_annotation}'
  json_annotation: '{json_annotation}'
"#,
            flutter_rust_bridge = flutter_rust_bridge,
            freezed_annotation = freezed_annotation,
            json_annotation = json_annotation,
        ),
        DartStyle::Ffi => format!(
            r#"  # Raw dart:ffi bindings use package:ffi for native memory helpers.
  ffi: '{ffi_package}'
  # Native-assets build hook resolves the FFI shared library at consumer build time (Dart 3.0+).
  native_assets_cli: '{native_assets_cli}'
  # Product-type DTOs use @freezed annotation for code generation.
  freezed_annotation: '{freezed_annotation}'
  json_annotation: '{json_annotation}'
"#,
            ffi_package = ffi_package,
            native_assets_cli = native_assets_cli,
            freezed_annotation = freezed_annotation,
            json_annotation = json_annotation,
        ),
    };
    let dev_dependency_block = match style {
        DartStyle::Frb => format!(
            r#"  # Required by flutter_rust_bridge_codegen 2.x for sealed classes.
  freezed: '{freezed}'
  build_runner: '{build_runner}'
  json_serializable: '{json_serializable}'
"#,
            freezed = freezed,
            build_runner = build_runner,
            json_serializable = json_serializable,
        ),
        DartStyle::Ffi => format!(
            r#"  # Required for product-type DTO code generation (@freezed annotation).
  freezed: '{freezed}'
  build_runner: '{build_runner}'
  json_serializable: '{json_serializable}'
"#,
            freezed = freezed,
            build_runner = build_runner,
            json_serializable = json_serializable,
        ),
    };

    let repository_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("repository: {repository}\n"))
        .unwrap_or_default();
    let homepage_line = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage: {}\n", meta.homepage)
    };

    let capsule_dependency_lines: String = {
        let mut deps: Vec<(String, String)> = config
            .dart
            .as_ref()
            .map(|c| {
                c.capsule_types
                    .values()
                    .filter(|cap| !cap.package.is_empty())
                    .map(|cap| {
                        let ver = if cap.package_version.is_empty() {
                            "any".to_string()
                        } else {
                            cap.package_version.clone()
                        };
                        (cap.package.clone(), ver)
                    })
                    .collect()
            })
            .unwrap_or_default();
        deps.sort();
        deps.dedup();
        deps.iter().map(|(pkg, ver)| format!("  {pkg}: '{ver}'\n")).collect()
    };

    let pubspec_yaml = format!(
        r#"name: {name}
description: {description}
version: {version}
{repository_line}{homepage_line}environment:
  sdk: '{dart_sdk}'
executables:
  download_libs:
dependencies:
  http: '{http_package}'
  # SHA-256 verification of downloaded native-library release assets.
  crypto: '{crypto_package}'
{capsule_dependency_lines}{dependency_block}dev_dependencies:
  test: '{test_package}'
  lints: '{lints}'
{dev_dependency_block}"#,
        name = pubspec_name,
        description = meta.description,
        version = version,
        repository_line = repository_line,
        homepage_line = homepage_line,
        capsule_dependency_lines = capsule_dependency_lines,
        http_package = http_package,
        crypto_package = crypto_package,
    );

    let generated_dir = format!("lib/src/{module_name}_bridge_generated/**");

    let analysis_options_yaml = format!(
        r#"include: package:lints/recommended.yaml

analyzer:
  exclude:
    - lib/src/frb/**
    - {generated_dir}
    - example/**
    - lib/src/traits.dart

linter:
  rules:
    - avoid_empty_else
    - avoid_print
    - avoid_relative_lib_imports
    - avoid_returning_this
    - avoid_slow_async_io
    - cancel_subscriptions
    - close_sinks
    - comment_references
    - control_flow_in_finally
    - empty_statements
    - hash_and_equals
    - literal_only_boolean_expressions
    - no_adjacent_strings_in_list
    - no_duplicate_case_values
    - prefer_void_to_null
    - throw_in_finally
    - unnecessary_statements
    - unrelated_type_equality_checks
"#
    );

    let gitignore = ".dart_tool/\nbuild/\npubspec.lock\n";

    // NOTE: do NOT exclude lib/src/native/ or *.so/*.dylib/*.dll here. Native FFI
    // libraries are staged into lib/src/native/<rid>/ at publish time and MUST ship in
    // the pub.dev tarball; .pubignore fully replaces git-based file listing, so excluding
    // them silently strips the natives and consumers cannot load the FFI library.
    let pubignore = "android/\nios/\nblobs/\nrust/\nexample/\ntest/\n";

    // `package:` URIs require at least one path segment after the package name.
    // FRB style exports the public API through the default barrel file at
    // `lib/{module_name}.dart` (see `barrel_name` in gen_bindings::generate_bindings,
    // which defaults to the same crate-derived module name used here). FFI style has
    // no barrel — the re-export wrapper lives at `lib/src/{module_name}.dart`
    // (see gen_ffi::emit). ~keep
    let package_import_path = match style {
        DartStyle::Frb => format!("{pubspec_name}/{module_name}.dart"),
        DartStyle::Ffi => format!("{pubspec_name}/src/{module_name}.dart"),
    };

    let test_dart = scaffold_dart_test(api, config, &module_name, &package_import_path);

    let crate_name = &api.crate_name;
    let build_commands = match style {
        DartStyle::Frb => format!(
            r#"cargo build -p {crate_name}-dart
flutter_rust_bridge_codegen generate
dart pub get
dart analyze
dart test"#
        ),
        DartStyle::Ffi => r#"cargo build --release -p {crate_name}-ffi
dart pub get
dart analyze
dart test"#
            .replace("{crate_name}", crate_name),
    };
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();

    let readme = format!(
        r#"# {pubspec_name}

{description}

## Installation

Add to your `pubspec.yaml`:

```yaml
dependencies:
  {pubspec_name}: ^{version}
```

Then run:

```sh
dart pub get
```

## Building

From the repository root:

```sh
{build_commands}
```
"#,
        pubspec_name = pubspec_name,
        description = meta.description,
        version = version,
    ) + &license_section;

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\n\n[*.dart]\nindent_style = space\nindent_size = 2\n";

    let changelog = format!(
        "# Changelog\n\nAll notable changes to this package will be documented in this file.\n\n## {version}\n\n- Initial release.\n",
        version = version,
    );

    let example_dart = format!(
        r#"import 'package:{package_import_path}' as {module_name};

void main() {{
  print('Example: {pubspec_name} loaded successfully');
  // Add your API calls here after code generation
}}
"#,
        package_import_path = package_import_path,
        pubspec_name = pubspec_name,
        module_name = module_name,
    );

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from("packages/dart/pubspec.yaml"),
            content: pubspec_yaml,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/analysis_options.yaml"),
            content: analysis_options_yaml,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.pubignore"),
            content: pubignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/dart/test/{module_name}_test.dart")),
            content: test_dart,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/dart/example/{module_name}_example.dart")),
            content: example_dart,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/dart/CHANGELOG.md"),
            content: changelog,
            generated_header: false,
        },
    ];
    // See the matching comment in `scaffold_swift`: once `[crates.readme.languages.dart]`
    // is configured, the README module owns this path end-to-end, and scaffold must not
    // compete with it as a second writer (#555). Inserted at its original position
    // (after `.editorconfig`, before the example) rather than appended, so file order
    // is unchanged for languages that still rely on this placeholder. ~keep
    if !readme_language_configured(config, "dart") {
        files.insert(
            6,
            GeneratedFile {
                path: PathBuf::from("packages/dart/README.md"),
                content: readme,
                generated_header: false,
            },
        );
    }

    if matches!(style, DartStyle::Ffi) {
        let build_dart = format!(
            r#"// Dart 3.0+ native-assets build hook.
// Resolves the FFI shared library produced by `cargo build --release -p {crate_name}-ffi`
// and bundles it into the consumer's Dart application at build time.
// See: https://dart.dev/interop/c-interop#native-assets

import 'dart:io' as io;
import 'package:native_assets_cli/native_assets_cli.dart';

const _crateName = '{crate_name}';
const _packageName = '{pubspec_name}';

Future<void> main(List<String> args) async {{
  await build(args, (input, output) async {{
    final libFile = input.config.targetOS.dylibFileName(_crateName);
    final repoRoot = _findRepoRoot(io.Directory.current);
    final candidates = <io.File>[
      io.File('${{repoRoot.path}}/target/release/$libFile'),
      io.File('${{repoRoot.path}}/crates/${{_crateName}}-ffi/target/release/$libFile'),
      io.File('${{repoRoot.path}}/packages/dart/rust/target/release/$libFile'),
    ];
    for (final candidate in candidates) {{
      if (candidate.existsSync()) {{
        output.addAsset(NativeCodeAsset(
          package: _packageName,
          name: '${{_packageName}}.dart',
          file: candidate.uri,
          linkMode: DynamicLoadingBundled(),
          os: input.config.targetOS,
          architecture: input.config.targetArchitecture,
        ));
        return;
      }}
    }}
    throw StateError(
      'Native library $libFile not found. '
      'Build it with: cargo build --release -p ${{_crateName}}-ffi',
    );
  }});
}}

io.Directory _findRepoRoot(io.Directory start) {{
  io.Directory current = start;
  while (current.path != current.parent.path) {{
    if (io.File('${{current.path}}/Cargo.toml').existsSync() &&
        io.Directory('${{current.path}}/.git').existsSync()) {{
      return current;
    }}
    current = current.parent;
  }}
  return start;
}}
"#,
            crate_name = crate_name,
            pubspec_name = pubspec_name,
        );
        files.push(GeneratedFile {
            path: PathBuf::from("packages/dart/hook/build.dart"),
            content: build_dart,
            generated_header: false,
        });
    }

    Ok(files)
}

/// Build the seed content for `test/{module_name}_test.dart`.
///
/// `write_scaffold_files_report` treats `generated_header: false` as create-only, so once
/// a real suite exists at this path alef never overwrites it; this only ever seeds a fresh
/// project. The seed must not be vacuous — `expect(1 + 1, equals(2))` compiles and passes
/// no matter what the generated API looks like, exactly the "0 assertions, silently green"
/// defect the zig test-module fix (`scaffold_zig_test`) and the Swift seed
/// (`scaffold_swift_test`) already closed one layer down. So, mirroring `scaffold_swift_test`'s
/// approach, this asserts against the *real*, currently-generated API surface (`api`), in
/// order of how strong a check is safely synthesizable without duplicating the Dart binding
/// emitter's full type-mapping surface:
///
/// 1. A visible, non-opaque DTO (`has_serde`, struct, all fields plain primitives/`String`,
///    no optional/cfg-gated fields) is literal-constructed twice with identical field values
///    and compared for equality — it fails to compile if the generated constructor drops or
///    renames a field, and fails at runtime if the generated (freezed) value-equality stops
///    being field-based, not just on a missing symbol.
/// 2. Otherwise, any other visible type or enum is referenced bare (`{module}.Name`), forcing
///    the analyzer to resolve it — weaker than the round trip (construction/field shape isn't
///    checked) but still a real, falsifiable fact about the generated output.
/// 3. Only when the API surface is genuinely empty (e.g. scaffolding before any Rust code
///    exists) does this fall back to the harmless `1 + 1 == 2` placeholder that merely forces
///    the package import to resolve — there is nothing else to assert against yet, and once
///    real items exist this file is never regenerated over. ~keep
fn scaffold_dart_test(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    module_name: &str,
    package_import_path: &str,
) -> String {
    let exclude_types = dart_binding_exclusions(config);

    let round_trip_candidate = api
        .types
        .iter()
        .filter(|t| dart_type_is_visible(t, &exclude_types))
        .filter(|t| !t.is_opaque && t.has_serde && !t.has_stripped_cfg_fields)
        .find_map(|t| simple_dart_fields(t).map(|fields| (t, fields)));
    if let Some((ty, fields)) = round_trip_candidate {
        return dart_equality_round_trip_test(module_name, package_import_path, &ty.name, &fields);
    }

    let visible_type_name = api
        .types
        .iter()
        .filter(|t| dart_type_is_visible(t, &exclude_types))
        .map(|t| t.name.clone())
        .next();
    let visible_enum_name = || {
        api.enums
            .iter()
            .filter(|e| !e.binding_excluded && !exclude_types.contains(&e.name))
            .map(|e| e.name.clone())
            .next()
    };
    if let Some(name) = visible_type_name.or_else(visible_enum_name) {
        return dart_type_reference_test(module_name, package_import_path, &name);
    }

    dart_placeholder_test(module_name, package_import_path)
}

/// Names excluded from Dart binding generation, mirroring `[crates.dart] exclude_types` plus
/// `[crates.ffi] exclude_types` that the real Dart binding emitter honors. Kept in sync
/// deliberately rather than shared, since this seed-picker only needs *a* safe, visible name,
/// not the emitter's exhaustive filtered set. Unlike the Swift seed's equivalent, there is no
/// `[crates.dart] exclude_fields` knob to fold in — Dart has no per-field exclusion config.
fn dart_binding_exclusions(config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .dart
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        exclude_types.extend(ffi.exclude_types.iter().cloned());
    }
    exclude_types
}

/// A visible (non-trait, non-`binding_excluded`, not config-excluded) candidate type for the
/// scaffold seed to reference.
fn dart_type_is_visible(ty: &TypeDef, exclude_types: &HashSet<String>) -> bool {
    !ty.is_trait && !ty.binding_excluded && !exclude_types.contains(&ty.name)
}

/// A field simple enough to synthesize a literal Dart value for: a primitive or `String`,
/// never optional, never `#[cfg(...)]`-gated (whether it exists depends on active features,
/// which this scaffold-time seed cannot know).
struct SimpleDartField {
    label: String,
    literal: String,
}

/// Compute a literal-constructible field list for `ty`, or `None` when any visible field
/// falls outside the safely synthesizable subset (optional, cfg-gated, or a type other than
/// a primitive/`String` — `Named`/`Vec`/`Map`/etc. would need recursive construction this
/// seed does not attempt). Bails on the *whole type* rather than partially constructing it,
/// since the real generated constructor requires every non-optional named parameter.
fn simple_dart_fields(ty: &TypeDef) -> Option<Vec<SimpleDartField>> {
    let mut fields = Vec::new();
    for field in binding_fields(&ty.fields) {
        if field.optional || field.cfg.is_some() {
            return None;
        }
        let literal = match &field.ty {
            TypeRef::Primitive(primitive) => dart_primitive_literal(primitive),
            TypeRef::String => "'alef-scaffold'".to_string(),
            _ => return None,
        };
        fields.push(SimpleDartField {
            label: dart_safe_ident(&field.name.to_lower_camel_case()),
            literal,
        });
    }
    if fields.is_empty() { None } else { Some(fields) }
}

/// A literal Dart value for a primitive type. `bool` gets a non-default `true` and floats a
/// non-integral `1.5` so a constructor that silently drops or zeroes a field is still caught
/// by the equality check.
fn dart_primitive_literal(primitive: &PrimitiveType) -> String {
    match primitive {
        PrimitiveType::Bool => "true".to_string(),
        PrimitiveType::F32 | PrimitiveType::F64 => "1.5".to_string(),
        _ => "1".to_string(),
    }
}

/// The strongest safe check: literal-construct a visible DTO twice with identical field
/// values and assert the two instances are equal, so a generated constructor that drops or
/// renames a field, or a value-equality (`==`) implementation that stops being field-based,
/// fails `dart test` immediately instead of shipping green with a suite that asserts nothing.
fn dart_equality_round_trip_test(
    module_name: &str,
    package_import_path: &str,
    type_name: &str,
    fields: &[SimpleDartField],
) -> String {
    let init_args = fields
        .iter()
        .map(|f| format!("{}: {}", f.label, f.literal))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"import 'package:test/test.dart';
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('{type_name} equality holds for identical field values', () {{
    // Literal-constructs the generated `{type_name}` DTO twice with identical field
    // values and compares them for equality, so a constructor that drops/renames a
    // field, or generated equality that stops being field-based, fails `dart test`
    // immediately instead of shipping green with a suite that asserts nothing about
    // the generated API. Create-only scaffold seed. ~keep
    final a = {module_name}.{type_name}({init_args});
    final b = {module_name}.{type_name}({init_args});
    expect(a, equals(b));
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
        type_name = type_name,
        init_args = init_args,
    )
}

/// `name` isn't a literal-constructible DTO this seed can safely construct generically, so
/// this checks the generated type or enum exists and is referenceable at compile time
/// instead.
fn dart_type_reference_test(module_name: &str, package_import_path: &str, name: &str) -> String {
    format!(
        r#"import 'package:test/test.dart';
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('{module_name} exposes `{name}`', () {{
    // `{name}` isn't a literal-constructible DTO this seed can safely construct
    // generically, so this checks the generated type exists and is referenceable at
    // compile time instead. Create-only scaffold seed. ~keep
    expect({module_name}.{name}, isNotNull);
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
        name = name,
    )
}

/// No generated API surface exists yet for this crate, so there is nothing to assert against
/// beyond the package import resolving. Once real types exist, alef never regenerates over
/// this file — it is a create-only scaffold seed.
fn dart_placeholder_test(module_name: &str, package_import_path: &str) -> String {
    format!(
        r#"import 'package:test/test.dart';
// ignore: unused_import
import 'package:{package_import_path}' as {module_name};

void main() {{
  test('placeholder', () {{
    // No generated API surface exists yet for this crate, so there is nothing to assert
    // against beyond the module resolving. Once real types exist, alef never regenerates
    // over this file -- it is a create-only scaffold seed. ~keep
    expect(1 + 1, equals(2));
  }});
}}
"#,
        package_import_path = package_import_path,
        module_name = module_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{EnumDef, FieldDef};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn minimal_config() -> ResolvedCrateConfig {
        resolve_config(
            r#"
[workspace]
languages = ["dart"]
[[crates]]
name = "my-lib"
sources = []
"#,
        )
    }

    fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a GeneratedFile {
        files
            .iter()
            .find(|f| f.path == std::path::Path::new(path))
            .unwrap_or_else(|| panic!("missing scaffolded file: {path}"))
    }

    fn simple_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            has_serde: true,
            fields: vec![
                FieldDef {
                    name: "count".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
                FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// A visible DTO whose fields are all plain primitives/`String` is literal-constructed
    /// twice and compared for equality — the strongest safe check, since it fails to
    /// compile on a dropped/renamed field and fails at runtime on broken equality, not
    /// just on a missing symbol.
    #[test]
    fn scaffold_test_round_trips_a_simple_dto() {
        let api = ApiSurface {
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("import 'package:my_lib/my_lib.dart' as my_lib;"), "got:\n{out}");
        assert!(
            out.contains("test('Widget equality holds for identical field values'"),
            "got:\n{out}"
        );
        assert!(
            out.contains("final a = my_lib.Widget(count: 1, label: 'alef-scaffold');"),
            "got:\n{out}"
        );
        assert!(
            out.contains("final b = my_lib.Widget(count: 1, label: 'alef-scaffold');"),
            "got:\n{out}"
        );
        assert!(out.contains("expect(a, equals(b));"), "got:\n{out}");
        assert!(
            !out.contains("expect(1 + 1, equals(2));"),
            "must not be the vacuous placeholder, got:\n{out}"
        );
        assert!(
            !out.contains("ignore: unused_import"),
            "the import is actually used here, got:\n{out}"
        );
    }

    /// An opaque type has no client-constructible representation, so it can't be literal
    /// -constructed; the seed falls back to a compile-time existence check on the type
    /// name instead of skipping straight to the vacuous placeholder.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_opaque_type() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Client".to_string(),
                is_opaque: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Client`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Client, isNotNull);"), "got:\n{out}");
        assert!(!out.contains("equals(b)"), "got:\n{out}");
    }

    /// A DTO with an unsupported field shape (e.g. `Optional<T>`) can't be literal
    /// -constructed safely by this seed either — it also falls back to the existence
    /// check rather than emitting a construction call with a guessed value.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_unsupported_field_shape() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "Config".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "nickname".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Config`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Config, isNotNull);"), "got:\n{out}");
    }

    /// With no visible struct at all, a visible enum is checked for existence instead.
    #[test]
    fn scaffold_test_falls_back_to_existence_check_for_an_enum_when_no_types_exist() {
        let api = ApiSurface {
            enums: vec![EnumDef {
                name: "Color".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("test('my_lib exposes `Color`'"), "got:\n{out}");
        assert!(out.contains("expect(my_lib.Color, isNotNull);"), "got:\n{out}");
    }

    /// `binding_excluded` types were never emitted into the generated Dart module, so the
    /// seed must skip them rather than asserting against a type that doesn't exist.
    #[test]
    fn scaffold_test_skips_binding_excluded_types() {
        let api = ApiSurface {
            types: vec![
                TypeDef {
                    name: "Hidden".to_string(),
                    is_opaque: true,
                    binding_excluded: true,
                    ..Default::default()
                },
                TypeDef {
                    name: "Visible".to_string(),
                    is_opaque: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let out = scaffold_dart_test(&api, &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("Visible"), "got:\n{out}");
        assert!(!out.contains("Hidden"), "got:\n{out}");
    }

    /// A genuinely empty API surface (no Rust code written yet) has nothing to assert
    /// against beyond the import resolving — the only honest seed content, and the only
    /// case where the placeholder assertion is legitimate.
    #[test]
    fn scaffold_test_falls_back_to_placeholder_when_api_surface_is_empty() {
        let out = scaffold_dart_test(&ApiSurface::default(), &minimal_config(), "my_lib", "my_lib/my_lib.dart");

        assert!(out.contains("import 'package:my_lib/my_lib.dart' as my_lib;"), "got:\n{out}");
        assert!(out.contains("test('placeholder', () {"), "got:\n{out}");
        assert!(out.contains("expect(1 + 1, equals(2));"), "got:\n{out}");
        assert!(
            out.contains("ignore: unused_import"),
            "placeholder never references the import, got:\n{out}"
        );
    }

    /// End-to-end through `scaffold_dart`: the emitted test file at
    /// `test/{module}_test.dart` must carry a real assertion against the generated API
    /// (not the vacuous `expect(1 + 1, equals(2))` placeholder) whenever the API surface
    /// has something to assert against, and must be `generated_header: false` so the
    /// create-only write-path guard (`write_scaffold_files_report`'s `can_skip`) never
    /// overwrites a real hand-written suite once one exists at that path.
    #[test]
    fn scaffold_dart_emits_real_test_assertions_and_is_create_only() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            types: vec![simple_type("Widget")],
            ..Default::default()
        };
        let files = scaffold_dart(&api, &minimal_config()).expect("scaffold");
        let test_file = find_file(&files, "packages/dart/test/my_lib_test.dart");

        assert!(
            !test_file.generated_header,
            "test seed must be generated_header: false (create-only)"
        );
        assert!(
            test_file.content.contains("equals(b)"),
            "got:\n{}",
            test_file.content
        );
        assert!(
            !test_file.content.contains("expect(1 + 1, equals(2));"),
            "must not emit the old vacuous placeholder test, got:\n{}",
            test_file.content
        );
    }
}
