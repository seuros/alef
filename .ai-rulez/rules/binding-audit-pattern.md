---
priority: high
---
Audit bindings for coverage gaps — is every public Rust item exposed in every target language binding?

**Steps:**

1. Enumerate intentional removals in the source repo's `alef.toml`. The real surfaces are:
   - `[crates.exclude]` (`types`/`functions`/`methods`/`fields`) inside a `[[crates]]` entry — a crate-wide list, unioned across every language.
   - Per-language `exclude_types` / `exclude_functions` fields directly on each `[crates.<lang>]` table (e.g. `[crates.python].exclude_types`, `[crates.ffi].exclude_functions`) — unioned with the crate-wide list for that language only. `src/docs/language_pages/excludes.rs::language_excludes` is the canonical per-language union (it also documents which FFI-derived families additionally fold in `[crates.ffi]`/`[crates.jni]`).
   - `[workspace.opaque_types]` — workspace-level only, no per-crate override exists. This is a type-**remapping** declaration (Rust type name → external path alef can't extract), not an exclusion list: it changes how a type is represented, it does not remove it from the surface.
   - There is **no** `[crates.skipped]`, **no** bare `exclude_types` key, and **no** per-crate override under `[workspace.crates."<name>"]`. `[[crates]]` is a plain array (`WorkspaceConfig` has no `crates` field; `RawCrateConfig` has no `skipped` field) — there is no name-keyed map to override into.
   Record these.
2. Grep the source Rust crate for `#[alef::skip]` and `#[doc(hidden)]` — the only two attribute-level intentional removals (`src/extract/extractor/helpers/attributes.rs::extract_binding_exclusion_reason`). **`#[alef::exclude]` and `#[alef::opaque]` do not exist in alef** — do not grep for or expect them.
   Both real attributes set the `binding_excluded` flag on the item's IR node at extraction time. That flag is honored **independently by every downstream consumer** — each backend, `src/core/jni.rs`, `src/core/validation/readiness.rs`, docs generation, etc. all filter on it separately; there is no single central enforcement point. Critically, `language_excludes` — the function that answers "what does `alef.toml` exclude for language X" — **never consults `binding_excluded`**; it only reads the config surfaces from step 1. So a `#[alef::skip]`'d (or `#[doc(hidden)]`) item is correctly invisible in every generated binding, but any tooling (including a naive coverage ledger) that treats `language_excludes`'s answer as the *complete* set of intentional removals will misclassify that skipped item as a real gap, because it never shows up in `language_excludes`'s output at all.
3. For every public Rust item (functions, types, methods, enum variants) not captured in steps 1–2, verify its presence across every generated binding under `packages/<lang>/`.
4. Diff across targets: identify items present in some bindings but not others. That gap is the bug.
5. **Triage outcomes:**
   - **Codegen issue** → fix in alef repo (`../alef`), update `CHANGELOG.md` `[Unreleased]`, commit, normal release flow.
   - **Alef-owned workflow/action bug** → fix the owning Alef workflow/action repository, commit, then retag only the documented action tags for that repository.
   - **Per-target config gap** → fix the consumer repo's `alef.toml`; do not touch alef or actions.
6. Always update `CHANGELOG.md` `[Unreleased]` on each upstream commit; use `--no-verify` only when hooks block a critical fix and re-run hooks afterwards.
