use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_script};

pub struct PhpValidator;

impl SnippetValidator for PhpValidator {
    fn language(&self) -> Language {
        Language::Php
    }

    fn is_available(&self) -> bool {
        which::which("php").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, None, ".php", "php", &["-l"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, session, ".php", "php", &["-l"])
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}
