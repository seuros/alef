use syn;

/// Extract doc comments from attributes.
///
/// Output is post-processed by [`normalize_rustdoc`] so binding emitters
/// never see rustdoc-hidden setup lines (`# tokio_test::block_on(async {`)
/// or unresolved intra-doc-link syntax (`[\`crate::Foo\`]`).
pub(crate) fn extract_doc_comments(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let syn::Meta::NameValue(meta) = &attr.meta
            && let syn::Expr::Lit(expr_lit) = &meta.value
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
        {
            let val = lit_str.value();
            let trimmed = val.strip_prefix(' ').unwrap_or(&val);
            lines.push(trimmed.to_string());
        }
    }
    let raw = lines.join("\n");
    normalize_rustdoc(&raw)
}

/// Pre-process raw rustdoc so binding emitters can treat it as plain prose.
///
/// 1. Inside ```rust / ```rust,no_run fences, drops lines starting with `# `
///    (rustdoc's "hidden" syntax used to inject test scaffolding such as
///    `# tokio_test::block_on(async {` or `# Ok::<(), Error>(())`).
/// 2. Converts intra-doc-link syntax `` [`crate::Foo`] `` and
///    `` [`super::Bar`] `` to plain `` `Foo` `` / `` `Bar` `` so unresolved
///    paths don't leak into JS / Java / dart output.
///
/// Any other content is preserved verbatim (existing per-host renderers
/// continue to translate `# Errors` / `# Returns` / etc).
pub fn normalize_rustdoc(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    let mut filtered = String::with_capacity(raw.len());
    let mut in_rust_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_rust_fence {
                in_rust_fence = false;
            } else {
                let lang = rest.split(',').next().unwrap_or("").trim();
                if lang.is_empty() || lang.eq_ignore_ascii_case("rust") {
                    in_rust_fence = true;
                }
            }
            filtered.push_str(line);
            filtered.push('\n');
            continue;
        }
        if in_rust_fence {
            let after_hash = trimmed.strip_prefix('#');
            if let Some(suffix) = after_hash
                && (suffix.is_empty() || suffix.starts_with(' '))
            {
                continue;
            }
        }
        filtered.push_str(&strip_internal_doc_markers(line));
        filtered.push('\n');
    }

    let mut out = String::with_capacity(filtered.len());
    let chars: Vec<char> = filtered.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '`' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < chars.len() {
                if chars[j] == '`' && chars[j + 1] == ']' {
                    break;
                }
                j += 1;
            }
            if j + 1 < chars.len() && chars[j] == '`' && chars[j + 1] == ']' {
                let inner: String = chars[start..j].iter().collect();
                let stripped = inner
                    .strip_prefix("crate::")
                    .or_else(|| inner.strip_prefix("super::"))
                    .or_else(|| inner.strip_prefix("self::"));
                if let Some(rest) = stripped {
                    let last = rest.rsplit("::").next().unwrap_or(rest);
                    out.push('`');
                    out.push_str(last);
                    out.push('`');
                    i = j + 2;
                    if i < chars.len() && chars[i] == '(' {
                        let mut depth = 1;
                        i += 1;
                        while i < chars.len() && depth > 0 {
                            match chars[i] {
                                '(' => depth += 1,
                                ')' => depth -= 1,
                                _ => {}
                            }
                            i += 1;
                        }
                    }
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

const MARKER: &str = "~keep";

/// Punctuation that belongs to the marker rather than to the sentence around it: `~keep:`
/// and `~keep,` introduce the rationale that follows them, so the punctuation is part of the
/// token being removed. Removing only `~keep` welded the orphaned punctuation onto the end of
/// the preceding sentence — the dangling `.:` that reached five generated bindings.
const MARKER_PUNCTUATION: [char; 3] = [':', ',', ';'];

/// Delimiter pairs a marker may be wrapped in. `(~keep)` and `` `~keep` `` own their
/// brackets; dropping only the token leaves an empty `()` or an empty code span.
const MARKER_DELIMITERS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('`', '`')];

fn strip_internal_doc_markers(line: &str) -> String {
    let mut output = line.to_string();
    let mut search_from = 0;
    while let Some(found) = output[search_from..].find(MARKER) {
        let mut start = search_from + found;
        let mut end = start + MARKER.len();

        if let Some(punctuation) = output[end..].chars().next()
            && MARKER_PUNCTUATION.contains(&punctuation)
        {
            end += punctuation.len_utf8();
        }

        for (open, close) in MARKER_DELIMITERS {
            if output[..start].ends_with(open) && output[end..].starts_with(close) {
                start -= open.len_utf8();
                end += close.len_utf8();
                break;
            }
        }

        let space_before = start == 0 || output[..start].ends_with(char::is_whitespace);
        let space_after = end == output.len() || output[end..].starts_with(char::is_whitespace);

        // Exactly one separator must survive. A removal that runs to the end of the line has
        // nothing left to separate, so the run in front of it goes too; anywhere else the run
        // behind it goes and the one in front keeps the surviving words apart. When prose
        // punctuation abuts the marker on the right (`(see the note ~keep)`), taking the run in
        // front is what avoids leaving a space before that punctuation. ~keep
        if space_before && space_after {
            if end == output.len() {
                start -= trailing_whitespace_len(&output[..start]);
            } else {
                end += leading_whitespace_len(&output[end..]);
            }
        } else if space_before {
            start -= trailing_whitespace_len(&output[..start]);
        }

        output.replace_range(start..end, "");
        search_from = start;
    }
    output
}

fn trailing_whitespace_len(text: &str) -> usize {
    text.len() - text.trim_end().len()
}

fn leading_whitespace_len(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::normalize_rustdoc;

    #[test]
    fn normalize_rustdoc_removes_internal_keep_tokens_without_dropping_prose() {
        let normalized = normalize_rustdoc(
            "~keep No deny-unknown-fields because this type is shared.\nThe field remains optional. ~keep",
        );

        assert_eq!(
            normalized,
            "No deny-unknown-fields because this type is shared.\nThe field remains optional."
        );
        assert!(!normalized.contains("~keep"));
    }

    /// Every punctuation variant of the marker found in the polyrepo, plus the forms a
    /// future author is likely to reach for. The surrounding punctuation is asserted
    /// exactly: the reported defect was `~keep:` leaving the colon welded onto the previous
    /// sentence (`... note below.:`), and the symmetric over-correction — eating the
    /// sentence's own full stop — is just as wrong.
    #[test]
    fn normalize_rustdoc_strips_every_keep_marker_variant_leaving_readable_prose() {
        let cases = [
            ("Recovered from a parse error. ~keep", "Recovered from a parse error."),
            (
                "~keep Explains why the fallback is safe.",
                "Explains why the fallback is safe.",
            ),
            (
                "The pool ~keep is rebuilt per request.",
                "The pool is rebuilt per request.",
            ),
            (
                "Drained after every page. ~keep: that drain observes one thread.",
                "Drained after every page. that drain observes one thread.",
            ),
            ("See the note below. ~keep:", "See the note below."),
            ("~keep: mirrors the bench tolerance.", "mirrors the bench tolerance."),
            (
                "Mirrors `image.rs`. ~keep, same rationale as above.",
                "Mirrors `image.rs`. same rationale as above.",
            ),
            (
                "Guarded by a flag. ~keep; the flag is compile-time.",
                "Guarded by a flag. the flag is compile-time.",
            ),
            (
                "Never smuggled into human-facing text. (~keep)",
                "Never smuggled into human-facing text.",
            ),
            (
                "Applies to both paths (see the note ~keep).",
                "Applies to both paths (see the note).",
            ),
            (
                "Tracked separately [~keep] for the async path.",
                "Tracked separately for the async path.",
            ),
            (
                "See the `~keep` note in `prepare_session`.",
                "See the note in `prepare_session`.",
            ),
            ("Two markers ~keep on one line. ~keep", "Two markers on one line."),
            ("~keep", ""),
            ("~keep:", ""),
            ("(~keep)", ""),
        ];

        for (raw, expected) in cases {
            let normalized = normalize_rustdoc(raw);
            assert_eq!(normalized, expected, "input: {raw:?}");
            assert!(!normalized.contains("~keep"), "marker survived in {normalized:?}");
            assert!(!normalized.contains(".:"), "dangling punctuation in {normalized:?}");
        }
    }

    /// The exact doc comment that produced the dangling `.:` in the generated bindings.
    #[test]
    fn normalize_rustdoc_handles_the_colon_suffixed_marker_mid_paragraph() {
        let normalized = normalize_rustdoc(
            "extraction that renders at least one page picks up any captured\n\
             glyph-drop warnings for free. ~keep: that drain only ever observes\n\
             warnings from render calls that happened on the same OS thread.",
        );

        assert_eq!(
            normalized,
            "extraction that renders at least one page picks up any captured\n\
             glyph-drop warnings for free. that drain only ever observes\n\
             warnings from render calls that happened on the same OS thread."
        );
    }
}
