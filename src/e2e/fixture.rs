//! Fixture loading, validation, and grouping for e2e test generation.

use crate::core::config::e2e::ArgMapping;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod metadata;
mod protocol;
pub use metadata::{
    FixtureDocs, FixtureDocsFileInput, FixtureDocsOperation, FixtureDocsPresentation, FixtureEnv, SetupCall,
    SideEffectClass, SnippetCoverageException, TemplateReturnForm,
};
pub use protocol::{
    AsyncApiFixture, WebSocketFixture, WebSocketFrameType, WebSocketHandler, WebSocketMessage,
    WebSocketMessageDirection, WebSocketSession,
};

/// Mock HTTP response for testing HTTP clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    /// HTTP status code.
    pub status: u16,
    /// JSON response body (for non-streaming responses).
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// SSE stream chunks (for streaming responses).
    /// Each chunk is a JSON object sent as `data: <chunk>\n\n`.
    #[serde(default)]
    pub stream_chunks: Option<Vec<serde_json::Value>>,
    /// Response headers to apply to the mock response.
    /// Bridged from `http.expected_response.headers` for HTTP fixtures.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Visitor specification for visitor pattern tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitorSpec {
    /// Map of callback method name to action.
    pub callbacks: BTreeMap<String, CallbackAction>,
}

/// Action a visitor callback should take.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum CallbackAction {
    /// Return VisitResult::Skip.
    #[serde(rename = "skip")]
    Skip,
    /// Return VisitResult::Continue.
    #[serde(rename = "continue")]
    Continue,
    /// Return VisitResult::PreserveHtml.
    #[serde(rename = "preserve_html")]
    PreserveHtml,
    /// Return VisitResult::Custom with static output.
    #[serde(rename = "custom")]
    Custom {
        /// The static replacement string.
        output: String,
    },
    /// Return VisitResult::Custom with template interpolation.
    #[serde(rename = "custom_template")]
    CustomTemplate {
        /// Template with placeholders like {text}, {href}.
        template: String,
        /// How the generated visitor returns the rendered template to the host.
        /// `Dict` (default) returns `{"custom": "..."}` (or per-language equivalent)
        /// to hit the structured-result code path; `BareString` returns the raw
        /// rendered string to hit the string-result code path. Both must produce
        /// `VisitResult::Custom`.
        #[serde(default)]
        return_form: TemplateReturnForm,
    },
}

impl CallbackAction {
    /// Canonical serde action tag consumed by every generated visitor bridge.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Continue => "continue",
            Self::PreserveHtml => "preserve_html",
            Self::Custom { .. } | Self::CustomTemplate { .. } => "custom",
        }
    }
}

/// A single e2e test fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    /// Unique identifier (used as test function name).
    pub id: String,
    /// Optional category (defaults to parent directory name).
    #[serde(default)]
    pub category: Option<String>,
    /// Human-readable description.
    pub description: String,
    #[serde(default)]
    pub docs: Option<FixtureDocs>,
    /// Declarative capabilities required to publish this fixture as a snippet.
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub skip: Option<SkipDirective>,
    /// Environment variable requirements (used by smoke/live tests).
    #[serde(default)]
    pub env: Option<FixtureEnv>,
    /// Setup calls executed before the main call (used to register backends, etc).
    #[serde(default)]
    pub setup: Vec<SetupCall>,
    /// Named call config to use (references `[e2e.calls.<name>]`).
    /// When omitted, uses the default `[e2e.call]`.
    #[serde(default)]
    pub call: Option<String>,
    /// Input data passed to the function under test.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Pass this fixture's declared URLs to the call verbatim instead of substituting
    /// the mock server address.
    ///
    /// ~keep `mock_url` and `mock_url_list` arguments normally ignore `input.url` /
    /// `input.urls` entirely and bind the per-fixture mock server address, because
    /// almost every fixture wants a live server to talk to. A minority of fixtures
    /// are testing the address *itself* — SSRF policy, scheme rejection, host parsing
    /// — and for those the substitution silently replaces the subject of the test, so
    /// several fixtures declaring different addresses all end up exercising one
    /// trivial case and passing for the wrong reason.
    ///
    /// This is opt-in per fixture rather than inferred: an "is it absolute?" heuristic
    /// would reclassify existing fixtures that legitimately declare an absolute URL
    /// next to a mock server, changing their meaning without anyone editing them.
    #[serde(default)]
    pub preserve_input_urls: bool,
    /// Optional mock HTTP response for testing HTTP clients.
    #[serde(default)]
    pub mock_response: Option<MockResponse>,
    /// Optional visitor specification for visitor pattern tests.
    #[serde(default)]
    pub visitor: Option<VisitorSpec>,
    /// Fixture-level argument mappings. When non-empty, overrides call_config.args
    /// for this specific fixture (used for trait-bridge stubs and other per-fixture args).
    #[serde(default)]
    pub args: Vec<ArgMapping>,
    /// Assertion recipes this fixture opts into.
    ///
    /// Domain-shaped assertions such as embeddings, keyword extraction,
    /// tree-query helpers, and streaming pseudo-fields require an explicit
    /// recipe opt-in so generic e2e fixtures don't silently inherit
    /// project-specific assumptions.
    #[serde(default)]
    pub assertion_recipes: Vec<String>,
    /// List of assertions to check.
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    /// Source file path (populated during loading).
    #[serde(skip)]
    pub source: String,
    /// HTTP server test specification. When present, this fixture tests
    /// an HTTP handler rather than a function call.
    #[serde(default)]
    pub http: Option<HttpFixture>,
    #[serde(default)]
    pub asyncapi: Option<AsyncApiFixture>,
    #[serde(default)]
    pub websocket: Option<WebSocketFixture>,
}

