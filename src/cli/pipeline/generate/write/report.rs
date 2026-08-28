//! What a write pass did NOT do, and why -- the report both generation writers return.
//!
//! Split out of `write.rs` rather than added to it: that file was within a dozen lines of this
//! repository's 1,000-line cap, and "what was withheld, and how it is described to an operator"
//! is a self-contained concern from "how bytes reach disk". ~keep

use crate::core::hash;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Default)]
pub struct WriteReport {
    pub expected_paths: std::collections::HashSet<std::path::PathBuf>,
    pub changed_paths: std::collections::HashSet<std::path::PathBuf>,
    /// Paths the ownership guard declined to write.
    ///
    /// Recorded rather than only logged, because a refusal is otherwise invisible to every
    /// downstream signal: the guard `continue`s before the path reaches `expected_paths`, so
    /// orphan sweeps, freshness checks and the changed count all behave as though alef never
    /// intended to write the file. A permanently frozen file is then indistinguishable from
    /// one alef simply does not manage.
    ///
    /// Unlike an ordinary skip the condition never clears on its own, and the remedy —
    /// `alef adopt` — is a human action. A human cannot act on a number nobody reports, so
    /// this has to be visible rather than inferred from what did not change.
    ///
    /// A `BTreeSet` rather than a `Vec`: the same path can be refused by more than one guard
    /// site in a run, and the report is read by a person, so it must not repeat itself or
    /// reorder between runs. ~keep
    pub refused_paths: std::collections::BTreeSet<std::path::PathBuf>,
    /// The subset of [`Self::refused_paths`] whose withheld content actually DIFFERS from
    /// what is on disk, once the provenance header alef itself would add is discounted
    /// ([`matches_alef_output`]).
    ///
    /// THE DEFECT this closes: the refusal tally reported how many files were withheld and
    /// nothing about what was withheld, so a refusal that would have changed a real byte and a
    /// refusal that would only have stamped a header on already-correct content produced the
    /// same number and the same sentence. Both are common -- a body-identical, header-missing
    /// file is refused by the guard exactly as a stale one is (the writers' unchanged check
    /// compares the header-stamped bytes) -- and only one of them is a live defect. The
    /// measured consequence: a generated test-app installer bakes the release version into its
    /// own bytes, the guard refused the rewrite on every run, and three separate consumer
    /// repositories shipped an installer pinned to a stale release for weeks, because from
    /// outside a refusal on version-derived content reads exactly like a file that is already
    /// up to date.
    ///
    /// Populated at the guard, not recomputed later: both writers hold the disk bytes and the
    /// prepared output in the same scope at the moment they decide, so this costs one string
    /// comparison and no extra I/O. A caller that inserted into [`Self::refused_paths`]
    /// directly would silently reintroduce the undifferentiated tally, which is why both
    /// guards go through [`Self::refuse_text`]/[`Self::refuse_drifted`] instead. ~keep
    pub refused_drifted_paths: std::collections::BTreeSet<std::path::PathBuf>,
    /// Paths this run left alone because `[workspace.ownership] user_owned`
    /// ([`crate::core::config::OwnershipConfig`]) declares them owned by the consuming
    /// repository rather than by alef.
    ///
    /// A separate set from [`Self::refused_paths`] because it is a separate FACT, not a
    /// softer wording of the same one. A refusal is alef reporting that it wanted to write a
    /// file and could not prove it may -- an unresolved condition with a human remedy. A
    /// declared skip is alef reporting that it was told not to, by a committed line of config
    /// someone can read. Folding the two would leave the operator with a failure tally that
    /// only goes down by deleting the declaration, which is exactly the stable bad state the
    /// declaration exists to end.
    ///
    /// Reported rather than silent for the same reason `refused_paths` is: a run that wrote
    /// nothing because 17 paths were declared must say so. See [`report_user_owned_skips`]. ~keep
    pub user_owned_paths: std::collections::BTreeSet<std::path::PathBuf>,
}

impl WriteReport {
    pub fn changed_count(&self) -> usize {
        self.changed_paths.len()
    }

    pub fn expected_count(&self) -> usize {
        self.expected_paths.len()
    }

    pub fn refused_count(&self) -> usize {
        self.refused_paths.len()
    }

    /// How many refused writes had different content to deliver -- see
    /// [`Self::refused_drifted_paths`].
    pub fn refused_drifted_count(&self) -> usize {
        self.refused_drifted_paths.len()
    }

    /// How many paths this report left alone on the strength of `[workspace.ownership]
    /// user_owned`.
    pub fn user_owned_count(&self) -> usize {
        self.user_owned_paths.len()
    }

