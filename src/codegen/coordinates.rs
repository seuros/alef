//! Grammar validation for cross-language publish/namespace coordinates.
//!
//! Each target ecosystem has its own coordinate grammar (a Maven groupId is not a Java
//! package; a NuGet package ID is not a C# namespace; a SwiftPM module name is not a
//! package name). This module validates each grammar separately so a backend can never
//! silently reuse one coordinate's rules -- or another coordinate's value -- for a
//! different one. See the `centralized-naming` rule: casing lives in `naming.rs`, but
//! coordinate *grammar* validation is centralized here for the same reason.
//!
//! Every validator accepts every value a currently-working default configuration would
//! produce; only explicit, out-of-grammar user input is rejected.

use crate::codegen::identifier_grammar::{
    is_csharp_identifier_part, is_csharp_identifier_start, is_java_identifier_part, is_java_identifier_start,
};

/// Java reserved keywords and literals (JLS SE 21 §3.9, §3.10.3, §3.10.7) that cannot
/// appear as an `Identifier`, and therefore cannot appear as a package-name segment
/// (JLS §7.4.1: `PackageName: Identifier | PackageName . Identifier`). Restricted
/// identifiers such as `var`, `yield`, `record`, `sealed`, and `permits` are
/// deliberately NOT listed: the JLS still permits them as ordinary identifiers outside
/// their restricted syntactic positions, so rejecting them here would reject valid
/// package names.
const JAVA_RESERVED: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "true",
    "false",
    "null",
    // `_` is a keyword as of Java 9 (JLS SE 21 3.9) even though
    // `Character.isJavaIdentifierStart('_')` is true, so the character grammar cannot catch it.
    // Confirmed against javac 25.0.2: `package probe._;` fails, `package probe._core;` compiles.
    // C# has no equivalent rule -- `namespace Probe._` compiles under dotnet 10.0.100. ~keep
    "_",
];

/// Kotlin hard keywords (kotlinlang.org/docs/keyword-reference.html "Hard keywords") that
/// cannot be used as an identifier without backtick-escaping. A JVM package emitted by
/// alef also carries the Kotlin/Kotlin-Android facade, so a package segment must be a
/// valid identifier in both languages. Soft and modifier keywords (e.g. `by`,
/// `constructor`, `data`) are intentionally excluded: Kotlin permits them unescaped.
const KOTLIN_HARD_KEYWORDS: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
    "while",
];

/// The identifier grammar one dot-separated coordinate segment must satisfy. A Java package, a
/// Kotlin package, and a C# namespace share this shape -- `Identifier ('.' Identifier)*` -- but
/// each admits a *different* set of characters, so the character classes are supplied per
/// language rather than approximated by one shared pair of predicates.
struct SegmentGrammar {
    language: &'static str,
    is_start: fn(char) -> bool,
    is_part: fn(char) -> bool,
    start_hint: &'static str,
    reserved: &'static [&'static str],
}

/// A Java package segment, minus the ISO control characters `Character.isJavaIdentifierPart`
/// accepts.
///
/// The JLS really does admit those 56 code points inside an identifier, and
/// [`is_java_identifier_part`] mirrors that exactly. A *coordinate* is additionally spliced into
/// `pom.xml`, `build.gradle.kts`, and filesystem paths, where an invisible control character is
/// a spoofing vector that no working default configuration can produce, so coordinate validation
/// is deliberately stricter than the JLS on this one point. ~keep
fn is_java_package_part(character: char) -> bool {
    is_java_identifier_part(character) && !character.is_control()
}

const JAVA_SEGMENT_GRAMMAR: SegmentGrammar = SegmentGrammar {
    language: "Java",
    is_start: is_java_identifier_start,
    is_part: is_java_package_part,
    start_hint: "a letter, or a currency (`$`) or connector (`_`) character",
    reserved: JAVA_RESERVED,
};

/// Kotlin's own lexer accepts a narrower set than this: measured against kotlinc 2.4.10, a
/// Kotlin identifier starts with `Lu|Ll|Lt|Lm|Lo` or a literal `_` and continues with those plus
/// `Nd`, rejecting `Nl` and every connector but `_`. Tightening it belongs in a Kotlin lane with
/// its own oracle -- this repair covers Java and C# -- so the previous approximation is kept
/// verbatim here rather than half-corrected. ~keep
fn is_kotlin_package_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