/// HTTP server test specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFixture {
    /// Handler/route definition.
    pub handler: HttpHandler,
    /// The HTTP request to send.
    pub request: HttpRequest,
    /// Expected response.
    pub expected_response: HttpExpectedResponse,
}

/// Handler/route definition for HTTP server tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHandler {
    /// Route pattern (e.g., "/users/{user_id}").
    pub route: String,
    /// HTTP method (GET, POST, PUT, etc.).
    pub method: String,
    /// JSON Schema for request body validation.
    #[serde(default)]
    pub body_schema: Option<serde_json::Value>,
    /// Parameter schemas by source (path, query, header, cookie).
    #[serde(default)]
    pub parameters: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// Middleware configuration.
    #[serde(default)]
    pub middleware: Option<HttpMiddleware>,
}

/// HTTP request to send in a server test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    #[serde(default)]
    pub form_data: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub content_type: Option<String>,
}

impl HttpRequest {
    /// Encode form_data as a URL-encoded body string (key=value&key=value).
    /// Returns None if form_data is None.
    pub fn url_encoded_body(&self) -> Option<String> {
        self.form_data.as_ref().map(|form| {
            form.iter()
                .map(|(k, v)| {
                    let encoded_k = Self::url_encode(k);
                    let encoded_v = Self::url_encode(v);
                    format!("{}={}", encoded_k, encoded_v)
                })
                .collect::<Vec<_>>()
                .join("&")
        })
    }

    /// Simple URL encoding for form data (RFC 3986).
    fn url_encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{:02X}", b),
            })
            .collect()
    }
}

/// Expected HTTP response specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExpectedResponse {
    pub status_code: u16,
    /// Exact body match.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// Partial body match (only check specified fields).
    #[serde(default)]
    pub body_partial: Option<serde_json::Value>,
    /// Header expectations. Special tokens: `<<uuid>>`, `<<present>>`, `<<absent>>`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Expected validation errors (for 422 responses).
    #[serde(default)]
    pub validation_errors: Option<Vec<ValidationErrorExpectation>>,
}

/// Expected validation error entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationErrorExpectation {
    pub loc: Vec<String>,
    pub msg: String,
    #[serde(rename = "type")]
    pub error_type: String,
}

/// CORS policy configuration for HTTP handler tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Allowed origins (e.g. `["https://example.com"]`). Empty means deny all.
    #[serde(default)]
    pub allow_origins: Vec<String>,
    /// Allowed HTTP methods (e.g. `["GET", "POST"]`). Empty means deny all.
    #[serde(default)]
    pub allow_methods: Vec<String>,
    /// Allowed request headers (e.g. `["Content-Type"]`). Empty means deny all.
    #[serde(default)]
    pub allow_headers: Vec<String>,
    /// Exposed response headers (e.g. `["X-Total-Count"]`).
    #[serde(default)]
    pub expose_headers: Vec<String>,
    /// `Access-Control-Max-Age` value in seconds.
    #[serde(default)]
    pub max_age: Option<u64>,
    /// Whether to allow credentials.
    #[serde(default)]
    pub allow_credentials: bool,
}

/// A single static file entry for the static-files middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticFile {
    /// Relative path within the served directory (e.g. `"hello.txt"`).
    pub path: String,
    /// File content (plain text or HTML string).
    pub content: String,
}

/// Static-files middleware configuration for HTTP handler tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticFilesConfig {
    /// URL route prefix (e.g. `"/public"`).
    pub route_prefix: String,
    /// Files to write to the temporary directory.
    #[serde(default)]
    pub files: Vec<StaticFile>,
    /// Whether to serve `index.html` for directory requests.
    #[serde(default)]
    pub index_file: bool,
    /// `Cache-Control` header value to apply.
    #[serde(default)]
    pub cache_control: Option<String>,
}

