use crate::core::config::Language;
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

pub(crate) fn lang_display_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "Python",
        Language::Node => "TypeScript",
        Language::Ruby => "Ruby",
        Language::Php => "PHP",
        Language::Elixir => "Elixir",
        Language::Go => "Go",
        Language::Java => "Java",
        Language::Csharp => "C#",
        Language::Ffi | Language::C | Language::Jni => "C",
        Language::Wasm => "WebAssembly",
        Language::R => "R",
        Language::Rust => "Rust",
        Language::Kotlin => "Kotlin",
        Language::KotlinAndroid => "Kotlin (Android)",
        Language::Swift => "Swift",
        Language::Dart => "Dart",
        Language::Gleam => "Gleam",
        Language::Zig => "Zig",
    }
}

/// Get the slug used in file names (e.g. `typescript` for `Node`).
pub(crate) fn lang_slug(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi | Language::C | Language::Jni => "c",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin-android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Get the code fence language identifier.
pub(crate) fn lang_code_fence(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node | Language::Wasm => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi | Language::C | Language::Jni => "c",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin | Language::KotlinAndroid => "kotlin",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Convert a Rust type name to the idiomatic name for the target language.
pub(crate) fn type_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    match lang {
        Language::Python
        | Language::Node
        | Language::Wasm
        | Language::Ruby
        | Language::Go
        | Language::Java
        | Language::Csharp
        | Language::Php
        | Language::Elixir
        | Language::R
        | Language::Rust
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => short.to_pascal_case(),
        // cbindgen renames every exported type with `[export] prefix`. Callers hand this
        // function the PascalCase form of `[ffi] prefix` (docs::generate_docs), so a
        // case-restoring conversion is required to recover the header spelling: for a consumer
        // whose `[ffi] prefix` is a single lowercase word such as `demoapi`, the header really
        // does say `DEMOAPIDefaultClient`, and emitting `DemoapiDefaultClient` names a symbol
        // that occurs zero times in it.
        //
        // Both sides now use the same conversion: `gen_cbindgen_toml`
        // (backends/ffi/gen_bindings/helpers.rs:392) computes the real `[export] prefix` as
        // `prefix.to_shouty_snake_case()`, which is what this arm applies too. An earlier
        // version of this note recorded a divergence -- `gen_cbindgen_toml` using plain
        // `.to_uppercase()`, so `SampleCore` became `SAMPLECORE` in the header but
        // `SAMPLE_CORE` here -- and instructed readers not to "fix" it. That divergence is
        // gone; the instruction would now reintroduce it. Matches the `enum_variant_name`
        // arm below. ~keep
        Language::Ffi | Language::C | Language::Jni => {
            format!("{}{}", ffi_prefix.to_shouty_snake_case(), short.to_pascal_case())
        }
    }
}

/// Convert a Rust function name to the idiomatic name for the target language.
pub(crate) fn func_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let base = match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::R | Language::Rust | Language::Zig => {
            name.to_snake_case()
        }
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
        Language::Csharp | Language::Go => name.to_pascal_case(),
        // `func_name` is fed Rust `fn` names, which arrive already snake_case, so routing
        // through `c_consumer::free_function_symbol` (which applies `pascal_to_snake` rather
        // than heck's `to_snake_case`) is a no-op rename here -- both conversions are the
        // identity on an already-snake_case input. Kept as the single source of the free-
        // function symbol shape so this and `render_c_fn_sig` cannot re-derive it
        // independently. ~keep
        Language::Ffi | Language::C | Language::Jni => {
            crate::codegen::c_consumer::free_function_symbol(&ffi_prefix.to_snake_case(), name)
        }
        Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart | Language::Gleam => {
            to_camel_case(name)
        }
    };
    // ~keep Java's keyword renames must match `safe_java_method_name`
    // (backends/java/gen_bindings/helpers.rs:207-215), which is what the Java backend applies
    // to every opaque-type method: `default` -> `defaultInstance`, `new` -> `create`, any
    // other keyword collision -> a trailing-underscore form. The previous hand-written arm
    // emitted `defaultOptions`, a name the backend never generates, and carried no `new` arm
    // at all -- so an opaque type's default static constructor (`pub fn new`, the shape the
    // IR carries for it) arrived at `assert_valid_identifier` as the Java reserved word `new`
    // and panicked the whole docs run instead of documenting `create`. Mirrored rather than
    // delegated because the backend applies it only to opaque methods, while this function
    // also names free functions (examples.rs, language_pages/function_render.rs); the mirror
    // is arm-for-arm and shares the same `JAVA_KEYWORDS` constant, and
    // `test_func_name_java_matches_backend_safe_java_method_name` pins it across that whole
    // constant rather than a sample. The two camel-case helpers differ in name only for these
    // inputs -- every entry in `JAVA_KEYWORDS` is a plain lowercase word, which both leave
    // unchanged, so the membership arm keys identically on both sides.
    //
    // Known shared blind spot, deliberately mirrored rather than silently diverged from:
    // `true`, `false`, and `null` are reserved *literals*, not keywords, so they are absent
    // from `JAVA_KEYWORDS` and neither this table nor `safe_java_method_name` renames them.
    // The backend would emit non-compiling Java for such a method and the docs gate
    // (`reserved_words(Java)` in formatting.rs, which does list all three) would panic. Fixing
    // that belongs in `safe_java_method_name`; this table must follow it, not lead it.
    match (lang, base.as_str()) {
        (Language::Java, "default") => "defaultInstance".to_string(),
        (Language::Java, "new") => "create".to_string(),
        (Language::Java, other) if crate::core::keywords::JAVA_KEYWORDS.contains(&other) => format!("{other}_"),
        (Language::Csharp, "Default") => "CreateDefault".to_string(),
        _ => base,
    }
}