fn is_kotlin_package_part(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

const KOTLIN_SEGMENT_GRAMMAR: SegmentGrammar = SegmentGrammar {
    language: "Kotlin",
    is_start: is_kotlin_package_start,
    is_part: is_kotlin_package_part,
    start_hint: "a letter or `_`",
    reserved: KOTLIN_HARD_KEYWORDS,
};

fn validate_package_segments(name: &str, grammar: &SegmentGrammar) -> Result<(), String> {
    if name.is_empty() {
        return Err("must not be empty".to_string());
    }
    for segment in name.split('.') {
        validate_one_segment(name, segment, grammar)?;
    }
    Ok(())
}

fn validate_one_segment(name: &str, segment: &str, grammar: &SegmentGrammar) -> Result<(), String> {
    let language = grammar.language;
    if segment.is_empty() {
        return Err(format!("`{name}` has an empty segment (leading/trailing/double dot)"));
    }
    let mut chars = segment.chars();
    let first = chars.next().expect("segment is non-empty");
    if !(grammar.is_start)(first) {
        return Err(format!(
            "segment `{segment}` in `{name}` must start with {}",
            grammar.start_hint
        ));
    }
    if let Some(bad) = chars.find(|character| !(grammar.is_part)(*character)) {
        return Err(format!(
            "segment `{segment}` in `{name}` contains `{bad}`, which is not a valid {language} identifier character"
        ));
    }
    if grammar.reserved.contains(&segment) {
        return Err(format!(
            "segment `{segment}` in `{name}` is a {language} reserved word and cannot be used unescaped"
        ));
    }
    Ok(())
}

/// Validate a Java package name using the JLS identifier grammar (JLS SE 21 3.8, 7.4.1), as
/// implemented by `Character.isJavaIdentifierStart`/`isJavaIdentifierPart`. Java keywords are
/// case-sensitive, and `$` is legal because it is a currency symbol -- as is every other
/// currency symbol, and every connector punctuation character.
pub fn validate_java_package(name: &str) -> Result<(), String> {
    validate_package_segments(name, &JAVA_SEGMENT_GRAMMAR)
}

/// Validate a Kotlin package name as it is emitted in Kotlin source. Kotlin keywords are
/// case-sensitive and `$` is not accepted in an unescaped source identifier.
pub fn validate_kotlin_package(name: &str) -> Result<(), String> {
    validate_package_segments(name, &KOTLIN_SEGMENT_GRAMMAR)
}

/// Validate a Maven `groupId` or `artifactId`. Maven Central's component validation
/// (Sonatype OSSRH publishing requirements) restricts both to ASCII letters, digits,
/// `.`, `_`, and `-`, since they become repository path segments; `..` or a leading/
/// trailing `.`/`-`/`/` would let a coordinate escape its repository directory. This is
/// deliberately more permissive than [`validate_jvm_package`] (Maven groupIds commonly
/// contain hyphens, e.g. `io.projectreactor.netty`) and deliberately ASCII-only (Maven
/// Central coordinates are conventionally ASCII; unlike a JVM package name, there is no
/// widely-interoperable non-ASCII Maven coordinate convention to preserve).
pub fn validate_maven_coordinate(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.contains("..") || value.contains('/') || value.contains('\\') {
        return Err(format!(
            "{field} `{value}` must not contain `..`, `/`, or `\\` (Maven coordinates become repository path segments)"
        ));
    }
    if value.starts_with('.') || value.starts_with('-') || value.ends_with('.') || value.ends_with('-') {
        return Err(format!("{field} `{value}` must not start or end with `.` or `-`"));
    }
    if let Some(bad) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "{field} `{value}` contains `{bad}`; Maven coordinates allow only ASCII letters, digits, `.`, `_`, and `-`"
        ));
    }
    Ok(())
}