    /// Record a refused write whose withheld content is known to differ from the bytes on
    /// disk.
    ///
    /// For the branches where no comparison is needed to know the answer: a binary target the
    /// guard reached only after an exact byte comparison already failed, and a text target
    /// whose existing bytes are not valid UTF-8 at all (alef's prepared output always is, so
    /// they cannot be equal, and the file cannot be compared to say anything narrower). ~keep
    pub fn refuse_drifted(&mut self, path: &Path) {
        self.refused_paths.insert(path.to_path_buf());
        self.refused_drifted_paths.insert(path.to_path_buf());
    }

    /// Record a refused text write, classifying it against the bytes already on disk.
    ///
    /// `existing` is `None` when the file could not be read as text, which classifies as
    /// drifted for the reason [`Self::refuse_drifted`]'s doc gives. ~keep
    pub fn refuse_text(&mut self, path: &Path, existing: Option<&str>, generated: &str) {
        match existing {
            Some(existing) if matches_alef_output(path, existing, generated) => {
                self.refused_paths.insert(path.to_path_buf());
            }
            _ => self.refuse_drifted(path),
        }
    }

    /// Fold another phase's refusals into this report.
    ///
    /// A run writes through several independent phases — bindings, service API, type stubs,
    /// public API, scaffolding — each returning its own report. The refusal summary is a
    /// run-level fact addressed to an operator, so reporting per phase understates it: the
    /// reader works the list they were shown and is left with the refusals from every other
    /// phase, unlisted and with no remaining signal that they exist. Only `refused_paths`
    /// merges; the changed and expected sets stay per-phase because their counts are reported
    /// per phase and summing them would double-count a path two phases both intended.
    ///
    /// Folds BOTH not-written sets -- refusals and declared user-owned skips -- through this
    /// one call rather than adding a second method beside it, so the ~13 `absorb_unwritten`
    /// call sites cannot end up folding one set and dropping the other. That is not
    /// hypothetical here: `alef all` already shipped a bug where a count-only wrapper
    /// discarded `refused_paths` for a whole class of writes, and the run reported success
    /// while the guard had silently refused thousands. The drifted subset folds here too, for
    /// the same reason and in the same call: a per-phase split of the refusal tally would let
    /// one phase's stale withheld content vanish from the run-level report. ~keep
    pub fn absorb_unwritten(&mut self, other: &WriteReport) {
        self.refused_paths.extend(other.refused_paths.iter().cloned());
        self.refused_drifted_paths
            .extend(other.refused_drifted_paths.iter().cloned());
        self.user_owned_paths.extend(other.user_owned_paths.iter().cloned());
    }
}

/// Whether `existing` already IS what alef would put at this path, discounting only the
/// provenance alef itself would prepend.
///
/// Two disjuncts, and they are not two rules -- they are the two shapes the SAME rule takes,
/// because the header is conditional on the emitting `GeneratedFile`. The first is the writers'
/// own unchanged-predicate verbatim (`strip_hash_line` on both sides, as in
/// `super::write_files_report` and `super::super::scaffold::write_scaffold_files_report`), and
/// it is the whole answer for a `generated_header: false` path, whose prepared bytes carry no
/// header for the disk copy to be missing. The second covers `generated_header: true`: a file
/// that reaches a refusal is by definition unmarked on disk, so its bytes can never equal
/// header-bearing output, and a bare body comparison would answer "differs" for every one of
/// them -- the distinction would degenerate into a constant.
///
/// The second disjunct asks [`super::ensure_generated_header`] itself what the header would be
/// and where it would go, rather than reasoning about prefixes. That is not a stylistic choice.
/// The header is a three-line block, only the first line of which
/// [`hash::is_provenance_only_prefix`] recognises, and `ensure_generated_header` inserts it
/// BELOW a shebang, below a `<?php` tag and below an XML declaration -- so no prefix or suffix
/// relation holds for a generated shell script, a PHP source file or a `.csproj`, and any
/// hand-rolled approximation reports those body-identical files stale on every run. Asking the
/// writer's own function is also what keeps this answer and the writer's answer from drifting.
/// It is a no-op on content that already carries a marker and on formats that cannot hold one,
/// so both of those cases fall through to the exact comparison, which is the right answer for
/// them.
///
/// Comparison is exact on both sides, never a suffix test: every string ends with `""`, so a
/// suffix test reports an emptied file as converged and silently certifies deleted content.
///
/// Deliberately NOT [`crate::cli::commands::adopt::classify`], which answers a neighbouring
/// question and answers it differently on purpose: it compares the bytes ADOPTION would stamp,
/// so it applies the header unconditionally and calls every create-once seed on a markable
/// extension drifted, whatever its content. That is right for `alef adopt`, whose subject is
/// the post-adoption file; it is the wrong answer to the question here, which is only whether
/// the refused write had different content to deliver. ~keep
pub(crate) fn matches_alef_output(path: &Path, existing: &str, generated: &str) -> bool {
    let existing_body = hash::strip_hash_line(existing);
    let generated_body = hash::strip_hash_line(generated);
    existing_body == generated_body
        || hash::strip_hash_line(&super::ensure_generated_header(path, &existing_body)) == generated_body
}

