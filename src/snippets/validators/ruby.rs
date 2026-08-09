use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_script};

pub struct RubyValidator;

impl SnippetValidator for RubyValidator {
    fn language(&self) -> Language {
        Language::Ruby
    }

    fn is_available(&self) -> bool {
        which::which("ruby").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }

        run_script(snippet, level, timeout_secs, None, ".rb", "ruby", &["-c"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }
        run_script(snippet, level, timeout_secs, session, ".rb", "ruby", &["-c"])
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn is_api_signature(code: &str) -> bool {
    code.lines().count() <= 3 && code.contains(" -> ")
}