/// ~keep NuGet package ID characters, per `NuGet.Packaging.PackageIdValidator`'s
/// `^\w+([_.-]\w+)*$` grammar. The pinned Unicode category table gives `\w` the .NET set:
/// letters, nonspacing marks, decimal digits, and connector punctuation. Callers must also
/// check case-insensitive collisions across a workspace.
pub fn validate_nuget_package_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("NuGet package ID must not be empty".to_string());
    }
    if value.encode_utf16().count() > 100 {
        return Err(format!(
            "NuGet package ID `{value}` exceeds the 100 UTF-16 code-unit limit"
        ));
    }
    if value.starts_with(['.', '-']) || value.ends_with(['.', '-']) {
        return Err(format!(
            "NuGet package ID `{value}` must not start or end with `.` or `-`"
        ));
    }
    if let Some(bad) = value
        .chars()
        .find(|character| !is_dotnet_word_character(*character) && !matches!(character, '.' | '-'))
    {
        return Err(format!(
            "NuGet package ID `{value}` contains `{bad}`; allowed characters are .NET word characters, `.`, and `-`"
        ));
    }
    if value
        .as_bytes()
        .windows(2)
        .any(|pair| matches!(pair, [b'.' | b'-', b'.' | b'-']))
    {
        return Err(format!(
            "NuGet package ID `{value}` must have a letter, digit, or `_` between `.`/`-` separators"
        ));
    }
    Ok(())
}

fn is_dotnet_word_character(character: char) -> bool {
    use unicode_general_category::{GeneralCategory, get_general_category};

    matches!(
        get_general_category(character),
        GeneralCategory::LowercaseLetter
            | GeneralCategory::UppercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::NonspacingMark
            | GeneralCategory::DecimalNumber
            | GeneralCategory::ConnectorPunctuation
    )
}

/// C# reserved keywords (ECMA-334 §6.4.3 / C# language specification, "Keywords") that
/// cannot appear as an `available_identifier`. A verbatim identifier (`@class`) can
/// escape a keyword, but alef does not emit verbatim identifiers for a generated
/// namespace, so a reserved word here would be a syntax error in the emitted source.
const CSHARP_RESERVED: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

const CSHARP_SEGMENT_GRAMMAR: SegmentGrammar = SegmentGrammar {
    language: "C#",
    is_start: is_csharp_identifier_start,
    is_part: is_csharp_identifier_part,
    start_hint: "a letter or `_`",
    reserved: CSHARP_RESERVED,
};

/// Validate a C# namespace: dot-separated identifiers per the C# language specification's
/// `qualified_identifier`, each an `identifier` under ECMA-334 6.4.3 and none of which is a
/// reserved keyword. Unicode letters are accepted, so `München.Parser` is a legal namespace.
///
/// This is *not* the Java package grammar with different keywords. Two differences bite:
/// a currency symbol starts a Java identifier but not a C# one, and Roslyn lexes UTF-16 code
/// units, so a supplementary-plane letter that `javac` accepts is a `CS1056` under `dotnet`.
/// Both are enforced by [`is_csharp_identifier_start`]/[`is_csharp_identifier_part`].
pub fn validate_csharp_namespace(name: &str) -> Result<(), String> {
    validate_package_segments(name, &CSHARP_SEGMENT_GRAMMAR)
}

/// Swift reserved keywords (Swift Language Reference, "Lexical Structure > Keywords and
/// Punctuation", swift.org/documentation) that require backtick-escaping to use as an
/// identifier.
const SWIFT_RESERVED: &[&str] = &[
    "associatedtype",
    "class",
    "deinit",
    "enum",
    "extension",
    "fileprivate",
    "func",
    "import",
    "init",
    "inout",
    "internal",
    "let",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "rethrows",
    "static",
    "struct",
    "subscript",
    "typealias",
    "var",
    "break",
    "case",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "fallthrough",
    "for",
    "guard",
    "if",
    "in",
    "repeat",
    "return",
    "switch",
    "where",
    "while",
    "as",
    "any",
    "catch",
    "false",
    "is",
    "nil",
    "self",
    "super",
    "throw",
    "throws",
    "true",
    "try",
];