/// Middleware configuration for HTTP handler tests.
///
/// Unknown keys are rejected rather than ignored: a middleware category that no
/// field covers would otherwise be dropped at deserialization and never reach any
/// backend, producing a silently weaker test suite. A hard parse error surfaces the
/// gap at fixture-load time instead. ~keep
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpMiddleware {
    #[serde(default)]
    pub jwt_auth: Option<serde_json::Value>,
    #[serde(default)]
    pub api_key_auth: Option<serde_json::Value>,
    #[serde(default)]
    pub compression: Option<serde_json::Value>,
    #[serde(default)]
    pub rate_limit: Option<serde_json::Value>,
    #[serde(default)]
    pub request_timeout: Option<serde_json::Value>,
    /// Maximum request-body size policy (e.g. `{"max_bytes": 1024}`). Passed
    /// through opaquely so backends can wire it to their body-limit middleware.
    #[serde(default)]
    pub body_limit: Option<serde_json::Value>,
    #[serde(default)]
    pub request_id: Option<serde_json::Value>,
    /// CORS policy to apply via tower-http `CorsLayer`.
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    /// Static-files configuration to serve via tower-http `ServeDir`.
    #[serde(default)]
    pub static_files: Option<Vec<StaticFilesConfig>>,
    /// GraphQL route configuration (e.g. `{"schema": "...", "response_data": {...}}`).
    /// Passed through opaquely so backends can register a GraphQL endpoint rather
    /// than the generic route+handler pattern used by the other middleware fields.
    #[serde(default)]
    pub graphql: Option<serde_json::Value>,
    /// Request/response lifecycle hooks, keyed by phase (e.g.
    /// `{"on_request": [{"name": "...", "handler": "..."}]}`). Passed through
    /// opaquely: phase names and per-hook fields are defined by the target
    /// runtime, not by this model.
    #[serde(default)]
    pub lifecycle_hooks: Option<serde_json::Value>,
    /// JSON-RPC service configuration carrying an OpenRPC document (e.g.
    /// `{"enabled": true, "spec": {...}}`). Passed through opaquely so backends
    /// can hand the document to their own OpenRPC support.
    #[serde(default)]
    pub openrpc: Option<serde_json::Value>,
    /// Deferred/background work configuration (e.g. `{"enabled": true, "max_concurrent": 5}`).
    /// Passed through opaquely because the tuning knobs differ per runtime.
    #[serde(default)]
    pub background_tasks: Option<serde_json::Value>,
    /// WebSocket endpoint configuration (e.g. `{"enabled": true}`). Passed through
    /// opaquely so backends can decide how to expose an upgrade route.
    #[serde(default)]
    pub websocket: Option<serde_json::Value>,
    /// Authorization policy applied after authentication (e.g. `{"required_role": "admin"}`).
    /// Passed through opaquely because the policy vocabulary is application-defined.
    #[serde(default)]
    pub authorization: Option<serde_json::Value>,
}

const ORIGIN_ROOT_ROUTE_PREFIXES: [&str; 2] = ["/robots", "/sitemap"];

