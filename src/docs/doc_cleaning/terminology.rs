//! Boundary-aware substring replacement for Rust-vocabulary-to-host-vocabulary rewrites.
//!
//! Most terminology rewrites in `doc_cleaning` target self-delimiting syntax (`Vec<String>`,
//! `` `Self::tables` ``) where a plain `str::replace` is already boundary-safe because the
//! surrounding `<`, backtick, or punctuation can't be part of a longer identifier. A handful of
//! targets are bare prose words instead (`vec`), and those are prefixes of unrelated longer
//! words (`vector`) that a plain substring replace would silently corrupt.

/// Returns `true` for characters that make up a "word" for the purposes of
/// [`replace_whole_word`]: letters, digits, and underscore.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replace every occurrence of `from` in `text` with `to`, but only when the match is not
/// glued to a longer word on either side.
///
/// A plain `str::replace("empty vec", "empty list")` corrupts "empty vector" into "empty
/// listtor": the naive replace matches the "empty vec" prefix and leaves the "tor" tail of
/// "vector" dangling after the substitution. This checks the character immediately before
/// and after each match and only substitutes when neither neighbor is a word character (or
/// the match sits at the start/end of the string), so a short target term like "vec" is never
/// rewritten when it is really the head of a longer word such as "vector". ~keep
pub(super) fn replace_whole_word(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(from) {
        let match_end = index + from.len();
        let before_ok = !rest[..index].chars().next_back().is_some_and(is_word_char);
        let after_ok = !rest[match_end..].chars().next().is_some_and(is_word_char);
        result.push_str(&rest[..index]);
        if before_ok && after_ok {
            result.push_str(to);
        } else {
            result.push_str(&rest[index..match_end]);
        }
        rest = &rest[match_end..];
    }
    result.push_str(rest);
    result
}
