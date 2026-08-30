//! Generic text-formatting helpers shared across scaffold language modules: XML
//! escaping, author-string parsing, and first-letter capitalization.

/// Escape special characters for XML text content.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Parse an author string like `"Name <email>"` into `(name, email)`.
/// If no angle brackets are found, returns `(input, "")`.
pub fn parse_author(s: &str) -> (&str, &str) {
    if let Some(start) = s.find('<')
        && let Some(end) = s.find('>')
    {
        let name = s[..start].trim();
        let email = &s[start + 1..end];
        return (name, email);
    }
    (s.trim(), "")
}

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