/// Returns true for fixture route paths that must be served from the origin root rather than
/// under a fixture-namespaced prefix. Mirrors the identical predicate in the standalone
/// mock-server binary (`codegen/rust/mock_server.rs`).
fn is_host_root_path(path: &str) -> bool {
    ORIGIN_ROOT_ROUTE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

impl Default for Fixture {
    fn default() -> Self {
        Fixture {
            id: String::new(),
            category: None,
            description: String::new(),
            docs: None,
            requirements: Vec::new(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: Vec::new(),
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }
}

impl Fixture {
    pub fn docs_call_fixture(&self) -> Self {
        let mut fixture = self.clone();
        if let Some(input) = self.docs.as_ref().and_then(|docs| docs.input.as_ref()) {
            fixture.input = input.clone();
        }
        if let Some(presentation) = self.docs.as_ref().and_then(|docs| docs.presentation.as_ref()) {
            if let Some(call) = &presentation.call {
                fixture.call = Some(call.clone());
            }
            if let Some(input) = &presentation.input {
                fixture.input = input.clone();
            }
            for file in &presentation.files {
                if let Some(value) = fixture.input.pointer_mut(&file.field) {
                    *value = serde_json::Value::String(file.path.clone());
                }
            }
            if let Some(args) = &presentation.args {
                fixture.args = args.clone();
            }
        }
        fixture.mock_response = None;
        if let Some(input) = fixture.input.as_object_mut() {
            input.remove("mock_responses");
        }
        replace_docs_mock_urls(&mut fixture.input);
        fixture
    }

    pub fn docs_files_for_arg(&self, field: &str) -> Vec<FixtureDocsFileInput> {
        let base = if field == "input" {
            if self.input.get("extract_input").is_some() {
                "/extract_input".to_string()
            } else {
                String::new()
            }
        } else {
            format!("/{}", field.strip_prefix("input.").unwrap_or(field).replace('.', "/"))
        };
        self.docs
            .as_ref()
            .and_then(|docs| docs.presentation.as_ref())
            .map(|presentation| {
                presentation
                    .files
                    .iter()
                    .filter_map(|file| {
                        file.field.strip_prefix(&base).and_then(|relative| {
                            (relative.is_empty() || relative.starts_with('/')).then(|| FixtureDocsFileInput {
                                field: relative.to_string(),
                                path: file.path.clone(),
                            })
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn docs_file_path(&self, field: &str) -> Option<String> {
        self.docs_files_for_arg(field)
            .into_iter()
            .find(|file| file.field.is_empty())
            .map(|file| file.path)
    }

    pub fn has_docs_presentation(&self) -> bool {
        self.docs.as_ref().is_some_and(|docs| {
            !docs.shows.is_empty()
                || docs
                    .presentation
                    .as_ref()
                    .is_some_and(|presentation| !presentation.operations.is_empty())
        })
    }

    /// Resolve the effective args for this fixture, preferring fixture-level args when present.
    ///
    /// When `self.args` is non-empty, returns a reference to it. Otherwise, returns
    /// a reference to `call_config.args`. This allows fixtures to override the call's
    /// default args (e.g., for trait-bridge stubs that need per-fixture test backend setup).
    pub fn resolved_args<'a>(&'a self, call_config: &'a crate::core::config::e2e::CallConfig) -> &'a [ArgMapping] {
        if !self.args.is_empty() {
            &self.args
        } else {
            &call_config.args
        }
    }

    /// Returns true if this is an HTTP server test fixture.
    pub fn is_http_test(&self) -> bool {
        self.http.is_some()
    }

    /// Returns true if this fixture requires a mock HTTP server.
    /// This is true when the fixture declares a single mock response, an HTTP expected
    /// response, or one or more entries in the generic `input.mock_responses` route array.
    pub fn needs_mock_server(&self) -> bool {
        if self.mock_response.is_some() || self.http.is_some() {
            return true;
        }
        self.input
            .get("mock_responses")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false)
    }

    /// Returns the effective mock response for this fixture, bridging both schemas:
    /// - call fixture schema: `mock_response: { status, body, stream_chunks }`
    /// - HTTP fixture schema: `http.expected_response: { status_code, body, headers }`
    ///
    /// Returns `None` if neither schema is present.
    pub fn as_mock_response(&self) -> Option<MockResponse> {
        if let Some(mock) = &self.mock_response {
            return Some(mock.clone());
        }
        if let Some(http) = &self.http {
            return Some(MockResponse {
                status: http.expected_response.status_code,
                body: http.expected_response.body.clone(),
                stream_chunks: None,
                headers: http.expected_response.headers.clone(),
            });
        }
        None
    }

    /// Returns true if the mock response uses streaming (SSE).
    pub fn is_streaming_mock(&self) -> bool {
        self.mock_response
            .as_ref()
            .and_then(|m| m.stream_chunks.as_ref())
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    /// Returns true if this fixture needs a dedicated origin-root listener.
    ///
    /// Route-array fixtures are normally mounted under `/fixtures/<id>` in the shared
    /// mock server. A dedicated listener is required when a route path or fixture body
    /// makes the client under test resolve follow-up requests from the origin root rather
    /// than the fixture namespace. Mirrors the `is_host_root_path` predicate in the
    /// standalone mock-server binary (`codegen/rust/mock_server.rs`).
    ///
    /// Origin-root fixtures get a dedicated per-fixture listener and their base URL is
    /// published in the `MOCK_SERVERS={"fixture_id":"http://..."}` JSON line.
    pub fn has_host_root_route(&self) -> bool {
        if let Some(arr) = self.input.get("mock_responses").and_then(|v| v.as_array()) {
            if arr.iter().any(|entry| {
                entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(is_host_root_path)
                    .unwrap_or(false)
            }) {
                return true;
            }
            // A response can trigger a follow-up request to an origin-root path. In that
            // case, the fixture must be served on a dedicated listener so the next request
            // resolves against the same route table. Three trigger shapes are detected:
            //   - 3xx with Location: /...
            //   - any status with Refresh: <s>;url=/...
            //   - 200 HTML with <meta http-equiv="refresh" content="...url=/...">
            return arr.iter().any(|entry| {
                let status = entry.get("status_code").and_then(|v| v.as_u64()).unwrap_or(0);
                let headers = entry.get("headers").and_then(|v| v.as_object());
                let location_redirect = (300..400).contains(&status)
                    && headers
                        .map(|hdrs| {
                            hdrs.iter().any(|(name, value)| {
                                name.eq_ignore_ascii_case("location")
                                    && value.as_str().is_some_and(|s| s.starts_with('/'))
                            })
                        })
                        .unwrap_or(false);
                let refresh_redirect = headers
                    .map(|hdrs| {
                        hdrs.iter().any(|(name, value)| {
                            if !name.eq_ignore_ascii_case("refresh") {
                                return false;
                            }
                            value
                                .as_str()
                                .and_then(|s| s.to_ascii_lowercase().find("url=").map(|i| (s.to_owned(), i)))
                                .map(|(s, idx)| s[idx + 4..].trim_start().starts_with('/'))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                let meta_refresh = entry
                    .get("body_inline")
                    .and_then(|v| v.as_str())
                    .map(|body| {
                        let lower = body.to_ascii_lowercase();
                        lower
                            .split("http-equiv=\"refresh\"")
                            .nth(1)
                            .and_then(|s| s.split("content=").nth(1))
                            .map(|s| s.trim_start_matches(['"', '\'']).contains("url=/"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                // Inline HTML anchor with host-absolute target (`<a href="/page1">`) uses
                // the same trigger as the runtime mock-server `has_inline_host_link`
                // detection. Generated tests for multi-page fixtures use the shared
                // `/fixtures/<id>/` URL while clients resolve linked `/page` paths against
                // the host root.
                let inline_host_link = entry
                    .get("body_inline")
                    .and_then(|v| v.as_str())
                    .map(|body| body.contains("href=\"/") || body.contains("href='/"))
                    .unwrap_or(false);
                location_redirect || refresh_redirect || meta_refresh || inline_host_link
            });
        }
        false
    }

    /// Get the resolved category (explicit or from source directory).
    pub fn resolved_category(&self) -> String {
        self.category.clone().unwrap_or_else(|| {
            Path::new(&self.source)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string()
        })
    }
}

fn replace_docs_mock_urls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            *text = text.replace(crate::e2e::codegen::MOCK_URL_PLACEHOLDER, "https://example.com");
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_docs_mock_urls(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                replace_docs_mock_urls(value);
            }
        }
        _ => {}
    }
}

/// Skip directive for conditionally excluding fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipDirective {
    /// Languages to skip (empty means skip all).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Human-readable reason for skipping.
    #[serde(default)]
    pub reason: Option<String>,
}

impl SkipDirective {
    /// Check if this fixture should be skipped for a given language.
    pub fn should_skip(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.iter().any(|l| l == language)
    }
}

/// A single assertion in a fixture.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Assertion {
    /// Assertion type (equals, contains, not_empty, error, etc.).
    #[serde(rename = "type")]
    pub assertion_type: String,
    /// Field path to access on the result (dot-separated).
    #[serde(default)]
    pub field: Option<String>,
    /// Expected value (string, number, bool, or array depending on type).
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Expected values (for contains_all, contains_any).
    #[serde(default)]
    pub values: Option<Vec<serde_json::Value>>,
    /// Method name to call on the result (for method_result assertions).
    #[serde(default)]
    pub method: Option<String>,
    /// Assertion check type for the method result (equals, is_true, is_false, greater_than_or_equal, count_min).
    #[serde(default)]
    pub check: Option<String>,
    /// Arguments to pass to the method call (for method_result assertions).
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// Return type hint for C method_result codegen.
    ///
    /// Supported values:
    /// - `"string"` — the method returns a heap-allocated `char*` that must be
    ///   freed with `free()` after the assertion.  The generator emits
    ///   `char* _r = call(); assert(...); free(_r);`.
    ///
    /// Defaults to primitive integer dispatch when absent.
    #[serde(default)]
    pub return_type: Option<String>,
}

impl Assertion {
    pub(crate) fn expected_values(&self) -> Vec<&serde_json::Value> {
        self.values
            .as_ref()
            .map(|values| values.iter().collect())
            .or_else(|| self.value.as_ref().map(|value| vec![value]))
            .unwrap_or_default()
    }
}

/// A group of fixtures sharing the same category.
#[derive(Debug, Clone)]
pub struct FixtureGroup {
    pub category: String,
    pub fixtures: Vec<Fixture>,
}

/// Load all fixtures from a directory recursively.
pub fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>> {
    let mut fixtures = Vec::new();
    load_fixtures_recursive(dir, dir, &mut fixtures)?;

    // Validate: check for duplicate IDs
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in &fixtures {
        if let Some(prev_source) = seen.get(&f.id) {
            bail!(
                "duplicate fixture ID '{}': found in '{}' and '{}'",
                f.id,
                prev_source,
                f.source
            );
        }
        seen.insert(f.id.clone(), f.source.clone());
    }

    // Sort by (category, id) for deterministic output
    fixtures.sort_by(|a, b| {
        let cat_cmp = a.resolved_category().cmp(&b.resolved_category());
        cat_cmp.then_with(|| a.id.cmp(&b.id))
    });

    Ok(fixtures)
}

fn load_fixtures_recursive(base: &Path, dir: &Path, fixtures: &mut Vec<Fixture>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_fixtures_recursive(base, &path, fixtures)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read fixture: {}", path.display()))?;
            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            // Try parsing as array first, then as single fixture. Normalize at the
            // raw JSON level so fixture-level helper fields that are not stored on
            // `Fixture` can still influence generated argument input.
            let parsed: Vec<Fixture> = if content.trim_start().starts_with('[') {
                let values: Vec<serde_json::Value> = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?;
                values
                    .into_iter()
                    .map(normalize_fixture_value)
                    .map(serde_json::from_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .with_context(|| format!("failed to parse fixture array: {}", path.display()))?
            } else {
                let value: serde_json::Value = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                let single: Fixture = serde_json::from_value(normalize_fixture_value(value))
                    .with_context(|| format!("failed to parse fixture: {}", path.display()))?;
                vec![single]
            };

            for mut fixture in parsed {
                fixture.source = relative.clone();
                validate_docs_file_inputs(&fixture)
                    .with_context(|| format!("invalid docs file input in fixture: {}", path.display()))?;
                validate_docs_metadata(&fixture)
                    .with_context(|| format!("invalid docs metadata in fixture: {}", path.display()))?;
                // Expand template expressions (e.g. `{{ repeat 'x' 10000 times }}`)
                // in all JSON string values so generators emit the expanded values.
                expand_json_templates(&mut fixture.input);
                if let Some(ref mut http) = fixture.http {
                    for v in http.request.headers.values_mut() {
                        *v = crate::e2e::escape::expand_fixture_templates(v);
                    }
                    if let Some(ref mut body) = http.request.body {
                        expand_json_templates(body);
                    }
                }
                fixtures.push(fixture);
            }
        }
    }
    Ok(())
}

fn validate_docs_metadata(fixture: &Fixture) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    let fixture_expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if docs.error == Some(true) && !fixture_expects_error {
        bail!("docs.error requires an error assertion");
    }
    if docs.error == Some(false) && fixture_expects_error {
        bail!("docs.error cannot be false when the fixture expects an error");
    }
    if docs.shows.iter().any(|path| path.trim().is_empty()) {
        bail!("docs.shows entries must be non-empty result paths");
    }
    Ok(())
}

fn validate_docs_file_inputs(fixture: &Fixture) -> Result<()> {
    let Some(presentation) = fixture.docs.as_ref().and_then(|docs| docs.presentation.as_ref()) else {
        return Ok(());
    };
    let input = presentation.input.as_ref().unwrap_or(&fixture.input);
    for file in &presentation.files {
        let path = Path::new(&file.path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            bail!("docs file path must be relative and traversal-free: {}", file.path);
        }
        if !file.field.starts_with('/') || input.pointer(&file.field).is_none() {
            bail!("docs file field must be an existing JSON pointer: {}", file.field);
        }
    }
    Ok(())
}

fn normalize_fixture_value(mut value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };

    if let Some(config) = object.get("config").cloned() {
        let input = object
            .entry("input")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(input_object) = input.as_object_mut() {
            input_object.entry("config".to_string()).or_insert(config);
        }
    }

    value
}

/// Validate that every id in every fixture's `skip.languages` refers to a
/// known e2e generator target.
///
/// ~keep A `skip.languages` id that is not an actual generator target silently
/// matches nothing in [`SkipDirective::should_skip`], so the fixture keeps
/// running everywhere the author thought it was disabled. So an id passes when
/// it is either configured for this run or in
/// [`crate::e2e::known_e2e_target_names`] — a real target the consumer simply
/// hasn't scaffolded, where matching nothing is correct and harmless.
///
/// `"ffi"` stays rejected even though `Language::Ffi` exists: it maps to the
/// `"c"` generator, so `"ffi"` itself never matches anything.
///
/// Returns an error naming the offending fixture, its source path, the bad
/// id, and the valid set as soon as the first unknown id is found.
pub fn validate_skip_languages(fixtures: &[Fixture], valid_languages: &[String]) -> Result<()> {
    let known_targets = crate::e2e::known_e2e_target_names();
    for fixture in fixtures {
        let Some(skip) = &fixture.skip else {
            continue;
        };
        for language in &skip.languages {
            let is_configured = valid_languages.iter().any(|valid| valid == language);
            let is_known_target = known_targets.iter().any(|known| known == language);
            if !is_configured && !is_known_target {
                let mut valid_ids = valid_languages.to_vec();
                for known in &known_targets {
                    if !valid_ids.contains(known) {
                        valid_ids.push(known.clone());
                    }
                }
                bail!(
                    "fixture '{}' ({}) has skip.languages id '{}' that is not a known e2e target \
                     (check for a typo); valid ids are: {}",
                    fixture.id,
                    fixture.source,
                    language,
                    valid_ids.join(", ")
                );
            }
        }
    }
    Ok(())
}

/// Group fixtures by their resolved category.
pub fn group_fixtures(fixtures: &[Fixture]) -> Vec<FixtureGroup> {
    let mut groups: HashMap<String, Vec<Fixture>> = HashMap::new();
    for f in fixtures {
        groups.entry(f.resolved_category()).or_default().push(f.clone());
    }
    let mut result: Vec<FixtureGroup> = groups
        .into_iter()
        .map(|(category, fixtures)| FixtureGroup { category, fixtures })
        .collect();
    result.sort_by(|a, b| a.category.cmp(&b.category));
    result
}

/// Recursively expand fixture template expressions in all string values of a JSON tree.
fn expand_json_templates(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            let expanded = crate::e2e::escape::expand_fixture_templates(s);
            if expanded != *s {
                *s = expanded;
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                expand_json_templates(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                expand_json_templates(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_with_mock_response() {
        let json = r#"{
            "id": "test_chat",
            "description": "Test chat",
            "call": "chat",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}]},
            "mock_response": {
                "status": 200,
                "body": {"choices": [{"message": {"content": "hello"}}]}
            },
            "assertions": [{"type": "not_error"}]
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
        assert_eq!(fixture.mock_response.unwrap().status, 200);
    }

    #[test]
    fn test_fixture_with_streaming_mock_response() {
        let json = r#"{
            "id": "test_stream",
            "description": "Test streaming",
            "input": {},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}, {"delta": " world"}]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.needs_mock_server());
        assert!(fixture.is_streaming_mock());
    }

    #[test]
    fn test_fixture_without_mock_response() {
        let json = r#"{
            "id": "test_no_mock",
            "description": "No mock",
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.needs_mock_server());
        assert!(!fixture.is_streaming_mock());
    }

    #[test]
    fn http_middleware_deserializes_lifecycle_hooks_keyed_by_phase() {
        let json = r#"{
            "lifecycle_hooks": {
                "on_request": [{"name": "request_logger", "handler": "log_request"}],
                "pre_validation": [
                    {"name": "rate_limiter", "handler": "check_rate_limit",
                     "config": {"max_requests": 10, "window_seconds": 60}}
                ]
            }
        }"#;
        let middleware: HttpMiddleware = serde_json::from_str(json).unwrap();
        let hooks = middleware
            .lifecycle_hooks
            .expect("lifecycle_hooks must survive deserialization");
        assert_eq!(hooks["on_request"][0]["handler"], serde_json::json!("log_request"));
        assert_eq!(
            hooks["pre_validation"][0]["config"]["max_requests"],
            serde_json::json!(10)
        );
    }

    #[test]
    fn http_middleware_deserializes_openrpc_spec_document() {
        let json = r#"{
            "openrpc": {
                "enabled": true,
                "spec": {
                    "openrpc": "1.3.2",
                    "info": {"title": "Math API", "version": "1.0.0"},
                    "methods": [{"name": "add"}]
                }
            }
        }"#;
        let middleware: HttpMiddleware = serde_json::from_str(json).unwrap();
        let openrpc = middleware.openrpc.expect("openrpc must survive deserialization");
        assert_eq!(openrpc["enabled"], serde_json::json!(true));
        assert_eq!(openrpc["spec"]["methods"][0]["name"], serde_json::json!("add"));
    }

    #[test]
    fn http_middleware_deserializes_background_tasks_config() {
        let json = r#"{"background_tasks": {"enabled": true, "max_concurrent": 5}}"#;
        let middleware: HttpMiddleware = serde_json::from_str(json).unwrap();
        let tasks = middleware
            .background_tasks
            .expect("background_tasks must survive deserialization");
        assert_eq!(tasks["max_concurrent"], serde_json::json!(5));
    }

    #[test]
    fn http_middleware_deserializes_websocket_config() {
        let json = r#"{"websocket": {"enabled": true}}"#;
        let middleware: HttpMiddleware = serde_json::from_str(json).unwrap();
        let websocket = middleware.websocket.expect("websocket must survive deserialization");
        assert_eq!(websocket["enabled"], serde_json::json!(true));
    }

    #[test]
    fn http_middleware_deserializes_authorization_policy() {
        let json = r#"{"authorization": {"required_role": "admin"}}"#;
        let middleware: HttpMiddleware = serde_json::from_str(json).unwrap();
        let authorization = middleware
            .authorization
            .expect("authorization must survive deserialization");
        assert_eq!(authorization["required_role"], serde_json::json!("admin"));
    }

    #[test]
    fn http_middleware_defaults_every_field_to_none() {
        let middleware: HttpMiddleware = serde_json::from_str("{}").unwrap();
        assert!(middleware.lifecycle_hooks.is_none());
        assert!(middleware.openrpc.is_none());
        assert!(middleware.background_tasks.is_none());
        assert!(middleware.websocket.is_none());
        assert!(middleware.authorization.is_none());
    }

    /// An unrecognised middleware category must fail loudly instead of being dropped —
    /// a silently ignored key produces a test suite that is weaker than it looks.
    #[test]
    fn http_middleware_rejects_unknown_key() {
        let error = serde_json::from_str::<HttpMiddleware>(r#"{"telemetry": {"enabled": true}}"#)
            .expect_err("unknown middleware keys must be rejected");
        assert!(
            error.to_string().contains("telemetry"),
            "error should name the offending key, got: {error}"
        );
    }

    #[test]
    fn normalize_fixture_value_copies_top_level_config_into_input() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {"kind": "uri", "uri": "doc.txt"},
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("markdown"))
        );
    }

    #[test]
    fn normalize_fixture_value_preserves_explicit_input_config() {
        let value = serde_json::json!({
            "id": "configured_call",
            "description": "Configured call",
            "input": {
                "kind": "uri",
                "uri": "doc.txt",
                "config": {"output_format": "html"}
            },
            "config": {"output_format": "markdown"}
        });

        let normalized = normalize_fixture_value(value);
        assert_eq!(
            normalized.pointer("/input/config/output_format"),
            Some(&serde_json::json!("html"))
        );
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_robot_route_path() {
        let json = r#"{
            "id": "robots_disallow_path",
            "description": "Robots fixture",
            "input": {
                "mock_responses": [
                    {"path": "/robots.txt", "status_code": 200, "body_inline": "User-agent: *\nDisallow: /"},
                    {"path": "/", "status_code": 200, "body_inline": "<html/>"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.has_host_root_route(), "expected true for /robots.txt path");
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_sitemap_route_path() {
        let json = r#"{
            "id": "sitemap_index",
            "description": "Sitemap fixture",
            "input": {
                "mock_responses": [
                    {"path": "/sitemap.xml", "status_code": 200, "body_inline": "<?xml version='1.0'?>"},
                    {"path": "/", "status_code": 200, "body_inline": "<html/>"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(fixture.has_host_root_route(), "expected true for /sitemap.xml path");
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_redirect_target() {
        let json = r#"{
            "id": "redirect_fixture",
            "description": "Redirect fixture",
            "input": {
                "mock_responses": [
                    {
                        "path": "/",
                        "status_code": 302,
                        "headers": {"Location": "/final"},
                        "body_inline": ""
                    },
                    {"path": "/final", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            fixture.has_host_root_route(),
            "expected origin-root listener for origin-root redirect target"
        );
    }

    #[test]
    fn has_host_root_route_true_for_origin_root_link_target() {
        let json = r#"{
            "id": "linked_pages",
            "description": "Linked pages",
            "input": {
                "mock_responses": [
                    {
                        "path": "/",
                        "status_code": 200,
                        "body_inline": "<html><a href='/page'>Page</a></html>"
                    },
                    {"path": "/page", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            fixture.has_host_root_route(),
            "expected origin-root listener for origin-root link target"
        );
    }

    #[test]
    fn has_host_root_route_false_for_data_json_path() {
        let json = r#"{
            "id": "data_endpoint",
            "description": "Namespaced route fixture",
            "input": {
                "mock_responses": [
                    {"path": "/data.json", "status_code": 200, "body_inline": "{}"}
                ]
            },
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.has_host_root_route(), "expected false for /data.json path");
    }

    #[test]
    fn has_host_root_route_false_for_single_mock_response_schema() {
        let json = r#"{
            "id": "basic_chat",
            "description": "Basic chat",
            "mock_response": {"status": 200, "body": {}},
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(
            !fixture.has_host_root_route(),
            "expected false for single mock_response schema"
        );
    }

    #[test]
    fn has_host_root_route_false_for_empty_mock_responses() {
        let json = r#"{
            "id": "empty_responses",
            "description": "No mock_responses",
            "input": {},
            "assertions": []
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        assert!(!fixture.has_host_root_route(), "expected false when no mock_responses");
    }

    #[test]
    fn validate_skip_languages_accepts_known_id() {
        let json = r#"{
            "id": "known_skip",
            "description": "Skips a real target",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["python", "node"], "reason": "not applicable"}
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        let valid = vec!["python".to_string(), "node".to_string(), "rust".to_string()];
        assert!(validate_skip_languages(&[fixture], &valid).is_ok());
    }

    #[test]
    fn validate_skip_languages_rejects_unknown_id() {
        let json = r#"{
            "id": "bogus_skip",
            "description": "Skips a nonexistent target",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["typescript"], "reason": "wrong id"}
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        let valid = vec!["python".to_string(), "node".to_string(), "rust".to_string()];
        let err = validate_skip_languages(&[fixture], &valid).expect_err("unknown id must fail validation");
        let message = err.to_string();
        assert!(
            message.contains("bogus_skip"),
            "error should name the fixture: {message}"
        );
        assert!(
            message.contains("typescript"),
            "error should name the bad id: {message}"
        );
        assert!(message.contains("python"), "error should list valid ids: {message}");
    }

    #[test]
    fn validate_skip_languages_accepts_known_target_not_in_configured_list() {
        let json = r#"{
            "id": "held_back_skip",
            "description": "Skips a target the consumer hasn't scaffolded yet",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["csharp"], "reason": "design-held backend"}
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        let valid = vec!["python".to_string(), "node".to_string(), "rust".to_string()];
        assert!(
            validate_skip_languages(&[fixture], &valid).is_ok(),
            "a known e2e target should validate even when it isn't in the configured list"
        );
    }

    #[test]
    fn validate_skip_languages_rejects_typo_id_even_when_similar_to_known_target() {
        let json = r#"{
            "id": "typo_skip",
            "description": "Skips a typo'd target name",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["c#", "wasm32"], "reason": "typo"}
        }"#;
        let fixture: Fixture = serde_json::from_str(json).unwrap();
        let valid = vec!["python".to_string(), "node".to_string(), "rust".to_string()];
        let err = validate_skip_languages(&[fixture], &valid).expect_err("typo id must still fail validation");
        let message = err.to_string();
        assert!(
            message.contains("typo_skip"),
            "error should name the fixture: {message}"
        );
        assert!(message.contains("c#"), "error should name the bad id: {message}");
        assert!(
            message.contains("not a known e2e target"),
            "error should say the id is not a known e2e target: {message}"
        );
    }

    #[test]
    fn validate_skip_languages_rejects_ffi_display_name_but_accepts_c() {
        let ffi_json = r#"{
            "id": "ffi_display_skip",
            "description": "Uses Language::Ffi's Display output, not its e2e target name",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["ffi"], "reason": "wrong id"}
        }"#;
        let ffi_fixture: Fixture = serde_json::from_str(ffi_json).unwrap();
        let valid = vec!["python".to_string(), "rust".to_string()];
        let err =
            validate_skip_languages(&[ffi_fixture], &valid).expect_err("'ffi' is not an e2e target and must fail");
        assert!(
            err.to_string().contains("not a known e2e target"),
            "'ffi' must be rejected because default_e2e_languages maps Language::Ffi to \"c\": {err}"
        );

        let c_json = r#"{
            "id": "c_target_skip",
            "description": "Uses the real e2e target name for the FFI binding",
            "input": {},
            "assertions": [],
            "skip": {"languages": ["c"], "reason": "not applicable"}
        }"#;
        let c_fixture: Fixture = serde_json::from_str(c_json).unwrap();
        assert!(
            validate_skip_languages(&[c_fixture], &valid).is_ok(),
            "'c' is the real e2e target name that Language::Ffi maps to and must be accepted"
        );
    }

    #[test]
    fn docs_file_inputs_require_safe_existing_pointers() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "file_input",
            "description": "Reads a local document",
            "input": {"content": [1, 2, 3]},
            "assertions": [],
            "docs": {
                "topic": "guides",
                "presentation": {"files": [{"field": "/content", "path": "../secret.pdf"}]}
            }
        }))
        .expect("fixture");

        let error = validate_docs_file_inputs(&fixture).expect_err("traversal must be rejected");
        assert!(error.to_string().contains("traversal-free"), "{error}");

        let mut missing = fixture;
        let file = &mut missing
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .files[0];
        file.path = "document.pdf".into();
        file.field = "/missing".into();
        let error = validate_docs_file_inputs(&missing).expect_err("missing pointers must be rejected");
        assert!(error.to_string().contains("existing JSON pointer"), "{error}");
    }

    #[test]
    fn docs_files_resolve_relative_to_each_argument() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "typed_file_input",
            "description": "Reads a typed document input",
            "input": {"extract_input": {"kind": "bytes", "bytes": [1, 2, 3]}},
            "assertions": [],
            "docs": {
                "topic": "guides",
                "presentation": {
                    "files": [{"field": "/extract_input/bytes", "path": "document.pdf"}]
                }
            }
        }))
        .expect("fixture");

        assert_eq!(
            fixture.docs_files_for_arg("input"),
            vec![FixtureDocsFileInput {
                field: "/bytes".into(),
                path: "document.pdf".into(),
            }]
        );
    }

    #[test]
    fn assertion_expected_values_supports_plural_and_singular_forms() {
        let plural: Assertion = serde_json::from_value(serde_json::json!({
            "type": "not_contains",
            "field": "content",
            "values": ["unsafe markup", "unsafe handler"]
        }))
        .expect("plural assertion");
        let singular: Assertion = serde_json::from_value(serde_json::json!({
            "type": "not_contains",
            "field": "content",
            "value": "unsafe markup"
        }))
        .expect("singular assertion");

        assert_eq!(
            plural.expected_values(),
            vec![
                &serde_json::json!("unsafe markup"),
                &serde_json::json!("unsafe handler")
            ]
        );
        assert_eq!(singular.expected_values(), vec![&serde_json::json!("unsafe markup")]);
    }
}
