use super::*;
use crate::core::config::NewAlefConfig;

fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
    cfg.resolve().expect("resolve").remove(0)
}

fn minimal_config() -> ResolvedCrateConfig {
    resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []
"#,
    )
}

fn zero_arg_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn dto(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..Default::default()
    }
}

/// The strongest tier: a visible zero-arg, primitive-returning function is actually
/// invoked across the Magnus boundary, not merely named.
#[test]
fn calls_a_visible_zero_argument_function() {
    let api = ApiSurface {
        functions: vec![zero_arg_function("ping", TypeRef::Primitive(PrimitiveType::Bool))],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(out.starts_with("# frozen_string_literal: true\n"), "got:\n{out}");
    assert!(out.contains("require_relative \"../lib/my_lib\"\n"), "got:\n{out}");
    assert!(out.contains("RSpec.describe MyLib do\n"), "got:\n{out}");
    assert!(
        out.contains("    expect(described_class.ping).to(be(true).or(be(false)))\n"),
        "got:\n{out}"
    );
}

/// The matcher must follow the Magnus type map, not a generic truthiness check.
#[test]
fn matches_the_returned_ruby_type_for_each_return_kind() {
    let cases = [
        (TypeRef::String, "expect(described_class.probe).to(be_a(String))"),
        (
            TypeRef::Primitive(PrimitiveType::U64),
            "expect(described_class.probe).to(be_a(Integer))",
        ),
        (
            TypeRef::Primitive(PrimitiveType::F64),
            "expect(described_class.probe).to(be_a(Float))",
        ),
    ];
    for (return_type, expected) in cases {
        let api = ApiSurface {
            functions: vec![zero_arg_function("probe", return_type)],
            ..Default::default()
        };
        let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");
        assert!(out.contains(expected), "expected `{expected}`, got:\n{out}");
    }
}

/// A function taking parameters cannot be called generically (unknown ownership and
/// conversion needs per parameter), so the ladder must degrade instead of guessing.
#[test]
fn skips_functions_that_take_parameters() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            params: vec![crate::core::ir::ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..zero_arg_function("greet", TypeRef::String)
        }],
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("greet"), "got:\n{out}");
    assert!(
        out.contains("described_class::Widget.new(label: \"alef-scaffold\")"),
        "got:\n{out}"
    );
}

/// Async functions run through a Tokio runtime under a differently-named Rust body; the
/// seed must not be the first thing to exercise that path.
#[test]
fn skips_async_functions() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            is_async: true,
            ..zero_arg_function("fetch", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("fetch"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// A `cfg`-gated function is registered only when the extension was compiled with that
/// feature, which a scaffold-time seed cannot know.
#[test]
fn skips_cfg_gated_functions() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            cfg: Some("feature = \"extra\"".to_string()),
            ..zero_arg_function("extra_ping", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("extra_ping"), "got:\n{out}");
}

/// A function the Magnus wrapper generator cannot delegate gets an `unimplemented` body
/// that raises `RuntimeError` when called. It is still registered and callable, so only
/// this predicate keeps the seed off it — otherwise the example would be permanently red
/// on a healthy build.
#[test]
fn skips_functions_whose_generated_body_only_raises() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            sanitized: true,
            error_type: Some("Error".to_string()),
            ..zero_arg_function("not_delegatable", TypeRef::String)
        }],
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("not_delegatable"), "got:\n{out}");
    assert!(
        out.contains("described_class::Widget.new(label: \"alef-scaffold\")"),
        "got:\n{out}"
    );
}

/// A fallible function can raise for reasons that have nothing to do with the binding, so
/// an infallible candidate wins even when it appears later in the surface.
#[test]
fn prefers_an_infallible_function_over_a_fallible_one() {
    let api = ApiSurface {
        functions: vec![
            FunctionDef {
                error_type: Some("Error".to_string()),
                ..zero_arg_function("might_fail", TypeRef::String)
            },
            zero_arg_function("always_works", TypeRef::String),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class.always_works).to(be_a(String))\n"),
        "got:\n{out}"
    );
    assert!(!out.contains("might_fail"), "got:\n{out}");
}

/// When every candidate is fallible the strongest tier still fires: an example that can
/// fail for a real reason is worth more than degrading to a weaker one.
#[test]
fn still_calls_a_fallible_function_when_it_is_the_only_candidate() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            error_type: Some("Error".to_string()),
            ..zero_arg_function("might_fail", TypeRef::String)
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class.might_fail).to(be_a(String))\n"),
        "got:\n{out}"
    );
}