/// Convert a Rust method name to the idiomatic name for the target language, folding in the
/// owning type for C.
///
/// A C symbol has no namespace, so the backend folds the owning type into the name
/// (`gen_method_wrapper` / `gen_streaming_method_wrapper` emit `{prefix}_{type_snake}_{method}`).
/// Every other language documents a method as a member of its owning type already -- the type
/// is spelled once, in the signature's receiver or class heading -- so `func_name`'s per-language
/// rules (including the Java keyword renames) keep applying unchanged there.
pub(crate) fn method_name(owner_type: &str, name: &str, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Ffi | Language::C | Language::Jni => {
            crate::codegen::c_consumer::method_symbol(&ffi_prefix.to_snake_case(), owner_type, name)
        }
        _ => func_name(name, lang, ffi_prefix),
    }
}

/// Convert a Rust field name to the idiomatic name for the target language.
pub(crate) fn field_name(name: &str, lang: Language) -> String {
    match lang {
        Language::Python
        | Language::Ruby
        | Language::Elixir
        | Language::R
        | Language::Ffi
        | Language::Rust
        | Language::C
        | Language::Jni
        | Language::Zig => name.to_snake_case(),
        Language::Go | Language::Csharp => name.to_pascal_case(),
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
        Language::Kotlin | Language::KotlinAndroid | Language::Swift | Language::Dart | Language::Gleam => {
            to_camel_case(name)
        }
    }
}

/// Convert a Rust enum variant name to the idiomatic name for the target language.
pub(crate) fn enum_variant_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    if name == "RDFa" {
        return match lang {
            Language::Python | Language::Java => "RDFA".to_string(),
            Language::Ruby | Language::Elixir | Language::Zig => "rdfa".to_string(),
            Language::R => "rdfa".to_string(),
            Language::Ffi | Language::C | Language::Jni => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), "RDFA"),
            _ => "RDFa".to_string(),
        };
    }
    match lang {
        Language::Python => name.to_shouty_snake_case(),
        Language::Java => name.to_shouty_snake_case(),
        Language::Ruby | Language::Elixir | Language::Zig => name.to_snake_case(),
        Language::Go
        | Language::Node
        | Language::Wasm
        | Language::Csharp
        | Language::Php
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam => name.to_pascal_case(),
        Language::R => name.to_snake_case(),
        Language::Rust => name.to_pascal_case(),
        Language::Ffi | Language::C | Language::Jni => {
            format!("{}_{}", ffi_prefix.to_shouty_snake_case(), name.to_shouty_snake_case())
        }
    }
}

