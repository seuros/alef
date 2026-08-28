//! [`super::matches_alef_output`] -- the one predicate both write guards and `alef verify`'s
//! frozen-file report use to decide whether a withheld write had different content to deliver.
//!
//! Cases are chosen around the boundary, because the whole risk in this predicate is the line
//! between "the only difference is provenance alef would add" and "the body genuinely differs",
//! and a boundary is not tested by one example on each side. Every fixture composes the header
//! the way `ensure_generated_header` documents composing it (`{header}\n{content}`, and below a
//! shebang when one is present) rather than calling that function to build the expectation --
//! deriving the fixture from the code under test would make every positive case pass by
//! construction. ~keep

use super::matches_alef_output;
use crate::core::hash::{CommentStyle, header};
use std::path::Path;

fn at(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(name)
}

#[test]
fn identical_bodies_match() {
    assert!(matches_alef_output(
        &at("Widget.java"),
        "final class Widget {}\n",
        "final class Widget {}\n"
    ));
}

/// The ordinary shape of a whole consumer tree that predates alef stamping an extension: the
/// body is already right, and the only thing the refused write would have added is the header.
#[test]
fn a_body_that_differs_only_by_the_header_alef_would_prepend_matches() {
    let existing = "final class Widget {}\n";
    let generated = format!("{}\n{existing}", header(CommentStyle::DoubleSlash));

    assert!(matches_alef_output(&at("Widget.java"), existing, &generated));
}

/// THE CASE a prefix- or suffix-shaped convergence test gets wrong. `ensure_generated_header`
/// puts the header BELOW a shebang, so for every generated shell script the provenance is
/// interior: `generated` is neither `header + existing` nor anything ending in `existing`. A
/// test written that way reports every body-identical generated script stale on every run --
/// a false positive on precisely the file type whose stale version string started this. ~keep
#[test]
fn a_converged_shell_script_matches_even_though_the_header_is_interior() {
    let existing = "#!/usr/bin/env bash\nrun\n";
    let generated = format!("#!/usr/bin/env bash\n{}\nrun\n", header(CommentStyle::Hash));

    assert!(matches_alef_output(&at("install.sh"), existing, &generated));
}

/// A `generated_header: false` path carries no header in its prepared bytes, so a stale one
/// differs in the body alone -- the create-once-seed shape.
#[test]
fn a_seed_whose_body_differs_does_not_match() {
    assert!(!matches_alef_output(
        &at("install.sh"),
        "#!/usr/bin/env bash\nVERSION=1.2.1\n",
        "#!/usr/bin/env bash\nVERSION=1.4.2\n"
    ));
}

#[test]
fn a_differing_body_under_a_header_does_not_match() {
    let generated = format!("{}\nVERSION=1.4.2\n", header(CommentStyle::Hash));

    assert!(!matches_alef_output(&at("install.sh"), "VERSION=1.2.1\n", &generated));
}

/// THE BOUND. Every string ends with `""`, so a suffix-shaped test reports an emptied file as
/// converged and certifies deleted content under a verdict that says nothing changed. ~keep
#[test]
fn an_emptied_file_does_not_match_generated_content() {
    let generated = format!("{}\nfinal class Widget {{}}\n", header(CommentStyle::DoubleSlash));

    assert!(!matches_alef_output(&at("Widget.java"), "", &generated));
}

/// The same bound one step less extreme: a truncated body is a suffix of the generated content,
/// but what was removed is real code rather than provenance.
#[test]
fn a_truncated_body_does_not_match() {
    let generated = format!(
        "{}\nfinal class First {{}}\nfinal class Second {{}}\n",
        header(CommentStyle::DoubleSlash)
    );

    assert!(!matches_alef_output(
        &at("Widget.java"),
        "final class Second {}\n",
        &generated
    ));
}

/// The stamp line is not body. Both writers strip it before comparing, and so must this, or a
/// file would read as drifted purely because its hash was computed at a different moment. ~keep
#[test]
fn the_hash_stamp_line_is_not_treated_as_a_difference() {
    let marker = header(CommentStyle::DoubleSlash);
    let stamped_line = format!("// alef:hash:{}", "ab".repeat(32));
    let first_line = marker.lines().next().expect("header has a first line");
    let rest: String = marker.lines().skip(1).map(|line| format!("{line}\n")).collect();
    let existing = format!("{first_line}\n{stamped_line}\n{rest}\nfinal class Widget {{}}\n");
    let generated = format!("{marker}\nfinal class Widget {{}}\n");

    assert!(matches_alef_output(Path::new("Widget.java"), &existing, &generated));
}
