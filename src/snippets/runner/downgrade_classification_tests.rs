//! `Downgraded` vs. `capability_capped` classification tests: a validator ceiling, an
//! environment gap, or a snippet's declared `level:` must each land in the right bucket.

use super::*;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;

/// A validator that passes but declares a ceiling below `Run`, standing in for the real
/// zig/toml/json/yaml validators whose maximum level is genuinely lower than TypeCheck.
struct CappedValidator {
    language: crate::snippets::types::Language,
    ceiling: ValidationLevel,
}

impl SnippetValidator for CappedValidator {
    fn language(&self) -> crate::snippets::types::Language {
        self.language
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        self.ceiling
    }
}

fn network_snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::Rust,
        title: None,
        code: "fn main() {}".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata {
            side_effect: Some(SideEffectClass::Network),
            ..SnippetMetadata::default()
        },
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

/// A validator whose declared ceiling sits below the requested level has not degraded
/// anything — that level was never reachable for the language. Marking it `Downgraded`
/// made `strict` + a level any validator caps below structurally unsatisfiable, so a
/// consumer's only escape was lowering the level for every other language too. ~keep
#[test]
fn validator_ceiling_passes_instead_of_downgrading() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Syntax,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert!(summary.results[0].capability_capped);
    assert_eq!(summary.downgraded, 0);
    assert_eq!(summary.capability_capped, 1);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
    assert_eq!(
        summary.results[0].downgrade_reason,
        Some(DowngradeReason::ValidatorCapability)
    );
}

/// The exemption is narrow: an annotation that lowers the level is the author's choice,
/// not a capability ceiling, so it must still register as a downgrade and still fail strict. ~keep
#[test]
fn annotation_downgrade_is_not_treated_as_a_capability_ceiling() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Run,
    }));
    let mut snippet = network_snippet();
    snippet.annotation = Some(crate::snippets::types::SnippetAnnotation {
        kind: SnippetAnnotationKind::SyntaxOnly,
        reason: None,
    });
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
    assert!(!summary.results[0].capability_capped);
    assert_eq!(summary.downgraded, 1);
    assert_eq!(summary.capability_capped, 0);
    assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Annotation));
}

/// A validator whose `max_level` never moves but whose current environment can't back a
/// deeper level (no real type-checker installed, say) reports that gap through
/// `achievable_level`, not `max_level`. `php`/`ruby`/`elixir` conflated the two: they claimed
/// `Run` as their ceiling while their `typecheck`-level check never resolved a symbol, so
/// `capability_capped` waved every request through as a language-ceiling Pass instead of a
/// downgrade. This pins the two inputs apart at the mechanism level. ~keep
struct EnvironmentLimitedValidator {
    language: crate::snippets::types::Language,
}

impl SnippetValidator for EnvironmentLimitedValidator {
    fn language(&self) -> crate::snippets::types::Language {
        self.language
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if requested == ValidationLevel::TypeCheck {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }
}

#[test]
fn environment_limited_validator_downgrades_instead_of_capability_capping() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(EnvironmentLimitedValidator {
        language: crate::snippets::types::Language::Rust,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
    assert!(!summary.results[0].capability_capped);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
    assert_eq!(summary.downgraded, 1);
    assert_eq!(summary.capability_capped, 0);
    assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Environment));
}

/// A validator whose `achievable_level` gap is declared structural — see
/// `achievable_level_is_structural` — is exempted from `Downgraded` the same way a
/// `max_level` ceiling is, mirroring `validator_ceiling_passes_instead_of_downgrading` but
/// through the `achievable_level` input instead. This is the generic form of what
/// `php`/`ruby`/`elixir`/`bash`/`r`'s own tests pin, without depending on a real toolchain. ~keep
struct StructurallyCappedAchievableValidator {
    language: crate::snippets::types::Language,
}

