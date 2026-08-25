use super::*;
use crate::core::ir::DefaultValue;

/// Table-driven coverage for `postprocess::resolve_enum_field_defaults`: a field whose default
/// comes from `<EnumType>::default()` (or `Default::default()`, which folds identically — see
/// `defaults::expr_to_default_value`) starts out `DefaultValue::Empty` and this pass narrows it
/// to a concrete `EnumVariant` whenever the field's own enum type has a knowable default. ~keep
///
/// (a) The common case: the enum derives `Default` and one variant carries `#[default]`. The
/// field must resolve to exactly that variant.
#[test]
fn enum_field_default_resolves_to_the_marked_default_variant() {
    let source = r#"
        #[derive(Default, Clone, Copy)]
        pub enum Mode {
            Fast,
            #[default]
            Balanced,
            Slow,
        }

        pub struct Config {
            pub mode: Mode,
        }

        impl Default for Config {
            fn default() -> Self {
                Self { mode: Mode::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "Config").unwrap();
    let mode_field = config.fields.iter().find(|field| field.name == "mode").unwrap();

    assert_eq!(
        mode_field.typed_default,
        Some(DefaultValue::EnumVariant("Balanced".to_string())),
        "a field defaulting to an enum's own `default()` must resolve to the `#[default]`-marked variant"
    );
}

/// (b) The enum has a hand-written `impl Default` rather than `#[derive(Default)]`, so no variant
/// carries the `#[default]` attribute — that attribute only exists for the derive macro. The
/// variant is read out of the impl body instead (see
/// `extract::extractor::functions::impl_blocks::manual_default_unit_variant`), so the resolved
/// value is the one the Rust core actually returns.
///
/// The fixture is deliberately built so a positional guess gives the wrong answer: `Priority`
/// declares `Low` first but its `default()` returns `Medium`. Asserting `Medium` here is what
/// distinguishes reading the impl from falling back to the first declared variant.
#[test]
fn enum_field_default_reads_the_variant_a_manual_default_impl_returns() {
    let source = r#"
        #[derive(Clone, Copy)]
        pub enum Priority {
            Low,
            Medium,
            High,
        }

        impl Default for Priority {
            fn default() -> Self {
                Priority::Medium
            }
        }

        pub struct Task {
            pub priority: Priority,
        }

        impl Default for Task {
            fn default() -> Self {
                Self { priority: Priority::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let priority_enum = surface.enums.iter().find(|e| e.name == "Priority").unwrap();
    assert!(
        priority_enum.has_default,
        "a manual `impl Default for Priority` must still set `has_default=true`"
    );
    assert_eq!(
        priority_enum
            .variants
            .iter()
            .filter(|v| v.is_default)
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Medium"],
        "the manual impl's returned variant must be marked `is_default`, and only that variant"
    );

    let task = surface.types.iter().find(|typ| typ.name == "Task").unwrap();
    let priority_field = task.fields.iter().find(|field| field.name == "priority").unwrap();

    assert_eq!(
        priority_field.typed_default,
        Some(DefaultValue::EnumVariant("Medium".to_string())),
        "the field must resolve to the variant the manual impl returns, not the first declared one"
    );
}

/// (b2) A manual `impl Default` returning a *non-unit* variant must not be narrowed.
/// `DefaultValue::EnumVariant` names a bare unit-variant path with no arguments of its own, so
/// emitting one for a struct variant would name a value that cannot be constructed without its
/// fields — a fabrication that fails to compile in the generated binding. The field stays `Empty`
/// and each backend keeps its own honest guard.
#[test]
fn enum_field_default_stays_empty_when_the_default_variant_carries_data() {
    let source = r#"
        pub enum NodeContent {
            Heading { level: u8, text: String },
            Paragraph { text: String },
        }

        impl Default for NodeContent {
            fn default() -> Self {
                Self::Heading { level: 1, text: String::new() }
            }
        }

        pub struct Node {
            pub content: NodeContent,
        }

        impl Default for Node {
            fn default() -> Self {
                Self { content: NodeContent::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let node_content = surface.enums.iter().find(|e| e.name == "NodeContent").unwrap();
    assert!(
        node_content.variants.iter().all(|v| !v.is_default),
        "a struct-variant default body must not mark any variant `is_default`"
    );

    let node = surface.types.iter().find(|typ| typ.name == "Node").unwrap();
    let content_field = node.fields.iter().find(|field| field.name == "content").unwrap();

    assert_eq!(
        content_field.typed_default,
        Some(DefaultValue::Empty),
        "a data-carrying default variant must stay `Empty` rather than become a bare variant name"
    );
}

/// (c) The enum has no `Default` impl at all — `EnumDef::has_default` is false. The genuinely
/// unresolved case must stay `Empty`, preserving the existing honest fallback (every backend
/// already treats `Empty` on a `Named` field as "unknown" and guards accordingly, e.g. C#'s
/// `required`). This is the control: resolving a field must never happen when alef cannot know
/// the enum's default at all.
#[test]
fn enum_field_default_stays_empty_when_the_enum_has_no_default_impl() {
    let source = r#"
        pub enum Format {
            Plain,
            Rich,
        }

        pub struct Document {
            pub format: Format,
        }

        impl Default for Document {
            fn default() -> Self {
                Self { format: Format::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let format_enum = surface.enums.iter().find(|e| e.name == "Format").unwrap();
    assert!(
        !format_enum.has_default,
        "`Format` derives no `Default` and has no manual impl in this fixture"
    );

    let document = surface.types.iter().find(|typ| typ.name == "Document").unwrap();
    let format_field = document.fields.iter().find(|field| field.name == "format").unwrap();

    assert_eq!(
        format_field.typed_default,
        Some(DefaultValue::Empty),
        "a field whose enum type has no knowable default must stay `Empty`, not a guessed variant"
    );
}

/// (h) A bare field-level `#[serde(default)]` field whose declared enum type's real default is
/// *not* its first-declared variant, alongside a manual `impl Default` that sets the field
/// directly to a variant path (`OcrStrategy::Auto`, not `OcrStrategy::default()`). The field must
/// resolve to that concrete variant — never fall back to the first-declared one, and never stay
/// `Empty` — regardless of extra attributes (`deserialize_with`) riding alongside `default` on
/// the same `#[serde(...)]` list.
#[test]
fn bare_serde_default_field_resolves_to_the_manual_impls_direct_variant_path() {
    let source = r#"
        #[derive(Clone, Copy)]
        pub enum OcrStrategy {
            Never,
            Auto,
            Always,
        }

        pub struct ExtractionConfig {
            #[serde(default, deserialize_with = "deserialize_null_default")]
            pub ocr_strategy: OcrStrategy,
        }

        impl Default for ExtractionConfig {
            fn default() -> Self {
                Self {
                    ocr_strategy: OcrStrategy::Auto,
                }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "ExtractionConfig").unwrap();
    let ocr_strategy_field = config.fields.iter().find(|field| field.name == "ocr_strategy").unwrap();

    assert_eq!(
        ocr_strategy_field.typed_default,
        Some(DefaultValue::EnumVariant("Auto".to_string())),
        "a bare `#[serde(default)]` field must resolve to the manual impl's concrete variant, \
         not stay `Empty` or default to the first-declared variant"
    );
}
