//! A real `javac` oracle for wire names that are hostile to Java source.
//!
//! A `#[serde(rename = "...")]` value is an arbitrary Rust string that the Java backend pastes
//! into Java *source* — `@JsonProperty("...")`, `@JsonSubTypes.Type(name = "...")`,
//! `case "..." ->`, `gen.writeStringField("...", tag)` and javadoc. Substring assertions cannot
//! tell a well-formed literal from one that closed early, and the nastiest failure is not even a
//! syntax error: JLS §3.3 rewrites Unicode escapes before lexing, so a name containing
//! backslash-`u` either silently becomes a *different* wire name or is rejected as
//! `illegal unicode escape`.
//!
//! So this test asks `javac` instead. It generates bindings from an API surface whose every wire
//! name is hostile, lifts **every string literal in every generated file** into a probe class,
//! compiles it, runs it, and compares the code points Java actually decoded against the wire
//! names serde will produce. Lifting every literal rather than a hand-picked list is deliberate:
//! it covers sinks nobody remembered to look at.
//!
//! [`unescaped_emission_is_the_defect_this_guards`] is the negative control. It builds the same
//! probe the way the backend built these literals before the fix — quote, raw name, quote — and
//! requires `javac` to reject it. Without that arm a probe that accidentally carried no hostile
//! content would pass just as green. ~keep

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use std::path::Path;
use std::process::Command;

/// A Java Unicode escape is spelled with `concat!` so no line of this file contains a bare
/// backslash-`u` sequence for a tool in the authoring chain to reinterpret. ~keep
const HOSTILE_UNICODE: &str = concat!("\\", "u0041");

/// The same shape but malformed, which `javac` rejects outright instead of silently rewriting.
const HOSTILE_BAD_UNICODE: &str = concat!("\\", "uZZZZ");

const HOSTILE_QUOTE: &str = r#"quote"break"#;
const HOSTILE_BACKSLASH: &str = r"back\slash";
const HOSTILE_TRAILING_BACKSLASH: &str = r"trailing\";
const HOSTILE_CONTROL: &str = "ctl\u{1b}\u{7}\u{0}x";
const HOSTILE_NEWLINE: &str = "two\nlines\rmore";
const HOSTILE_NON_ASCII: &str = "caf\u{e9}\u{2028}\u{1F600}";
const HOSTILE_BREAKOUT: &str = r#"x") String injected; //"#;
const HOSTILE_COMMENT_CLOSE: &str = "close*/here";

/// One wire name per hazard class the escaper claims to handle.
const HOSTILE_NAMES: &[&str] = &[
    HOSTILE_QUOTE,
    HOSTILE_BACKSLASH,
    HOSTILE_TRAILING_BACKSLASH,
    HOSTILE_UNICODE,
    HOSTILE_BAD_UNICODE,
    HOSTILE_CONTROL,
    HOSTILE_NEWLINE,
    HOSTILE_NON_ASCII,
    HOSTILE_BREAKOUT,
    HOSTILE_COMMENT_CLOSE,
];

const PROBE_PACKAGE_DIRECTORY: &str = "dev/sample";
const PROBE_PACKAGE: &str = "dev.sample";

fn hostile_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "dev.sample"

[crates.java.dto]
builder = "always"
"#,
    )
    .expect("valid hostile-name Java config");
    config.resolve().expect("resolved hostile-name Java config").remove(0)
}

fn renamed_field(index: usize, wire_name: &str) -> FieldDef {
    FieldDef {
        name: format!("field_{index}"),
        ty: TypeRef::String,
        serde_rename: Some(wire_name.to_owned()),
        ..Default::default()
    }
}

fn renamed_variant(index: usize, wire_name: &str) -> EnumVariant {
    EnumVariant {
        name: format!("Variant{index}"),
        serde_rename: Some(wire_name.to_owned()),
        ..Default::default()
    }
}