/// `binding_excluded` functions never reach the generated extension, so the seed must not
/// call one.
#[test]
fn skips_binding_excluded_functions() {
    let api = ApiSurface {
        functions: vec![
            FunctionDef {
                binding_excluded: true,
                ..zero_arg_function("hidden", TypeRef::String)
            },
            zero_arg_function("visible", TypeRef::String),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(out.contains("described_class.visible"), "got:\n{out}");
    assert!(!out.contains("hidden"), "got:\n{out}");
}

/// `[crates.ruby] exclude_functions` mirrors `MagnusBackend`'s own filter, so a function
/// excluded there must be skipped here too.
#[test]
fn skips_functions_excluded_via_ruby_config() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []

[crates.ruby]
exclude_functions = ["ping"]
"#,
    );
    let api = ApiSurface {
        functions: vec![zero_arg_function("ping", TypeRef::String)],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &config, "my_lib");

    assert!(
        !out.contains("ping"),
        "excluded function must not be referenced, got:\n{out}"
    );
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// With no callable function, a literal-constructible DTO is built through the generated
/// keyword constructor and every field read back through its accessor.
#[test]
fn constructs_a_simple_dto_and_asserts_every_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("count", TypeRef::Primitive(PrimitiveType::U32)),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    instance = described_class::Widget.new(label: \"alef-scaffold\", count: 1)\n"),
        "got:\n{out}"
    );
    assert!(
        out.contains("    expect([instance.label, instance.count]).to(eq([\"alef-scaffold\", 1]))\n"),
        "got:\n{out}"
    );
}

/// An all-String multi-field DTO must not emit a bracketed `["alef-scaffold",
/// "alef-scaffold"]` array literal: `RUBY_SEED_STRING_LITERAL` is a hyphenated word, which
/// the `.rubocop.yml` scaffolded alongside this file still matches via `Style/WordArray`'s
/// `WordRegex` (it explicitly allows one hyphen) at its default `MinSize` of 2 -- flagging
/// every new Ruby consumer's freshly-scaffolded spec red before a single line is hand-edited.
#[test]
fn constructs_an_all_string_dto_without_a_bracketed_word_array() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("note", TypeRef::String),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect([instance.label, instance.note]).to(eq(%w[alef-scaffold alef-scaffold]))\n"),
        "got:\n{out}"
    );
    assert!(
        !out.contains("[\"alef-scaffold\", \"alef-scaffold\"]"),
        "must not emit a bracketed all-String literal array: got:\n{out}"
    );
}

/// A single-field DTO reads better as a scalar comparison than a one-element array.
#[test]
fn asserts_a_single_field_dto_without_an_array() {
    let api = ApiSurface {
        types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(instance.label).to(eq(\"alef-scaffold\"))\n"),
        "got:\n{out}"
    );
}

/// A `Named` field has no default in the generated constructor, so a partial construction
/// would raise `ArgumentError`. The whole type is rejected rather than partly built.
#[test]
fn falls_back_to_a_constant_reference_for_a_dto_with_a_named_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![
                simple_field("label", TypeRef::String),
                simple_field("nested", TypeRef::Named("Other".to_string())),
            ],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains(".new("), "got:\n{out}");
    assert!(
        out.contains("    expect(described_class.const_get(:Widget)).to(be_a(Module))\n"),
        "got:\n{out}"
    );
}

/// Optional fields carry `nil` semantics this seed does not model, so they disqualify the
/// construction tier rather than being guessed at.
#[test]
fn falls_back_to_a_constant_reference_for_a_dto_with_an_optional_field() {
    let api = ApiSurface {
        types: vec![dto(
            "Widget",
            vec![FieldDef {
                optional: true,
                ..simple_field("label", TypeRef::String)
            }],
        )],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains(".new("), "got:\n{out}");
    assert!(out.contains("const_get(:Widget)"), "got:\n{out}");
}

/// `[crates.ruby] exclude_types` and `binding_excluded` both remove a class from the
/// generated extension, so neither may be named by the seed.
#[test]
fn skips_types_excluded_by_config_or_binding_exclusion() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "my-lib"
sources = []

[crates.ruby]
exclude_types = ["Excluded"]
"#,
    );
    let api = ApiSurface {
        types: vec![
            dto("Excluded", vec![simple_field("label", TypeRef::String)]),
            TypeDef {
                binding_excluded: true,
                ..dto("Hidden", vec![simple_field("label", TypeRef::String)])
            },
            dto("Visible", vec![simple_field("label", TypeRef::String)]),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &config, "my_lib");

    assert!(!out.contains("Excluded"), "got:\n{out}");
    assert!(!out.contains("Hidden"), "got:\n{out}");
    assert!(out.contains("described_class::Visible.new("), "got:\n{out}");
}

