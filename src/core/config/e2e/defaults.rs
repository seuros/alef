pub(super) fn default_test_apps_dir() -> String {
    "test_apps".to_string()
}
pub(super) fn default_response_body_field() -> String {
    "body".to_string()
}

pub(super) fn default_harness_host() -> String {
    "127.0.0.1".to_string()
}

pub(super) fn default_harness_port() -> u16 {
    8000
}
pub(super) fn default_fixtures_dir() -> String {
    "fixtures".to_string()
}

pub(super) fn default_output_dir() -> String {
    "e2e".to_string()
}

pub(super) fn default_test_documents_dir() -> String {
    "test_documents".to_string()
}
/// The name a generated call binds its return value to when the call does not name one.
///
/// Both the serde default below and [`CallConfig::effective_result_var`](super::CallConfig::effective_result_var)
/// read this constant, so the name a TOML-loaded call gets and the name a consumer falls back to
/// cannot drift apart. ~keep
pub(super) const DEFAULT_RESULT_VAR: &str = "result";

pub(super) fn default_result_var() -> String {
    DEFAULT_RESULT_VAR.to_string()
}

pub(super) fn default_returns_result() -> bool {
    false
}
pub(super) fn default_arg_type() -> String {
    "string".to_string()
}
pub(super) fn default_true() -> bool {
    true
}
