//! Python test-file import-line computation, split out of `test_file.rs` (over the
//! 1,000-line file-size cap) to keep the touched concern's growth out of that file.
//!
//! Declared as a submodule of `test_file` (its only caller), not a sibling under `python`,
//! so `super` below reaches `test_file` first and `python::helpers` needs one more `super`.

use crate::e2e::fixture::Fixture;

use super::super::helpers::is_skipped;
use super::references_identifier;

/// `import pytest` and the `sys.stdout.write` diagnostic branch are only genuinely
/// referenced in the emitted body under specific conditions. Mirroring those exactly
/// (rather than a coarser "any fixture has an env api key" check) means each import is
/// only emitted when it will actually be used — a fixture with both a mock response AND
/// an env api key never reaches the `pytest.skip(...)` branch in `test_function.rs` (it
/// takes the mock/real-API `sys.stdout.write` branch instead), so blanket-including
/// `pytest` for it produced a real unused import; blanket-including `sys` for every env
/// api key fixture (mock or not) did the same for the print/`T201` branch. ~keep
pub(super) fn compute_pytest_and_sys_import_needs(
    fixtures: &[&Fixture],
    client_factory: Option<&str>,
    has_error_test: bool,
    is_async: bool,
) -> (bool, bool) {
    let has_skipped_fixture = fixtures
        .iter()
        .filter(|f| !f.is_http_test())
        .any(|f| is_skipped(f, "python"));
    let has_pytest_skip_call = client_factory.is_some()
        && fixtures.iter().filter(|f| !f.is_http_test()).any(|f| {
            let has_mock = f.mock_response.is_some() || f.http.is_some();
            !has_mock && f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some()
        });
    let needs_pytest = has_error_test || is_async || has_skipped_fixture || has_pytest_skip_call;

    let needs_sys_import = client_factory.is_some()
        && fixtures.iter().filter(|f| !f.is_http_test()).any(|f| {
            let has_mock = f.mock_response.is_some() || f.http.is_some();
            has_mock && f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some()
        });

    (needs_pytest, needs_sys_import)
}

/// Finalizes `stdlib_imports`/`thirdparty_bare`: adds `json`/`re` when the already-rendered
/// `fixtures_body` actually references them (`http_test.jinja` only needs those modules for
/// fixtures whose request/response shape reaches the branch that uses them — reading the
/// answer off the rendered body keeps this the one source of truth instead of a second copy
/// of `http.rs`'s branch conditions that could silently drift from it), adds the
/// unconditional/precomputed entries, and sorts each list isort-canonically. ~keep
#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_stdlib_and_bare_imports(
    fixtures_body: &str,
    has_http_tests: bool,
    needs_base64_import: bool,
    needs_json_import: bool,
    needs_os_import: bool,
    needs_path_import: bool,
    needs_sys_import: bool,
    needs_pytest: bool,
    stdlib_imports: &mut Vec<String>,
    thirdparty_bare: &mut Vec<String>,
) {
    let needs_json_import = needs_json_import
        || references_identifier(fixtures_body, "json.dumps")
        || references_identifier(fixtures_body, "json.loads");
    let needs_re_import =
        references_identifier(fixtures_body, "re.match") || references_identifier(fixtures_body, "re.search");

    if needs_base64_import {
        stdlib_imports.push("import base64".to_string());
    }
    if needs_json_import {
        stdlib_imports.push("import json".to_string());
    }
    if needs_os_import {
        stdlib_imports.push("import os".to_string());
    }
    if needs_path_import {
        stdlib_imports.push("from pathlib import Path".to_string());
    }
    if needs_re_import {
        stdlib_imports.push("import re".to_string());
    }
    if has_http_tests {
        stdlib_imports.push("import urllib.request".to_string());
    }
    if needs_sys_import {
        stdlib_imports.push("import sys".to_string());
    }
    if needs_pytest {
        thirdparty_bare.push("import pytest".to_string());
    }
    // A plain lexicographic sort interleaves `from X import Y` before `import Z` whenever X
    // sorts earlier than Z (e.g. "from pathlib import Path" before "import os"), which isort
    // (ruff's I001) rejects — it wants every `import X` line before every `from X import Y`
    // line within a section. Sorting on `(is_from, line)` gets both groups right and each one
    // alphabetized without maintaining two separate Vecs end-to-end. ~keep
    stdlib_imports.sort_by(|a, b| (a.starts_with("from "), a).cmp(&(b.starts_with("from "), b)));
    thirdparty_bare.sort();
}
