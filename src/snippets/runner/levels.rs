//! How the validation level actually attempted is resolved.
//!
//! Four narrowing steps that must agree: the author's `<!-- snippet:*-only -->` annotation, the
//! front-matter `level:` contract, the validator's permanent `max_level`, and this run's
//! environment-dependent `achievable_level`. They are kept together because `finalize_result`
//! has to tell a *downgrade* (author or environment lowered it) from a *contract* (the snippet
//! declared this level) from *structurally unreachable* (no environment could satisfy it), and
//! that distinction only makes sense with all four in view. ~keep

use crate::snippets::runner::RunnerConfig;
use crate::snippets::types::{Snippet, SnippetAnnotationKind, ValidationLevel};

/// The ceiling imposed by a `<!-- snippet:*-only -->` comment annotation, if any. Distinct from
/// `snippet.metadata.level` (a front-matter `level:` contract): an annotation is the author
/// suppressing validation below what the run requested, so `finalize_result` keeps it a
/// `Downgraded` cause, while `snippet.metadata.level` is read directly wherever the contract
/// counterpart is needed. ~keep
pub(super) fn annotation_level_limit(snippet: &Snippet) -> Option<ValidationLevel> {
    snippet
        .annotation
        .as_ref()
        .and_then(|annotation| match annotation.kind {
            SnippetAnnotationKind::SyntaxOnly => Some(ValidationLevel::Syntax),
            SnippetAnnotationKind::CompileOnly => Some(ValidationLevel::Compile),
            SnippetAnnotationKind::TypeCheckOnly => Some(ValidationLevel::TypeCheck),
            SnippetAnnotationKind::Skip => None,
        })
}

/// The level implied by the snippet's own declarations, independent of the validator or
/// environment: an annotation lowers it as a downgrade; a front-matter `level:` lowers it as a
/// contract instead. Both narrow the level actually attempted the same way here — only
/// `finalize_result` tells the two apart, to decide whether hitting this level is a violation or
/// a satisfied request. ~keep
pub(super) fn effective_validation_level(snippet: &Snippet, requested: ValidationLevel) -> ValidationLevel {
    [annotation_level_limit(snippet), snippet.metadata.level]
        .into_iter()
        .flatten()
        .fold(requested, ValidationLevel::min)
}

/// The level a validator will actually be invoked at: the requested level, narrowed by the
/// snippet's own declarations (`effective_validation_level`), by the validator's permanent
/// `max_level` ceiling, and by `achievable_level` — this run's environment-dependent limit (e.g.
/// no real type-checker binary on `PATH`). ~keep
pub(super) fn capped_level(
    snippet: &Snippet,
    config: &RunnerConfig,
    validator: &dyn crate::snippets::validators::SnippetValidator,
) -> ValidationLevel {
    effective_validation_level(snippet, config.level)
        .min(validator.max_level())
        .min(validator.achievable_level(config.level))
}

/// Whether the validator can never reach `requested` for this snippet's language, in any
/// environment: either its permanent `max_level` sits below it, or its `achievable_level` gap is
/// declared structural (see `SnippetValidator::achievable_level_is_structural`). Both make a
/// strict request for `requested` unsatisfiable for this language regardless of the user's
/// environment, so `finalize_result` treats them the same way. ~keep
pub(super) fn structurally_unreachable(
    validator: &dyn crate::snippets::validators::SnippetValidator,
    requested: ValidationLevel,
) -> bool {
    validator.max_level() < requested
        || (validator.achievable_level(requested) < requested && validator.achievable_level_is_structural(requested))
}