impl SnippetValidator for StructurallyCappedAchievableValidator {
    fn language(&self) -> crate::snippets::types::Language {
        self.language
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if requested == ValidationLevel::TypeCheck {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        requested == ValidationLevel::TypeCheck
    }
}

#[test]
fn structural_achievable_level_gap_is_capability_capped_not_downgraded() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(StructurallyCappedAchievableValidator {
        language: crate::snippets::types::Language::Rust,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert!(summary.results[0].capability_capped);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
    assert_eq!(summary.downgraded, 0);
    assert_eq!(summary.capability_capped, 1);
    assert_eq!(
        summary.results[0].downgrade_reason,
        Some(DowngradeReason::ValidatorCapability)
    );
}

/// The regression this whole change fixes: a front-matter `level:` is a validation contract,
/// not a suppression. Before this, `discovery::extract_snippets_from_file` collapsed
/// `metadata.level` into the same `annotation` field a `<!-- snippet:*-only -->` comment
/// uses, so an author who declared exactly the level they wanted was charged a `Downgraded`
/// violation identical to one who suppressed validation below what was requested. No test
/// exercised this end to end through `run_validation` — every prior downgrade test
/// constructed `snippet.annotation` directly, which is exactly why the collapse went
/// unnoticed. ~keep
#[test]
fn declared_level_contract_passes_instead_of_downgrading() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Run,
    }));
    let mut snippet = network_snippet();
    snippet.metadata.level = Some(ValidationLevel::Syntax);
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert!(!summary.results[0].capability_capped);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
    assert_eq!(summary.downgraded, 0);
    assert_eq!(summary.capability_capped, 0);
    assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Declared));
    assert_eq!(
        summary.results[0].message.as_deref(),
        Some("requested typecheck, validated at declared level syntax")
    );
}

/// Regression for the `docs.snippets.validation_level = "run"` reported as unreachable: an
/// e2e-generated fixture snippet's front matter always declares `level: typecheck` (see
/// `e2e::snippets::render_snippet_markdown`), which caps `effective_validation_level` below
/// any stronger `config.level` a consumer configures — `run` included. That cap is legitimate
/// (the snippet's own contract, `DowngradeReason::Declared`), so this does not turn it into a
/// failure; what it must not do is stay silent about the gap. Before the `finalize_result`
/// message fix, this asserted `"validated at declared level typecheck"`, which never named
/// what was actually requested — indistinguishable from an ordinary declared-level snippet
/// with no gap at all. ~keep
#[test]
fn declared_typecheck_ceiling_names_the_clamped_run_request() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Run,
    }));
    let mut snippet = network_snippet();
    snippet.metadata.level = Some(ValidationLevel::TypeCheck);
    let config = RunnerConfig {
        level: ValidationLevel::Run,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::TypeCheck);
    assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Declared));
    assert_eq!(
        summary.results[0].message.as_deref(),
        Some("requested run, validated at declared level typecheck"),
        "a `run` request clamped by a snippet's declared level must name both the request and \
         the level it was clamped to, not just the clamped level"
    );
}

/// Negative control for the same clamp path: a consumer who legitimately configures a lower
/// `validation_level` (not `run`) against a snippet with no front-matter `level:` contract at
/// all must validate normally, with no downgrade classification and no clamp message —
/// `effective_validation_level` has nothing to fold against `requested`, so it passes through
/// unchanged. ~keep
#[test]
fn legitimately_configured_lower_level_has_no_downgrade_reason() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Run,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::TypeCheck);
    assert_eq!(summary.results[0].downgrade_reason, None);
    assert_eq!(summary.results[0].message, None);
}

/// A declared `level:` is a contract for what was requested, not a guarantee the environment
/// or validator can honor it: when the actual outcome lands below even the declared level,
/// that is a real downgrade, not a satisfied contract.
#[test]
fn declared_level_the_validator_cannot_reach_still_downgrades() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(EnvironmentLimitedValidator {
        language: crate::snippets::types::Language::Rust,
    }));
    let mut snippet = network_snippet();
    snippet.metadata.level = Some(ValidationLevel::Compile);
    let config = RunnerConfig {
        level: ValidationLevel::TypeCheck,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
    assert!(!summary.results[0].capability_capped);
    assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
    assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Environment));
}

/// A validator that can reach the requested level must not be flagged at all.
#[test]
fn validator_at_or_above_requested_level_is_not_capped() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CappedValidator {
        language: crate::snippets::types::Language::Rust,
        ceiling: ValidationLevel::Run,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::Syntax,
        parallelism: 1,
        cache_dir: None,
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.results[0].status, SnippetStatus::Pass);
    assert!(!summary.results[0].capability_capped);
    assert_eq!(summary.capability_capped, 0);
}
