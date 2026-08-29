//! Regression coverage for the `"stream.items.length" | "chunks.length"` accessor arm gaining a
//! `"kotlin_android"` case.
//!
//! ~keep Split into its own file rather than added to `tests.rs`: that file is already at 959
//! lines, close enough to the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md)
//! that growing it risks crossing the line entirely, which the size ratchet treats as a hard
//! failure for a file with no baseline entry. A fresh module keeps the addition small and the
//! existing file untouched.
//!
//! Before this fix, `accessor("chunks.length" | "stream.items.length", "kotlin_android", ..)`
//! had no `"kotlin_android"` arm and fell to the `_` default meant for node/wasm/typescript
//! (`.length`). `chunks` is a Kotlin `List<T>` in both kotlin and kotlin_android's generated
//! host-JVM tests (`kotlin_android.rs`'s doc comment: "Generates host-JVM tests..."), and
//! `List<T>` has no `.length` member -- only `.size`. A fixture asserting `chunks.length` or
//! `stream.items.length` for a kotlin_android target therefore emitted Kotlin that referenced a
//! non-existent member, a compile error in the generated e2e file, not merely a dropped or
//! skipped assertion.

use super::StreamingFieldResolver;

/// The load-bearing assertion: kotlin and kotlin_android must render the IDENTICAL expression
/// for `chunks.length`, since both back the virtual field with the same `List<T>` collection.
/// Exact string equality, not `contains`, so a regression back to `.length` (or any other
/// divergence between the two languages) fails this test.
#[test]
fn chunks_length_renders_the_same_size_property_for_kotlin_and_kotlin_android() {
    let kotlin = StreamingFieldResolver::accessor("chunks.length", "kotlin", "chunks").unwrap();
    assert_eq!(kotlin, "chunks.size", "kotlin: {kotlin}");

    let kotlin_android = StreamingFieldResolver::accessor("chunks.length", "kotlin_android", "chunks").unwrap();
    assert_eq!(
        kotlin_android, "chunks.size",
        "kotlin_android must match kotlin's `.size` property access, got: {kotlin_android}"
    );
}

/// The neutral synonym (`stream.items.length`) must resolve the same way as the legacy
/// `chunks.length` spelling for both languages -- the match arm handles both field names
/// together, so a regression in one implies a regression in the other.
#[test]
fn stream_items_length_renders_the_same_size_property_for_kotlin_and_kotlin_android() {
    let kotlin = StreamingFieldResolver::accessor("stream.items.length", "kotlin", "items").unwrap();
    assert_eq!(kotlin, "items.size", "kotlin: {kotlin}");

    let kotlin_android = StreamingFieldResolver::accessor("stream.items.length", "kotlin_android", "items").unwrap();
    assert_eq!(
        kotlin_android, "items.size",
        "kotlin_android must match kotlin's `.size` property access, got: {kotlin_android}"
    );
}

/// Negative control: before the fix, `kotlin_android` silently fell through to the
/// node/wasm/typescript default and rendered `.length`. Pin that this specific wrong string is
/// no longer produced, so a regression back to the missing-arm shape is caught even if the
/// positive assertions above were weakened.
#[test]
fn kotlin_android_no_longer_falls_through_to_the_dot_length_default() {
    let kotlin_android = StreamingFieldResolver::accessor("chunks.length", "kotlin_android", "chunks").unwrap();
    assert_ne!(
        kotlin_android, "chunks.length",
        "kotlin_android must not render `.length` on a Kotlin List, got: {kotlin_android}"
    );
}