/// `MagnusBackend::generate_public_api` drops `*Update` and `*Builder` types from the
/// module's curated re-export list, so a seed naming one may reference nothing.
#[test]
fn skips_update_and_builder_types() {
    let api = ApiSurface {
        types: vec![
            dto("WidgetUpdate", vec![simple_field("label", TypeRef::String)]),
            dto("WidgetBuilder", vec![simple_field("label", TypeRef::String)]),
        ],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("WidgetUpdate"), "got:\n{out}");
    assert!(!out.contains("WidgetBuilder"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// Enums are not registered as Ruby constants by the Magnus backend, so an enum-only
/// surface must degrade to the version tier rather than name a constant that is absent.
#[test]
fn never_names_an_enum_because_magnus_registers_none_as_constants() {
    let api = ApiSurface {
        enums: vec![crate::core::ir::EnumDef {
            name: "Colour".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");

    assert!(!out.contains("Colour"), "got:\n{out}");
    assert!(out.contains("described_class::VERSION"), "got:\n{out}");
}

/// An empty API surface still gets a falsifiable example: `VERSION` only resolves once the
/// gem — and therefore the native extension `native.rb` dlopens — has loaded.
#[test]
fn falls_back_to_the_version_assertion_when_the_api_surface_is_empty() {
    let out = scaffold_ruby_spec(&ApiSurface::default(), &minimal_config(), "my_lib");

    assert!(
        out.contains("    expect(described_class::VERSION).to match(/\\A\\d+\\.\\d+\\.\\d+/)\n"),
        "got:\n{out}"
    );
}

/// No tier may emit a tautology, and every tier must go through the `require_relative`
/// that dlopens the native extension — that is the property making even the weakest tier
/// falsifiable rather than decorative.
#[test]
fn no_tier_emits_a_vacuous_or_unlinked_example() {
    let surfaces = [
        ApiSurface {
            functions: vec![zero_arg_function("ping", TypeRef::String)],
            ..Default::default()
        },
        ApiSurface {
            types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
            ..Default::default()
        },
        ApiSurface {
            types: vec![dto(
                "Widget",
                vec![simple_field("nested", TypeRef::Named("Other".to_string()))],
            )],
            ..Default::default()
        },
        ApiSurface::default(),
    ];
    for api in surfaces {
        let out = scaffold_ruby_spec(&api, &minimal_config(), "my_lib");
        assert!(
            out.contains("require_relative \"../lib/my_lib\""),
            "every tier must load the gem, got:\n{out}"
        );
        assert_eq!(out.matches("  it \"").count(), 1, "exactly one example, got:\n{out}");
        for tautology in ["expect(1)", "eq(1 + 1)", "to be_truthy", "to be_falsey"] {
            assert!(!out.contains(tautology), "vacuous assertion `{tautology}` in:\n{out}");
        }
        assert!(
            out.contains("described_class"),
            "the example must assert against the generated module, got:\n{out}"
        );
    }
}

/// The seed carries no alef header marker, and must not: the marker is what
/// `write_scaffold_files_report`'s ownership guard reads as "alef owns this file", which
/// would let an `overwrite: true` run (e.g. `alef version`) replace a hand-written suite.
#[test]
fn seed_content_carries_no_alef_marker() {
    let out = scaffold_ruby_spec(&ApiSurface::default(), &minimal_config(), "my_lib");

    assert!(
        !crate::core::hash::content_has_alef_marker(&out),
        "seed must stay unmarked so it is never reclaimed by an overwrite run, got:\n{out}"
    );
}

/// The seed lands at the path the generated `Rakefile`'s `RSpec::Core::RakeTask` already
/// scans, and is emitted create-only so a real suite is never overwritten.
#[test]
fn seed_is_emitted_create_only_at_the_rspec_default_path() {
    let config = minimal_config();
    let api = ApiSurface {
        version: "1.2.3".to_string(),
        ..Default::default()
    };
    let files = scaffold_ruby(&api, &config).expect("scaffold");
    let spec = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("/spec/"))
        .expect("a spec seed must be emitted");

    assert_eq!(spec.path.to_string_lossy(), "packages/ruby/spec/my_lib_spec.rb");
    assert!(!spec.generated_header, "the seed must stay create-only");
}

/// The `~keep` in this seed's rationale is load-bearing, unlike the markers the
/// render-time strip (`core::keep_marker`) removes from `.jinja` output. The seed is
/// create-only, so alef never rewrites it and the consumer's own `poly` uncomment pass is
/// what reads it — without the marker the rationale is deleted by the next `poly fmt`.
/// Pinned across every tier so a broadening of the strip cannot silently take it. ~keep
#[test]
fn every_seed_tier_keeps_its_uncomment_pass_marker() {
    let config = minimal_config();
    let tiers = [
        (
            "call",
            ApiSurface {
                functions: vec![zero_arg_function("ping", TypeRef::Primitive(PrimitiveType::Bool))],
                ..Default::default()
            },
        ),
        (
            "construct",
            ApiSurface {
                types: vec![dto("Widget", vec![simple_field("label", TypeRef::String)])],
                ..Default::default()
            },
        ),
        (
            "constant",
            ApiSurface {
                types: vec![dto(
                    "Widget",
                    vec![
                        simple_field("label", TypeRef::String),
                        simple_field("nested", TypeRef::Named("Other".to_string())),
                    ],
                )],
                ..Default::default()
            },
        ),
        ("version", ApiSurface::default()),
    ];

    for (tier, api) in tiers {
        let out = scaffold_ruby_spec(&api, &config, "my_lib");
        assert!(
            out.contains("replace it with a real suite. ~keep") || out.contains("break on the next release. ~keep"),
            "the {tier} tier lost its uncomment-pass marker, got:\n{out}"
        );
    }
}