/// Convert snake_case or PascalCase to camelCase.
pub(crate) fn to_camel_case(s: &str) -> String {
    let pascal = s.to_upper_camel_case();
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::docs::test_helpers::TEST_PREFIX;

    #[test]
    fn test_enum_variant_name_python() {
        assert_eq!(enum_variant_name("Atx", Language::Python, TEST_PREFIX), "ATX");
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Python, TEST_PREFIX),
            "SNAKE_CASE"
        );
    }

    #[test]
    fn test_enum_variant_name_java() {
        assert_eq!(enum_variant_name("Atx", Language::Java, TEST_PREFIX), "ATX");
    }

    #[test]
    fn test_enum_variant_name_ffi() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, TEST_PREFIX), "HTM_ATX");
    }

    #[test]
    fn test_type_name_ffi_uses_prefix() {
        assert_eq!(
            type_name("ParseOptions", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATEParseOptions"
        );
        assert_eq!(
            type_name("ParseOutput", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATEParseOutput"
        );
    }

    /// The C reference page must spell a type exactly as it appears in the emitted header.
    ///
    /// `alef all` used to publish `DemoapiDefaultClient` while the emitted `demo_api.h` declared
    /// `DEMOAPIDefaultClient` -- a name that occurs zero times in the header, so every C
    /// snippet on the same site contradicted the reference page. Observed in a consumer repo
    /// whose `[ffi] prefix` is a single lowercase word.
    ///
    /// The lowercase, single-word and already-underscored prefixes below do not discriminate
    /// between conversions -- `to_shouty_snake_case` and a plain `.to_uppercase()` return the
    /// same string for all of them, so pinning only those would pass no matter which one
    /// `gen_cbindgen_toml` used. `SampleCore` is the discriminating case: it is the one shape
    /// where the two disagree (`SAMPLE_CORE` vs `SAMPLECORE`), so it is the only row here that
    /// can actually fail if the header side stops using `to_shouty_snake_case`
    /// (backends/ffi/gen_bindings/helpers.rs:392). ~keep
    #[test]
    fn type_name_ffi_matches_cbindgen_export_prefix() {
        for (ffi_prefix, expected_export_prefix) in [
            ("demoapi", "DEMOAPI"),
            ("Demoapi", "DEMOAPI"),
            ("demo_api", "DEMO_API"),
            ("SampleCore", "SAMPLE_CORE"),
        ] {
            assert_eq!(
                type_name("DefaultClient", Language::C, ffi_prefix),
                format!("{expected_export_prefix}DefaultClient"),
                "docs must use cbindgen's export prefix for `{ffi_prefix}`"
            );
            assert_eq!(
                type_name("sample_crate::DefaultClient", Language::Ffi, ffi_prefix),
                format!("{expected_export_prefix}DefaultClient"),
                "a fully qualified rust path must resolve to the same header symbol"
            );
        }
    }

    #[test]
    fn test_func_name_ffi_uses_prefix() {
        assert_eq!(
            func_name("convert", Language::Ffi, "SampleCrate"),
            "sample_crate_convert"
        );
    }

    /// The whole point of `method_name`: a C symbol has no namespace, so the owning type must
    /// be folded into the name, or the docs publish a symbol that occurs zero times in the
    /// emitted header (the backend always emits `{prefix}_{type_snake}_{method}`, never
    /// `{prefix}_{method}`). Regresses if a call site falls back to `func_name` for C.
    #[test]
    fn test_method_name_ffi_folds_in_owning_type() {
        assert_eq!(
            method_name("Converter", "convert", Language::Ffi, TEST_PREFIX),
            "htm_converter_convert"
        );
        assert_ne!(
            method_name("Converter", "convert", Language::Ffi, TEST_PREFIX),
            func_name("convert", Language::Ffi, TEST_PREFIX),
            "a method symbol must not collide with the free-function symbol of the same name"
        );
    }

    /// For every non-C language, `method_name` must be an exact passthrough to `func_name` --
    /// including its Java keyword renames -- since the owning type is already spelled
    /// elsewhere in those languages' method docs (receiver, class heading). Regresses if
    /// `method_name` grows a second, divergent per-language table instead of delegating.
    #[test]
    fn test_method_name_non_c_delegates_to_func_name() {
        assert_eq!(method_name("Type", "new", Language::Java, TEST_PREFIX), "create");
        assert_eq!(
            method_name("Type", "new", Language::Java, TEST_PREFIX),
            func_name("new", Language::Java, TEST_PREFIX)
        );
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Csharp,
            Language::Ruby,
            Language::Zig,
        ] {
            assert_eq!(
                method_name("Document", "parse_document", lang, TEST_PREFIX),
                func_name("parse_document", lang, TEST_PREFIX),
                "{lang} must delegate to func_name unchanged"
            );
        }
    }

    #[test]
    fn test_enum_variant_name_ffi_uses_prefix() {
        assert_eq!(
            enum_variant_name("Atx", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATE_ATX"
        );
    }

    #[test]
    fn test_field_name_go_pascal_case() {
        assert_eq!(field_name("heading_style", Language::Go), "HeadingStyle");
        assert_eq!(field_name("list_indent_type", Language::Go), "ListIndentType");
    }

    #[test]
    fn test_func_name_conventions() {
        assert_eq!(func_name("convert", Language::Python, TEST_PREFIX), "convert");
        assert_eq!(
            func_name("parse_document", Language::Node, TEST_PREFIX),
            "parseDocument"
        );
        assert_eq!(func_name("parse_document", Language::Go, TEST_PREFIX), "ParseDocument");
        assert_eq!(func_name("convert", Language::Ffi, TEST_PREFIX), "htm_convert");
    }

    #[test]
    fn test_type_name_ffi_prefix() {
        assert_eq!(type_name("ParseOptions", Language::Ffi, TEST_PREFIX), "HTMParseOptions");
        assert_eq!(type_name("ParseOutput", Language::Ffi, TEST_PREFIX), "HTMParseOutput");
    }

    #[test]
    fn test_lang_slug_kotlin_vs_kotlin_android() {
        assert_eq!(lang_slug(Language::Kotlin), "kotlin");
        assert_eq!(lang_slug(Language::KotlinAndroid), "kotlin-android");
    }

    #[test]
    fn test_lang_display_name_kotlin_vs_kotlin_android() {
        assert_eq!(lang_display_name(Language::Kotlin), "Kotlin");
        assert_eq!(lang_display_name(Language::KotlinAndroid), "Kotlin (Android)");
    }

    #[test]
    fn test_lang_code_fence_kotlin_android_uses_kotlin() {
        assert_eq!(lang_code_fence(Language::Kotlin), "kotlin");
        assert_eq!(lang_code_fence(Language::KotlinAndroid), "kotlin");
    }

    #[test]
    fn test_func_name_zig_uses_snake_case() {
        assert_eq!(func_name("create_engine", Language::Zig, TEST_PREFIX), "create_engine");
        assert_eq!(func_name("map_urls", Language::Zig, TEST_PREFIX), "map_urls");
        assert_eq!(func_name("batch_scrape", Language::Zig, TEST_PREFIX), "batch_scrape");
    }

    #[test]
    fn test_field_name_zig_uses_snake_case() {
        assert_eq!(field_name("max_depth", Language::Zig), "max_depth");
        assert_eq!(field_name("user_agent", Language::Zig), "user_agent");
    }

    #[test]
    fn test_enum_variant_name_zig_uses_snake_case() {
        assert_eq!(enum_variant_name("Auto", Language::Zig, TEST_PREFIX), "auto");
        assert_eq!(enum_variant_name("Stealth", Language::Zig, TEST_PREFIX), "stealth");
        assert_eq!(
            enum_variant_name("NetworkIdle", Language::Zig, TEST_PREFIX),
            "network_idle"
        );
    }

    #[test]
    fn test_enum_variant_name_zig_rdfa_special_case_uses_snake_case() {
        assert_eq!(enum_variant_name("RDFa", Language::Zig, TEST_PREFIX), "rdfa");
    }

    #[test]
    fn test_type_name_zig_preserves_pascal_case() {
        assert_eq!(type_name("BrowserMode", Language::Zig, TEST_PREFIX), "BrowserMode");
        assert_eq!(type_name("CrawlConfig", Language::Zig, TEST_PREFIX), "CrawlConfig");
    }

    /// The default opaque constructor is a static `pub fn new` carried in `TypeDef::methods`.
    /// The Java backend renames it to `create`; documenting it as `new` names a Java reserved
    /// word, which `assert_valid_identifier` turns into a panic that aborts the whole docs run.
    #[test]
    fn test_func_name_java_renames_reserved_new_to_create() {
        assert_eq!(func_name("new", Language::Java, TEST_PREFIX), "create");
    }

    /// `defaultOptions` was a docs-only invention; the backend emits `defaultInstance`.
    #[test]
    fn test_func_name_java_renames_default_to_default_instance() {
        assert_eq!(func_name("default", Language::Java, TEST_PREFIX), "defaultInstance");
    }

    #[test]
    fn test_func_name_java_suffixes_other_reserved_words() {
        assert_eq!(func_name("class", Language::Java, TEST_PREFIX), "class_");
        assert_eq!(func_name("static", Language::Java, TEST_PREFIX), "static_");
    }

    #[test]
    fn test_func_name_java_leaves_ordinary_names_alone() {
        assert_eq!(
            func_name("parse_document", Language::Java, TEST_PREFIX),
            "parseDocument"
        );
        assert_eq!(func_name("create", Language::Java, TEST_PREFIX), "create");
    }

    /// Pin the docs table against the Java backend's own, so the two cannot drift apart
    /// silently the way `defaultOptions` did.
    ///
    /// Exhaustive over `JAVA_KEYWORDS`, not a sample: `safe_java_method_name`'s third arm is a
    /// membership test against that whole constant, so a sampled cross-check would pass while
    /// leaving 40-odd keywords free to diverge. The non-keyword names cover the fallthrough
    /// arm, where the two sides use different (but equivalent) camel-case helpers -- docs'
    /// `to_camel_case` vs heck's `to_lower_camel_case`.
    #[test]
    fn test_func_name_java_matches_backend_safe_java_method_name() {
        let ordinary = ["parse_document", "to_json", "create", "with_options", "from_str", "id"];
        for name in crate::core::keywords::JAVA_KEYWORDS.iter().copied().chain(ordinary) {
            assert_eq!(
                func_name(name, Language::Java, TEST_PREFIX),
                crate::backends::java::gen_bindings::helpers::safe_java_method_name(name),
                "docs must name `{name}` exactly as the Java backend does"
            );
        }
    }

    /// A Java identifier the docs emit must survive the identifier gate; before the rename
    /// table was corrected, `new` reached it verbatim and panicked. Exhaustive over the
    /// keyword table so no reserved word can reach the gate unrenamed.
    #[test]
    fn test_func_name_java_output_passes_the_identifier_gate() {
        let ordinary = ["parse_document", "to_json", "create"];
        for name in crate::core::keywords::JAVA_KEYWORDS.iter().copied().chain(ordinary) {
            let rendered = func_name(name, Language::Java, TEST_PREFIX);
            crate::docs::formatting::assert_valid_identifier(&rendered, Language::Java, "a naming test");
        }
    }
}