/// Validate a Swift module name (SwiftPM `Package.swift` target/product `name:`). Swift
/// identifiers (Swift Language Reference, "Lexical Structure > Identifiers") start with a
/// Unicode letter or `_` and continue with letters, digits, or `_`; unlike a JVM package
/// or C# namespace, a Swift module name has no internal dot-separated structure -- it is
/// a single identifier, since it becomes the argument to `import` and is embedded in
/// mangled symbol names. SwiftPM rejects a target name containing `.` or `-` for the same
/// reason.
pub fn validate_swift_module_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("must not be empty".to_string());
    }
    let mut chars = name.chars();
    let first = chars.next().expect("name is non-empty");
    if !(first.is_alphabetic() || first == '_') {
        return Err(format!("`{name}` must start with a letter or `_`"));
    }
    if let Some(bad) = chars.find(|c| !(c.is_alphanumeric() || *c == '_')) {
        return Err(format!(
            "`{name}` contains `{bad}`; a Swift module name has no internal `.`/`-` structure and must \
             be a single identifier"
        ));
    }
    if SWIFT_RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "`{name}` is a Swift reserved keyword and cannot be used unescaped"
        ));
    }
    Ok(())
}

/// Validate a SwiftPM package name (`Package(name: "...")`). Unlike [`validate_swift_module_name`]
/// (which governs the *module*/target/import identity and must be a single Swift identifier),
/// the package's own `name:` argument is a free-form manifest label with no identifier
/// grammar of its own -- real published packages routinely use kebab-case
/// (`swift-argument-parser`, `swift-collections`), which a strict identifier check would
/// wrongly reject. It is still a Swift string literal, so the only requirement is that it
/// cannot break out of the literal or trigger `\(...)` string interpolation (Swift Language
/// Reference, "String Literals"): reject a literal `"`, `\`, or control character.
pub fn validate_swift_package_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("must not be empty".to_string());
    }
    if let Some(bad) = name.chars().find(|c| *c == '"' || *c == '\\' || c.is_control()) {
        return Err(format!(
            "`{name}` contains `{bad}`, which would break out of the Swift string literal `Package(name: \"...\")` \
             is spliced into"
        ));
    }
    Ok(())
}

/// Dart reserved words (dart.dev/language/keywords, "Reserved words" -- words that can
/// never be used as identifiers, as opposed to the language's "built-in" and
/// contextual/limited-reserved keywords, which remain valid identifiers).
const DART_RESERVED: &[&str] = &[
    "assert", "break", "case", "catch", "class", "const", "continue", "default", "do", "else", "enum", "extends",
    "false", "final", "finally", "for", "if", "in", "is", "new", "null", "rethrow", "return", "super", "switch",
    "this", "throw", "true", "try", "var", "void", "while", "with",
];

/// Validate a Dart/pub.dev package name. pub.dev package-name conventions (dart.dev/tools/
/// pub/pubspec, "Package name conventions") require `lowercase_with_underscores`: ASCII
/// lowercase letters, digits, and `_` only, starting with a lowercase letter, and must
/// not be a Dart reserved word.
pub fn validate_dart_package_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("must not be empty".to_string());
    }
    let mut chars = name.chars();
    let first = chars.next().expect("name is non-empty");
    if !first.is_ascii_lowercase() {
        return Err(format!("`{name}` must start with a lowercase ASCII letter"));
    }
    if let Some(bad) = chars.find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')) {
        return Err(format!(
            "`{name}` contains `{bad}`; pub.dev package names allow only lowercase ASCII letters, digits, and `_`"
        ));
    }
    if DART_RESERVED.contains(&name) {
        return Err(format!(
            "`{name}` is a Dart reserved word and cannot be used as a package name"
        ));
    }
    Ok(())
}

