//! Bounding the subprocess output that becomes a human-facing diagnostic.
//!
//! A toolchain that wanders into a pathological state does not emit one long message; it emits the
//! same short message thousands of times. A Gradle build whose SDK discovery walked a shared
//! package prefix repeated a corrupted-metadata warning for every unrelated file it found, and the
//! full stream reached the operator as a single `before command failed: ...` string. The output is
//! still captured in full for the callers that *parse* it -- classpath entries, dependency-error
//! matching -- and bounded only where it turns into prose a person reads.
//!
//! Truncation is never silent: whatever is dropped is reported inline, in the text itself, because
//! a reader who scrolls to the end of a diagnostic has no other way to learn that an end was cut
//! off. ~keep

/// Lines kept from the start of an over-long diagnostic. The head is where a compiler puts the
/// first real error, and where [`crate::snippets::runner::unresolved_dependency_message`] puts the
/// remediation prefix that `output::unresolved_dependency_rollup` matches on. ~keep
pub(crate) const DIAGNOSTIC_HEAD_LINES: usize = 40;

/// Lines kept from the end of an over-long diagnostic. The tail is where a build system puts its
/// failure summary and the task that failed. ~keep
pub(crate) const DIAGNOSTIC_TAIL_LINES: usize = 40;

/// The character ceiling applied after line bounding, so a tool that emits a megabyte on a single
/// line -- no newline for the line bound to act on -- is still bounded. ~keep
pub(crate) const DIAGNOSTIC_MAX_CHARS: usize = 32 * 1024;

/// A diagnostic reduced to a reportable size, and exactly how much was dropped to get there.
pub(crate) struct BoundedDiagnostic {
    pub(crate) text: String,
    pub(crate) dropped_lines: usize,
    pub(crate) dropped_chars: usize,
}

fn omitted_lines_marker(dropped_lines: usize) -> String {
    format!(
        "[alef: {dropped_lines} more lines omitted from this diagnostic; \
         kept the first {DIAGNOSTIC_HEAD_LINES} and the last {DIAGNOSTIC_TAIL_LINES}]"
    )
}

fn omitted_chars_marker(dropped_chars: usize) -> String {
    format!("[alef: {dropped_chars} more characters omitted from this diagnostic]")
}

/// Reduces `output` to at most [`DIAGNOSTIC_HEAD_LINES`] + [`DIAGNOSTIC_TAIL_LINES`] lines and
/// [`DIAGNOSTIC_MAX_CHARS`] characters, reporting the amount dropped both inline and in the
/// returned counts.
pub(crate) fn bound_diagnostic(output: &str) -> BoundedDiagnostic {
    let (text, dropped_lines) = bound_lines(output);
    let (text, dropped_chars) = bound_chars(text);
    BoundedDiagnostic {
        text,
        dropped_lines,
        dropped_chars,
    }
}

/// Bounds `output` and warns when anything was dropped, for callers that only need the text.
pub(crate) fn bounded_text(output: &str) -> String {
    let bounded = bound_diagnostic(output);
    if bounded.dropped_lines > 0 || bounded.dropped_chars > 0 {
        tracing::warn!(
            dropped_lines = bounded.dropped_lines,
            dropped_chars = bounded.dropped_chars,
            "truncated a command's diagnostic output; the omitted amount is reported inline"
        );
    }
    bounded.text
}

/// The marker costs a line of its own, so replacing a single dropped line with it would report a
/// truncation that saved nothing. Bounding therefore starts at two lines beyond the budget. ~keep
fn bound_lines(output: &str) -> (String, usize) {
    let lines: Vec<&str> = output.lines().collect();
    let budget = DIAGNOSTIC_HEAD_LINES + DIAGNOSTIC_TAIL_LINES;
    if lines.len() <= budget + 1 {
        return (output.to_owned(), 0);
    }
    let dropped = lines.len() - budget;
    let head = lines[..DIAGNOSTIC_HEAD_LINES].join("\n");
    let tail = lines[lines.len() - DIAGNOSTIC_TAIL_LINES..].join("\n");
    (format!("{head}\n{}\n{tail}", omitted_lines_marker(dropped)), dropped)
}

