//! One authority for turning an arbitrary Rust-side string into Elixir source text.
//!
//! Every value that reaches generated Elixir as a literal — a `#[serde(rename = "...")]` wire
//! name, a `#[serde(tag = "...")]` discriminator key, a field's wire spelling — is
//! attacker-influenced in exactly the sense that matters here: it is copied verbatim from a
//! dependency's source into a file that is then COMPILED. Elixir's double-quoted literals
//! interpolate, so `#{...}` inside one is not a syntax problem, it is arbitrary code executed at
//! the generated module's compile time. Confirmed against Elixir 1.20.4: a rename of
//! `a#{File.write!("...")}b` compiles, writes the file, and yields the atom `:aokb` — the
//! interpolation's return value spliced in — rather than the literal name. `#` is therefore
//! escaped here alongside `\` and `"`, and both the unescaped and escaped forms PARSE, so a
//! `Code.string_to_quoted` check cannot tell them apart; only evaluating the module can. ~keep

/// Escape `value` for embedding inside a double-quoted Elixir string literal or a quoted atom.
///
/// The two literal forms share one escape grammar in Elixir (`:"..."` is a quoted atom whose
/// body is lexed as a string), so one function serves both and they cannot drift apart.
pub(crate) fn escape_elixir_string_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '#' => escaped.push_str("\\#"),
            control if control.is_control() => {
                escaped.push_str(&format!("\\u{{{:x}}}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// Render `value` as the body of an Elixir atom — the text that follows a `:` — returning either
/// a bare identifier (`foo`) or a quoted, escaped literal (`"og:image"`).
///
/// **Quote, do not reject.** `:123`, `:1foo`, `:` and `:café` are `SyntaxError`s in Elixir, and a
/// single one poisons the whole generated module, so something has to give. The alternative to
/// quoting is refusing the name — dropping the variant or failing generation — and that trades a
/// compile error for a silent capability gap on a program that is perfectly legal Rust. The
/// quoted forms are not a workaround: `:"123"`, `:"1foo"`, `:""` and `:"café"` are ordinary
/// Elixir atoms that `Atom.to_string/1` round-trips to the exact original bytes (verified against
/// Elixir 1.20.4 over the whole invalid space — empty, leading digit, non-ASCII, whitespace,
/// quote, backslash, `#{`, and control characters). So every name survives, and the escaping
/// above is what makes surviving safe. ~keep
pub(crate) fn elixir_atom_body(value: &str) -> String {
    if is_bare_atom_identifier(value) {
        value.to_owned()
    } else {
        format!("\"{}\"", escape_elixir_string_literal(value))
    }
}

/// Whether `value` can follow a `:` unquoted: `[A-Za-z_][A-Za-z0-9_]*` with at most one trailing
/// `?` or `!`. Deliberately ASCII-only — Elixir accepts some non-ASCII identifiers, but the
/// quoted form is always correct and never needs a Unicode identifier table to stay right.
fn is_bare_atom_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    loop {
        let Some(character) = characters.next() else {
            return true;
        };
        if character == '?' || character == '!' {
            return characters.next().is_none();
        }
        if !character.is_ascii_alphanumeric() && character != '_' {
            return false;
        }
    }
}

#[cfg(test)]
mod tests;
