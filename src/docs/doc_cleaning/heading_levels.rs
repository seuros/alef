//! Re-levelling of markdown headings lifted out of rustdoc comments.
//!
//! A doc comment is written as a standalone document, so its headings start at whatever level the
//! author chose. Alef splices that text underneath a heading it generated itself (a function's
//! `####`, a type's `###`), so the doc's own heading levels have to be rewritten to sit below that
//! parent. Getting this wrong produces markdown that rumdl rejects: a heading above its own parent
//! (MD001 increment violations, and MD025 when it lands on `#`).

/// Returns the levels of every ATX heading in `doc`, in document order, skipping fenced code.
fn heading_levels(doc: &str) -> Vec<usize> {
    let mut levels = Vec::new();
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || !line.starts_with('#') {
            continue;
        }
        let level = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) {
            levels.push(level);
        }
    }

    levels
}

/// Check if a markdown document has monotonic heading increments (no skips of >1 level).
///
/// Returns `Ok(())` if all headings increment by at most 1 level, or an error message
/// describing the first violation found.
#[cfg(test)]
pub(crate) fn check_monotonic_headings(doc: &str) -> Result<(), String> {
    let mut previous_level: Option<usize> = None;
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || !line.starts_with('#') {
            continue;
        }

        let heading_level = line.chars().take_while(|&c| c == '#').count();
        if heading_level == 0 || heading_level > 6 {
            continue;
        }

        if let Some(prev) = previous_level {
            let increment = heading_level.saturating_sub(prev);
            if increment > 1 {
                let heading_text = line.trim_start_matches('#').trim();
                return Err(format!(
                    "Heading increment violation: H{} → H{} (skip of {})\nHeading: {}",
                    prev, heading_level, increment, heading_text
                ));
            }
        }

        previous_level = Some(heading_level);
    }

    Ok(())
}

/// Rewrite every heading in `doc` according to `remap`, leaving fenced code untouched.
fn rewrite_heading_levels(doc: &str, remap: impl Fn(usize) -> usize) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut in_code_block = false;

    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code_block || !line.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let level = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) {
            out.push_str(&"#".repeat(remap(level)));
            out.push_str(&line[level..]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    out.trim_end().to_string()
}

/// Demote all markdown headings by a given number of levels.
///
/// For example, with `levels=2`, all `#` become `###`, `##` become `####`, etc.
/// Headings inside code blocks are not modified.
pub(crate) fn demote_headings(doc: &str, levels: usize) -> String {
    if levels == 0 || doc.is_empty() {
        return doc.to_string();
    }
    rewrite_heading_levels(doc, |level| std::cmp::min(level + levels, 6))
}

/// Re-level `doc`'s headings so they nest under a parent heading at `target_level - 1`.
///
/// The doc's distinct heading levels are mapped, shallowest first, onto consecutive levels
/// starting at `target_level` (clamped at H6). So a doc using `#` and `###` placed under an H4
/// section comes out as H5 and H6.
///
/// Renumbering rather than shifting by a fixed amount is what makes the result valid markdown in
/// both directions, and neither of the two obvious shift strategies manages that:
///
/// - Anchoring the *first* heading at `target_level` leaves any later, shallower heading above the
///   section it belongs to — a rustdoc `# Note` after a `##### Detail` stayed a bare H1 inside a
///   page rooted at `##`, tripping MD025. Every heading alef does not rewrite to a bold label
///   reaches here, so this is the general case, not a property of particular section names.
/// - Anchoring the *shallowest* heading at `target_level` fixes that but preserves the source's own
///   level gaps, so a doc jumping `#` → `#####` still emits an MD001 increment violation.
///
/// Mapping onto consecutive levels preserves the relative hierarchy — which heading is nested
/// under which — while discarding the arbitrary gaps between the levels the author happened to
/// pick. Docs whose headings are all already at or below `target_level` are left alone, so this
/// never promotes a heading. ~keep
pub(crate) fn demote_headings_to_start_at(doc: &str, target_level: usize) -> String {
    let target_level = target_level.clamp(1, 6);

    let mut distinct_levels = heading_levels(doc);
    distinct_levels.sort_unstable();
    distinct_levels.dedup();

    let Some(&shallowest) = distinct_levels.first() else {
        return doc.to_string();
    };
    if shallowest >= target_level {
        return doc.to_string();
    }

    rewrite_heading_levels(doc, |level| {
        let rank = distinct_levels
            .iter()
            .position(|&candidate| candidate == level)
            .unwrap_or(0);
        std::cmp::min(target_level + rank, 6)
    })
}