/// Truncation is by character, not byte, so a diagnostic quoting non-ASCII source cannot panic on
/// a split boundary. ~keep
fn bound_chars(text: String) -> (String, usize) {
    let total = text.chars().count();
    if total <= DIAGNOSTIC_MAX_CHARS {
        return (text, 0);
    }
    let dropped = total - DIAGNOSTIC_MAX_CHARS;
    let end = text
        .char_indices()
        .nth(DIAGNOSTIC_MAX_CHARS)
        .map_or(text.len(), |(index, _)| index);
    (format!("{}\n{}", &text[..end], omitted_chars_marker(dropped)), dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeated_warning_lines(count: usize) -> String {
        (0..count)
            .map(|index| format!("warning: corrupted metadata in entry {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_diagnostic_within_the_line_budget_is_returned_unchanged() {
        let output = repeated_warning_lines(DIAGNOSTIC_HEAD_LINES + DIAGNOSTIC_TAIL_LINES + 1);

        let bounded = bound_diagnostic(&output);

        assert_eq!(bounded.dropped_lines, 0);
        assert_eq!(bounded.dropped_chars, 0);
        assert_eq!(bounded.text, output);
    }

    /// The count is the whole point: a reader who sees a truncation marker without a number cannot
    /// tell a build that repeated one warning twice from one that repeated it four thousand times.
    #[test]
    fn truncation_reports_the_exact_number_of_dropped_lines() {
        let total = 4_000;
        let output = repeated_warning_lines(total);
        let expected_dropped = total - DIAGNOSTIC_HEAD_LINES - DIAGNOSTIC_TAIL_LINES;

        let bounded = bound_diagnostic(&output);

        assert_eq!(bounded.dropped_lines, expected_dropped);
        assert_eq!(
            bounded.text.lines().count(),
            DIAGNOSTIC_HEAD_LINES + DIAGNOSTIC_TAIL_LINES + 1,
            "the bounded text is the head, the tail, and one marker line"
        );
        assert!(
            bounded.text.contains(&format!("{expected_dropped} more lines omitted")),
            "the dropped count must be visible in the text itself: {}",
            bounded.text
        );
    }

    #[test]
    fn the_first_and_last_lines_survive_truncation() {
        let output = repeated_warning_lines(4_000);

        let bounded = bound_diagnostic(&output);
        let lines: Vec<&str> = bounded.text.lines().collect();

        assert_eq!(lines[0], "warning: corrupted metadata in entry 0");
        assert_eq!(
            lines[DIAGNOSTIC_HEAD_LINES - 1],
            format!("warning: corrupted metadata in entry {}", DIAGNOSTIC_HEAD_LINES - 1)
        );
        assert_eq!(lines[lines.len() - 1], "warning: corrupted metadata in entry 3999");
    }

    /// A build that emits a megabyte without a newline gives the line bound nothing to act on, so
    /// the character ceiling is what stops it -- and it reports its own count separately. ~keep
    #[test]
    fn a_single_enormous_line_is_bounded_by_the_character_ceiling() {
        let output = "x".repeat(DIAGNOSTIC_MAX_CHARS + 5_000);

        let bounded = bound_diagnostic(&output);

        assert_eq!(bounded.dropped_lines, 0);
        assert_eq!(bounded.dropped_chars, 5_000);
        assert!(
            bounded.text.contains("5000 more characters omitted"),
            "the dropped character count must be visible in the text itself"
        );
    }

    #[test]
    fn truncation_never_splits_a_multi_byte_character() {
        let output = "é".repeat(DIAGNOSTIC_MAX_CHARS + 12);

        let bounded = bound_diagnostic(&output);

        assert_eq!(bounded.dropped_chars, 12);
        assert_eq!(
            bounded.text.chars().filter(|character| *character == 'é').count(),
            DIAGNOSTIC_MAX_CHARS
        );
    }
}
