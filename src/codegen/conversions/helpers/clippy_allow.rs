//! Detects whether generated `From` impl bodies still need the `redundant_closure` and/or
//! `useless_conversion` clippy allows, so generated code carries only the `#[allow(...)]`
//! groups it actually requires instead of a blanket, unconditional pair on every impl.

/// Scan generated field/statement/argument fragments for the textual signatures of the two
/// lints these impls historically suppressed unconditionally:
/// - `clippy::redundant_closure` — an inline closure, recognisable by the `(|` that opens a
///   `.map(|v| ...)` / `.and_then(|v| ...)` style call.
/// - `clippy::useless_conversion` — a `.into()` call or bare `Into::into` function reference.
///
/// Returns `(needs_redundant_closure_allow, needs_useless_conversion_allow)`. This is a
/// conservative over-approximation (e.g. a closure that merely projects a field, like
/// `|v| v.inner`, cannot actually trigger `redundant_closure`, but is still counted as
/// needing the allow) — safe because emitting an unnecessary-but-harmless allow is far
/// cheaper than emitting code that fails `-D warnings`.
pub(crate) fn needs_clippy_allow<'a, I>(fragments: I) -> (bool, bool)
where
    I: IntoIterator<Item = &'a str>,
{
    let mut needs_redundant_closure = false;
    let mut needs_useless_conversion = false;
    for fragment in fragments {
        needs_redundant_closure |= fragment.contains("(|");
        needs_useless_conversion |= fragment.contains(".into()") || fragment.contains("Into::into");
        if needs_redundant_closure && needs_useless_conversion {
            break;
        }
    }
    (needs_redundant_closure, needs_useless_conversion)
}

/// Render the `#[allow(...)]` line (including trailing newline) for a hand-assembled
/// (non-template) generated impl, based on the flags from [`needs_clippy_allow`]. Returns an
/// empty string when neither lint can fire, so the impl carries no allow at all.
pub(crate) fn clippy_allow_attr_line(needs_redundant_closure: bool, needs_useless_conversion: bool) -> String {
    match (needs_redundant_closure, needs_useless_conversion) {
        (true, true) => "#[allow(clippy::redundant_closure, clippy::useless_conversion)]\n".to_string(),
        (true, false) => "#[allow(clippy::redundant_closure)]\n".to_string(),
        (false, true) => "#[allow(clippy::useless_conversion)]\n".to_string(),
        (false, false) => String::new(),
    }
}