/// Surface every write the ownership guard declined, naming the remedy.
///
/// The guard is self-perpetuating by construction: it refuses because the file carries no
/// marker, and the marker can only arrive by writing the file. No later run breaks that
/// cycle, so a per-file `warn!` mid-run understates the situation — the condition is
/// permanent rather than transient, and only an operator can clear it. One consolidated
/// block naming the fix is the difference between a log line and an actionable report. ~keep
///
/// States the DRIFTED count beside the total, and marks each drifted path in the list. A
/// refusal that would only have stamped a header on already-correct content is benign and
/// common; a refusal that withheld different bytes means the file on disk is stale for as long
/// as it stays frozen, and no rerun changes that. Reporting both as one number is what made a
/// stale, version-bearing generated file indistinguishable from an up-to-date one -- see
/// [`WriteReport::refused_drifted_paths`] for the measured incident. ~keep
pub fn report_refused_writes(report: &WriteReport) {
    if report.refused_paths.is_empty() {
        return;
    }
    let mut paths: Vec<&std::path::PathBuf> = report.refused_paths.iter().collect();
    paths.sort();
    warn!(
        "{} file(s) were NOT written, {} of them holding content that DIFFERS from what alef \
         would now generate: each already exists, carries no alef provenance marker, and \
         alef has no durable record of owning it. This will not resolve on its own — the marker can \
         only be written by writing the file, which is exactly what the guard declines. The \
         differing ones are stale for as long as they stay frozen, and a file whose content is \
         derived from the release version is the case that bites, because a stale copy stays \
         plausible. Review the \
         diff for each and adopt the ones alef should own with `alef adopt <path>`. At migration \
         scale, `alef adopt <glob>` previews the whole set and `alef adopt <glob> --converged-only \
         --write` clears every file that already matches generated output, leaving the drifted \
         ones for you to read one at a time. If these are \
         formats that cannot carry a marker (package.json, *.jar) and this is a fresh clone or a CI \
         checkout, check whether .alef-ownership.toml was committed — that file is where their \
         ownership is recorded. Do NOT hand-add the marker line: a refusal can be protecting a \
         deliberate hand-edit, and stamping it blind re-enables exactly the clobbering the guard \
         exists to prevent.",
        paths.len(),
        report.refused_drifted_paths.len()
    );
    for path in paths {
        if report.refused_drifted_paths.contains(path) {
            warn!(
                "  not written, content DIFFERS (stale until adopted or deleted): {}",
                path.display()
            );
        } else {
            warn!(
                "  not written, content already matches generated output: {}",
                path.display()
            );
        }
    }
}

/// State the count of writes skipped because `[workspace.ownership] user_owned` declares the
/// path owned by the consuming repository.
///
/// INFO, not WARN, and worded as a disposition rather than a problem: nothing here needs
/// fixing, and there is no remedy to name -- the remedy already happened, in a committed line
/// of `alef.toml` a reviewer approved. Reporting it as a warning beside
/// [`report_refused_writes`] would recreate the condition this option exists to end, where a
/// deliberate, correct state is announced as a failure on every single run.
///
/// Not silent either. A run that skipped 17 paths has narrowed its own scope, and the same
/// standard `crate::core::config::VerifyConfig`'s module doc sets for `alef verify` applies:
/// the number is stated unconditionally, so the declaration stays visible to whoever reads the
/// log rather than only to whoever reads the config. Paths go to DEBUG because the count is
/// the operator-facing fact and a large declaration would otherwise bury the rest of the run. ~keep
pub fn report_user_owned_skips(report: &WriteReport) {
    if report.user_owned_paths.is_empty() {
        return;
    }
    tracing::info!(
        "{} file(s) were not written because `[workspace.ownership] user_owned` in alef.toml \
         declares them owned by this repository rather than by alef. alef does not overwrite \
         them, does not stamp them with a provenance marker, and does not verify their \
         contents. Remove the matching pattern to hand a path back to alef.",
        report.user_owned_paths.len()
    );
    for path in &report.user_owned_paths {
        debug!("  declared user-owned, not written: {}", path.display());
    }
}

#[cfg(test)]
mod tests;