/// Escape a string for interpolation into a Kotlin (Gradle KTS) double-quoted string
/// literal. Beyond `"` and `\`, Kotlin string literals have an ACTIVE construct: `$identifier`
/// and `${expression}` trigger string templates (Kotlin language specification, "String
/// templates"), so a literal `$` must become `\$` or injected config data can reference (or,
/// via `${...}`, evaluate) arbitrary in-scope Gradle build-script symbols. Grammar validation
/// (e.g. [`validate_jvm_package`]) already rejects most injection shapes for identifier-typed
/// coordinates, but `$` is a legal JVM identifier character, so this escape is the remaining
/// defense for any coordinate spliced into a raw `.kts` string literal.
pub fn kotlin_string_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_and_kotlin_packages_accept_typical_defaults() {
        for value in ["dev.sample_core", "com.github.foo_org", "unconfigured.alef"] {
            assert!(validate_java_package(value).is_ok());
            assert!(validate_kotlin_package(value).is_ok());
        }
    }

    #[test]
    fn jvm_package_accepts_unicode_letters() {
        assert!(
            validate_java_package("München.parser").is_ok(),
            "non-ASCII letters are legal Java identifiers"
        );
    }

    #[test]
    fn jvm_package_rejects_empty_and_double_dot_segments() {
        assert!(validate_java_package("").is_err());
        assert!(validate_java_package("dev..example").is_err());
        assert!(validate_java_package(".dev.example").is_err());
        assert!(validate_java_package("dev.example.").is_err());
    }

    #[test]
    fn jvm_package_rejects_path_traversal_segments() {
        assert!(validate_java_package("../../etc").is_err());
    }

    #[test]
    fn java_and_kotlin_package_grammars_are_distinct_and_case_sensitive() {
        assert!(validate_java_package("dev.class").is_err());
        assert!(validate_java_package("dev.Class").is_ok());
        assert!(validate_java_package("dev.fun").is_ok());
        assert!(validate_java_package("dev.$internal").is_ok());
        assert!(validate_kotlin_package("dev.class").is_err());
        assert!(validate_kotlin_package("dev.Class").is_ok());
        assert!(validate_kotlin_package("dev.fun").is_err());
        assert!(validate_kotlin_package("dev.$internal").is_err());
    }

    #[test]
    fn java_package_accepts_restricted_identifiers() {
        assert!(
            validate_java_package("dev.var").is_ok(),
            "var is a restricted identifier, not a keyword"
        );
        assert!(
            validate_java_package("dev.yield").is_ok(),
            "yield is a restricted identifier, not a keyword"
        );
        assert!(
            validate_java_package("dev.record").is_ok(),
            "record is a restricted identifier, not a keyword"
        );
    }

    #[test]
    fn jvm_package_rejects_injection_characters() {
        assert!(validate_java_package("dev\";System.exit(1);//").is_err());
        assert!(validate_kotlin_package("dev.example;import evil.Class").is_err());
    }

    #[test]
    fn maven_coordinate_accepts_hyphenated_group_ids() {
        assert!(validate_maven_coordinate("groupId", "io.projectreactor.netty").is_ok());
        assert!(validate_maven_coordinate("artifactId", "my-lib-android").is_ok());
    }

    #[test]
    fn maven_coordinate_rejects_empty_and_path_traversal() {
        assert!(validate_maven_coordinate("groupId", "").is_err());
        assert!(validate_maven_coordinate("groupId", "../../evil").is_err());
        assert!(validate_maven_coordinate("groupId", "dev/example").is_err());
        assert!(validate_maven_coordinate("groupId", ".dev").is_err());
        assert!(validate_maven_coordinate("groupId", "dev.").is_err());
    }

    #[test]
    fn maven_coordinate_rejects_quote_and_interpolation_characters() {
        assert!(validate_maven_coordinate("groupId", "dev\"); System.exit(1); //").is_err());
        assert!(validate_maven_coordinate("groupId", "dev${evil}").is_err());
    }

    #[test]
    fn nuget_package_id_accepts_typical_ids() {
        assert!(validate_nuget_package_id("MyCompany.MyLib").is_ok());
        assert!(validate_nuget_package_id("my_lib-android").is_ok());
    }

    #[test]
    fn nuget_package_id_matches_dotnet_word_character_categories() {
        assert!(validate_nuget_package_id("Cafe\u{301}").is_ok());
        assert!(validate_nuget_package_id("My\u{203f}Lib").is_ok());
        assert!(validate_nuget_package_id("München.Δοκιμή").is_ok());
        assert!(validate_nuget_package_id("Ⅻ").is_err());
        assert!(validate_nuget_package_id("²").is_err());
    }

    #[test]
    fn nuget_package_id_limit_counts_dotnet_utf16_code_units() {
        assert!(validate_nuget_package_id(&"𐐀".repeat(50)).is_ok());
        assert!(validate_nuget_package_id(&"𐐀".repeat(51)).is_err());
    }

    #[test]
    fn nuget_package_id_rejects_empty_leading_dot_and_quotes() {
        assert!(validate_nuget_package_id("").is_err());
        assert!(validate_nuget_package_id(".MyLib").is_err());
        assert!(validate_nuget_package_id("MyLib\"><Evil/>").is_err());
        assert!(validate_nuget_package_id("MyLib.").is_err());
        assert!(validate_nuget_package_id("MyLib-.Core").is_err());
        assert!(validate_nuget_package_id(&"a".repeat(101)).is_err());
        assert!(validate_nuget_package_id(&"a".repeat(100)).is_ok());
    }

    #[test]
    fn csharp_namespace_accepts_unicode_letters() {
        assert!(validate_csharp_namespace("München.Parser").is_ok());
    }

    #[test]
    fn csharp_namespace_rejects_reserved_words_and_empty_segments() {
        assert!(validate_csharp_namespace("My.class").is_err());
        assert!(validate_csharp_namespace("My..Lib").is_err());
        assert!(validate_csharp_namespace("").is_err());
    }

    #[test]
    fn csharp_namespace_rejects_msbuild_and_quote_characters() {
        assert!(validate_csharp_namespace("My\"><Evil/>").is_err());
        assert!(validate_csharp_namespace("My.$(Evil)").is_err());
    }

    #[test]
    fn swift_module_name_accepts_typical_names() {
        assert!(validate_swift_module_name("SampleCore").is_ok());
        assert!(validate_swift_module_name("_SampleCore").is_ok());
    }

    #[test]
    fn swift_module_name_rejects_dots_dashes_and_reserved_words() {
        assert!(validate_swift_module_name("Sample.Core").is_err());
        assert!(validate_swift_module_name("sample-core").is_err());
        assert!(validate_swift_module_name("class").is_err());
        assert!(validate_swift_module_name("").is_err());
    }

    #[test]
    fn swift_module_name_rejects_interpolation_characters() {
        assert!(validate_swift_module_name("Sample\\(evilCode)").is_err());
    }

    #[test]
    fn swift_package_name_accepts_kebab_case_and_dotted_names() {
        // Real published SwiftPM packages: apple/swift-argument-parser, apple/swift-collections.
        // `validate_swift_module_name`'s identifier grammar would wrongly reject these.
        assert!(validate_swift_package_name("swift-argument-parser").is_ok());
        assert!(validate_swift_package_name("Sample.Router").is_ok());
        assert!(validate_swift_package_name("Sample Router").is_ok());
    }

    #[test]
    fn swift_package_name_rejects_quote_and_backslash_but_not_dots() {
        assert!(validate_swift_package_name("Sample\"); print(\"evil").is_err());
        assert!(validate_swift_package_name("Sample\\(evilCode)").is_err());
        assert!(validate_swift_package_name("").is_err());
    }

    #[test]
    fn swift_package_name_rejects_control_characters() {
        for value in ["Sample\nInjected", "Sample\rInjected", "Sample\0Injected"] {
            assert!(
                validate_swift_package_name(value).is_err(),
                "`{value:?}` must be rejected"
            );
        }
    }

    #[test]
    fn dart_package_name_accepts_lowercase_with_underscores() {
        assert!(validate_dart_package_name("sample_core").is_ok());
    }

    #[test]
    fn dart_package_name_rejects_uppercase_dashes_and_reserved_words() {
        assert!(validate_dart_package_name("SampleCore").is_err());
        assert!(validate_dart_package_name("sample-core").is_err());
        assert!(validate_dart_package_name("var").is_err());
        assert!(validate_dart_package_name("").is_err());
    }

    #[test]
    fn kotlin_string_escape_neutralizes_dollar_template_interpolation() {
        assert_eq!(kotlin_string_escape("dev.$evilVar"), "dev.\\$evilVar");
        assert_eq!(kotlin_string_escape("dev.${evil()}"), "dev.\\${evil()}");
    }

    #[test]
    fn kotlin_string_escape_neutralizes_quotes_and_backslashes() {
        assert_eq!(
            kotlin_string_escape("dev\"); System.exit(1); //"),
            "dev\\\"); System.exit(1); //"
        );
        assert_eq!(kotlin_string_escape("dev\\example"), "dev\\\\example");
    }

    #[test]
    fn kotlin_string_escape_is_identity_for_a_plain_valid_package() {
        let plain = "dev.sample_core";
        assert_eq!(kotlin_string_escape(plain), plain);
    }
}
