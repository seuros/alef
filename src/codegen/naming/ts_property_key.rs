//! Rendering a serde WIRE name into the key position of a TypeScript object type member.
//!
//! A host-language identifier and a wire name are different name surfaces, and only the first is
//! guaranteed to be spellable bare. `naming::public_host_identifier` and `to_node_name` always
//! return something lexically legal, so every emitter that declared a `.d.ts` member from a HOST
//! name could interpolate it directly. The emitters that describe a *structurally* bridged value
//! — `backends::napi::gen_bindings::errors`'s `untagged_variant_dts_type` and
//! `backends::wasm::gen_bindings::ts_union` — must declare serde wire names instead, because the
//! runtime object is produced by `serde_json` / `serde_wasm_bindgen` against the core type and
//! never passes through a wrapper with host-cased accessors. Wire names are unconstrained:
//! `#[serde(rename = "content-type")]`, `#[serde(rename_all = "kebab-case")]`, and a
//! `#[serde(rename)]` carrying a space or a quote are all legal serde and all produce a key that
//! is a *syntax error* if pasted bare after four spaces of indentation.
//!
//! So the wire name has to be rendered, not interpolated, and the rendering rule has to live in
//! one place: two emitters describing the same runtime shape that disagree about when a key needs
//! quotes is the same per-emitter-oracle failure this branch's other fixes closed. ~keep

use crate::core::keywords::JS_KEYWORDS;

/// Render a serde wire name as a TypeScript/JavaScript object property key: bare when the name is
/// a legal identifier, otherwise a double-quoted, escaped string key.
///
/// The output is a complete key token, ready to be followed by `?`, `:` and the member type. It
/// is NOT an identifier — never feed it to `escape_identifier` or a casing helper, and never use
/// it in dot-access position.
///
/// Reserved words are quoted even though the ECMAScript grammar accepts any `IdentifierName` in
/// key position, so `{ default: string }` would in fact compile. Quoting them is therefore
/// conservative rather than required — it costs nothing, keeps the emitted key valid if it is
/// ever moved to a position that takes an `Identifier` rather than an `IdentifierName`, and means
/// a reader does not have to know that distinction to check the output. ~keep
#[must_use]
pub fn ts_property_key(wire_name: &str) -> String {
    if is_ascii_js_identifier(wire_name) && !JS_KEYWORDS.contains(&wire_name) {
        return wire_name.to_string();
    }
    quote_property_key(wire_name)
}

/// ASCII-only `IdentifierName` test: `[A-Za-z_$][A-Za-z0-9_$]*`.
///
/// ECMAScript also admits non-ASCII `ID_Start`/`ID_Continue`, so a name like `café` is a legal
/// bare key and this returns `false` for it. Quoting it is still correct output — the narrow test
/// only ever costs a pair of quotes, whereas a wrong `true` emits a broken declaration. ~keep
fn is_ascii_js_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

/// Wrap a wire name in double quotes, escaping what a double-quoted ECMAScript string literal
/// cannot carry raw.
///
/// This is lexical escaping of a *value*, not emission of code structure, so it stays in Rust
/// rather than moving into a Jinja template: a template can interpolate the finished token but
/// has no way to decide which of a name's characters need a backslash. `U+2028`/`U+2029` are
/// escaped because they terminate a line in ECMAScript source even inside a string literal. ~keep
fn quote_property_key(wire_name: &str) -> String {
    let mut rendered = String::with_capacity(wire_name.len() + 2);
    rendered.push('"');
    for ch in wire_name.chars() {
        match ch {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            '\u{8}' => rendered.push_str("\\b"),
            '\u{b}' => rendered.push_str("\\v"),
            '\u{c}' => rendered.push_str("\\f"),
            '\u{2028}' => rendered.push_str("\\u2028"),
            '\u{2029}' => rendered.push_str("\\u2029"),
            control if (control as u32) < 0x20 => rendered.push_str(&unicode_escape(control)),
            other => rendered.push(other),
        }
    }
    rendered.push('"');
    rendered
}

fn unicode_escape(ch: char) -> String {
    format!("\\u{:04x}", ch as u32)
}

#[cfg(test)]
mod tests;