/// Every hostile name is used twice — once as a struct field rename and once as an enum variant
/// rename — so each one is guaranteed to reach both the `@JsonProperty` family of sinks and the
/// discriminator family, whichever branch the enum emitter takes. ~keep
fn hostile_api() -> ApiSurface {
    let fields: Vec<FieldDef> = HOSTILE_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| renamed_field(index, name))
        .collect();
    let variants: Vec<EnumVariant> = HOSTILE_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| renamed_variant(index, name))
        .collect();

    ApiSurface {
        crate_name: "sample_crate".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![TypeDef {
            name: "Payload".to_owned(),
            rust_path: "sample_crate::Payload".to_owned(),
            fields,
            is_clone: true,
            has_serde: true,
            ..Default::default()
        }],
        enums: vec![
            simple_enum(variants),
            internally_tagged_enum(),
            adjacently_tagged_enum(),
        ],
        ..Default::default()
    }
}

fn simple_enum(variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "Kind".to_owned(),
        rust_path: "sample_crate::Kind".to_owned(),
        variants,
        excluded_variants: vec![EnumVariant {
            name: "Hidden".to_owned(),
            serde_rename: Some(HOSTILE_COMMENT_CLOSE.to_owned()),
            binding_excluded: true,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }
}

/// `#[serde(tag = "...")]` with struct-shaped variants: exercises `@JsonTypeInfo(property)`,
/// `@JsonSubTypes.Type(name)` and the struct-variant field `@JsonProperty`.
fn internally_tagged_enum() -> EnumDef {
    EnumDef {
        name: "Tagged".to_owned(),
        rust_path: "sample_crate::Tagged".to_owned(),
        serde_tag: Some(HOSTILE_QUOTE.to_owned()),
        variants: vec![
            EnumVariant {
                name: "First".to_owned(),
                serde_rename: Some(HOSTILE_BACKSLASH.to_owned()),
                fields: vec![renamed_field(0, HOSTILE_UNICODE), renamed_field(1, HOSTILE_NEWLINE)],
                ..Default::default()
            },
            EnumVariant {
                name: "Second".to_owned(),
                serde_rename: Some(HOSTILE_BREAKOUT.to_owned()),
                fields: vec![renamed_field(0, HOSTILE_CONTROL)],
                ..Default::default()
            },
        ],
        has_serde: true,
        ..Default::default()
    }
}

/// `#[serde(tag, content)]`: the only shape that reaches the hand-written sealed-union codecs,
/// where the tag and content keys are pasted into `node.get(...)`, `node.remove(...)`,
/// `gen.writeStringField(...)`, `gen.writeFieldName(...)` and the `case` labels.
fn adjacently_tagged_enum() -> EnumDef {
    EnumDef {
        name: "Adjacent".to_owned(),
        rust_path: "sample_crate::Adjacent".to_owned(),
        serde_tag: Some(HOSTILE_TRAILING_BACKSLASH.to_owned()),
        serde_content: Some(HOSTILE_BAD_UNICODE.to_owned()),
        variants: vec![
            EnumVariant {
                name: "Empty".to_owned(),
                serde_rename: Some(HOSTILE_NON_ASCII.to_owned()),
                ..Default::default()
            },
            EnumVariant {
                name: "Wrapped".to_owned(),
                serde_rename: Some(HOSTILE_COMMENT_CLOSE.to_owned()),
                fields: vec![FieldDef {
                    name: "0".to_owned(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        ..Default::default()
    }
}

fn generated_java() -> Vec<String> {
    JavaBackend
        .generate_bindings(&hostile_api(), &hostile_config())
        .expect("hostile-name Java generation must succeed")
        .into_iter()
        .map(|file| file.content)
        .collect()
}

// ---------------------------------------------------------------------------
// A minimal Java lexer: enough to separate string literals from comments so the
// probes below quote each back verbatim without confusing one for the other.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct JavaTokens {
    /// String literals including their surrounding quotes, verbatim from the generated source.
    literals: Vec<String>,
    /// Comments including their delimiters, verbatim from the generated source.
    comments: Vec<String>,
}

fn scan_java(source: &str) -> JavaTokens {
    let mut tokens = JavaTokens::default();
    let characters: Vec<char> = source.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '"' => index = take_string_literal(&characters, index, &mut tokens.literals),
            '/' if characters.get(index + 1) == Some(&'/') => {
                index = take_line_comment(&characters, index, &mut tokens.comments);
            }
            '/' if characters.get(index + 1) == Some(&'*') => {
                index = take_block_comment(&characters, index, &mut tokens.comments);
            }
            '\'' => index = skip_char_literal(&characters, index),
            _ => index += 1,
        }
    }
    tokens
}

fn take_string_literal(source: &[char], start: usize, out: &mut Vec<String>) -> usize {
    let mut index = start + 1;
    while index < source.len() {
        match source[index] {
            '\\' => index += 2,
            '"' => {
                out.push(source[start..=index].iter().collect());
                return index + 1;
            }
            '\n' => break,
            _ => index += 1,
        }
    }
    panic!("generated Java contains an unterminated string literal at character {start}");
}

fn take_line_comment(source: &[char], start: usize, out: &mut Vec<String>) -> usize {
    let mut index = start;
    while index < source.len() && source[index] != '\n' {
        index += 1;
    }
    out.push(source[start..index].iter().collect());
    index
}

fn take_block_comment(source: &[char], start: usize, out: &mut Vec<String>) -> usize {
    let mut index = start + 2;
    while index + 1 < source.len() {
        if source[index] == '*' && source[index + 1] == '/' {
            out.push(source[start..index + 2].iter().collect());
            return index + 2;
        }
        index += 1;
    }
    panic!("generated Java contains an unterminated block comment at character {start}");
}

fn skip_char_literal(source: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < source.len() {
        match source[index] {
            '\\' => index += 2,
            '\'' => return index + 1,
            _ => index += 1,
        }
    }
    start + 1
}

// ---------------------------------------------------------------------------
// Probe construction
// ---------------------------------------------------------------------------

/// A class whose only content is every literal the generator emitted, plus a `main` that prints
/// each one back as decimal code points so the Rust side can compare exact values.
fn literal_probe(literals: &[String]) -> String {
    let mut source = format!("package {PROBE_PACKAGE};\n\npublic final class LiteralProbe {{\n");
    source.push_str("    private static final String[] LITERALS = {\n");
    for literal in literals {
        source.push_str("        ");
        source.push_str(literal);
        source.push_str(",\n");
    }
    source.push_str("    };\n\n");
    source.push_str("    public static void main(final String[] args) {\n");
    source.push_str("        for (String value : LITERALS) {\n");
    source.push_str("            StringBuilder line = new StringBuilder();\n");
    source.push_str("            value.codePoints().forEach(c -> line.append(c).append(' '));\n");
    source.push_str("            System.out.println(line.toString().trim());\n");
    source.push_str("        }\n");
    source.push_str("    }\n}\n");
    source
}

/// A class whose only content is every comment the generator emitted. A comment that closes early
/// — or that carries a Unicode escape the pre-lexer expands into a line terminator — is a
/// `javac` error here even though it is invisible to any substring assertion. ~keep
fn comment_probe(comments: &[String]) -> String {
    let mut source = format!("package {PROBE_PACKAGE};\n\npublic final class CommentProbe {{\n");
    for (index, comment) in comments.iter().enumerate() {
        source.push_str("    ");
        source.push_str(comment);
        source.push('\n');
        source.push_str(&format!("    static final int MARKER_{index} = {index};\n"));
    }
    source.push_str("}\n");
    source
}

// ---------------------------------------------------------------------------
// javac / java drivers
// ---------------------------------------------------------------------------

fn java_available() -> bool {
    Command::new("javac").arg("-version").output().is_ok()
}

fn write_source(directory: &Path, relative: &str, contents: &str) {
    let path = directory.join(relative);
    std::fs::create_dir_all(path.parent().expect("probe parent directory")).expect("probe directory");
    std::fs::write(path, contents).expect("write Java probe");
}

/// Runs `javac` and returns its stderr on failure, `None` on success. Deliberately not an
/// assertion so the negative control can require a *failure*.
fn compile(directory: &Path, relative: &str) -> Option<String> {
    let output = Command::new("javac")
        .args(["-encoding", "UTF-8", relative])
        .current_dir(directory)
        .output()
        .expect("run javac");
    if output.status.success() {
        None
    } else {
        Some(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn run_probe(directory: &Path, class: &str) -> String {
    let output = Command::new("java")
        .args(["-cp", ".", class])
        .current_dir(directory)
        .output()
        .expect("run java probe");
    assert!(
        output.status.success(),
        "probe {class} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn code_points(value: &str) -> Vec<u32> {
    value.chars().map(u32::from).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Every string literal in every generated file must compile, and the ones carrying wire names
/// must decode back to exactly the bytes serde will put on the wire.
#[test]
fn every_generated_java_literal_compiles_and_round_trips_its_wire_name() {
    if !java_available() {
        return;
    }
    let mut literals: Vec<String> = Vec::new();
    for source in generated_java() {
        literals.extend(scan_java(&source).literals);
    }
    assert!(
        literals.len() > HOSTILE_NAMES.len(),
        "generation produced too few literals to be exercising the sinks: {}",
        literals.len()
    );

    let directory = tempfile::tempdir().expect("temporary Java literal probe directory");
    let relative = format!("{PROBE_PACKAGE_DIRECTORY}/LiteralProbe.java");
    write_source(directory.path(), &relative, &literal_probe(&literals));
    if let Some(stderr) = compile(directory.path(), &relative) {
        panic!("generated Java literals do not compile:\n{stderr}");
    }

    let decoded: Vec<Vec<u32>> = run_probe(directory.path(), &format!("{PROBE_PACKAGE}.LiteralProbe"))
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(|point| point.parse::<u32>().expect("decimal code point"))
                .collect()
        })
        .collect();

    for name in HOSTILE_NAMES {
        assert!(
            decoded.contains(&code_points(name)),
            "wire name {name:?} never reached a generated Java literal, or did not survive it"
        );
    }
}

/// The comment sinks: excluded-variant listings and the sealed-union codec javadoc both paste a
/// wire name into comment text, where `*/` closes the comment and a Unicode escape is still
/// expanded before lexing.
#[test]
fn every_generated_java_comment_compiles() {
    if !java_available() {
        return;
    }
    let mut comments: Vec<String> = Vec::new();
    for source in generated_java() {
        comments.extend(scan_java(&source).comments);
    }
    assert!(!comments.is_empty(), "generation produced no comments to check");

    let directory = tempfile::tempdir().expect("temporary Java comment probe directory");
    let relative = format!("{PROBE_PACKAGE_DIRECTORY}/CommentProbe.java");
    write_source(directory.path(), &relative, &comment_probe(&comments));
    if let Some(stderr) = compile(directory.path(), &relative) {
        panic!("generated Java comments do not compile:\n{stderr}");
    }
}

/// Negative control. Rebuilds the probes the way the backend built these sinks before the fix —
/// a quote, the raw wire name, a quote — and requires `javac` to reject both. If this ever
/// passes, the fixture above has stopped carrying hostile content and its green is meaningless.
#[test]
fn unescaped_emission_is_the_defect_this_guards() {
    if !java_available() {
        return;
    }
    let raw_literals: Vec<String> = HOSTILE_NAMES.iter().map(|name| format!("\"{name}\"")).collect();
    let raw_comments: Vec<String> = HOSTILE_NAMES.iter().map(|name| format!("/** wire {name} */")).collect();

    let directory = tempfile::tempdir().expect("temporary Java negative-control directory");
    let literal_relative = format!("{PROBE_PACKAGE_DIRECTORY}/LiteralProbe.java");
    write_source(directory.path(), &literal_relative, &literal_probe(&raw_literals));
    assert!(
        compile(directory.path(), &literal_relative).is_some(),
        "unescaped wire names compiled — the hostile fixture is not reproducing the defect"
    );

    let comment_relative = format!("{PROBE_PACKAGE_DIRECTORY}/CommentProbe.java");
    write_source(directory.path(), &comment_relative, &comment_probe(&raw_comments));
    assert!(
        compile(directory.path(), &comment_relative).is_some(),
        "unescaped wire names in comments compiled — the hostile fixture is not reproducing the defect"
    );
}
