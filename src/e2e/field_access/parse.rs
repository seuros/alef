use super::types::PathSegment;

pub(super) fn strip_numeric_indices(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut key = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    closed = true;
                    break;
                }
                key.push(inner);
            }
            if closed && !key.is_empty() && key.chars().all(|k| k.is_ascii_digit()) {
                // Numeric index — drop it entirely (including any trailing dot).
            } else {
                result.push('[');
                result.push_str(&key);
                if closed {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    // Collapse any double-dots introduced by dropping `[N].` sequences.
    while result.contains("..") {
        result = result.replace("..", ".");
    }
    if result.starts_with('.') {
        result.remove(0);
    }
    result
}

pub(crate) fn normalize_numeric_indices(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut key = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    closed = true;
                    break;
                }
                key.push(inner);
            }
            if closed && !key.is_empty() && key.chars().all(|k| k.is_ascii_digit()) {
                result.push_str("[0]");
            } else {
                result.push('[');
                result.push_str(&key);
                if closed {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub(crate) fn normalize_indices_to_wildcards(path: &str) -> String {
    let mut normalized = String::with_capacity(path.len());
    let mut characters = path.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '[' {
            normalized.push(character);
            continue;
        }
        let mut index = String::new();
        while let Some(&inner) = characters.peek() {
            characters.next();
            if inner == ']' {
                break;
            }
            index.push(inner);
        }
        normalized.push('[');
        if !index.chars().all(|inner| inner.is_ascii_digit()) {
            normalized.push_str(&index);
        }
        normalized.push(']');
    }
    normalized
}

/// The field/array/map-access name carried by a path segment, or `None` for `.length`/`.count`
/// pseudo-segments, which never name a real struct field. Shared by every IR path-walker
/// (`ir_enum`, `ir_collection`) that has to advance a type cursor one segment at a time.
pub(super) fn segment_name(segment: &PathSegment) -> Option<&str> {
    match segment {
        PathSegment::Field(name) | PathSegment::ArrayField { name, .. } => Some(name),
        PathSegment::MapAccess { field, .. } => Some(field),
        PathSegment::Length => None,
    }
}

pub(super) fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        if part == "length" || part == "count" || part == "size" {
            segments.push(PathSegment::Length);
        } else if let Some(bracket_pos) = part.find('[') {
            let name = part[..bracket_pos].to_string();
            let key = part[bracket_pos + 1..].trim_end_matches(']').to_string();
            if key.is_empty() {
                // `foo[]` — a bare bracket LOSES its wildcard meaning here and becomes index 0,
                // indistinguishable from a hand-written `foo[0]`. Nothing downstream restores it:
                // `FieldResolver::inject_array_indexing` passes an explicit `ArrayField` straight
                // through ("the user's explicit index takes precedence"), and the renderers emit
                // the index verbatim. A caller that means "every element" must therefore split the
                // wildcard off with `FieldResolver::wildcard_split` BEFORE building an accessor —
                // reaching this arm with a wildcard is how a whole-array claim silently becomes an
                // element-zero check. ~keep
                segments.push(PathSegment::ArrayField { name, index: 0 });
            } else if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
                // `foo[N]` — user-typed explicit numeric index.
                let index: usize = key.parse().unwrap_or(0);
                segments.push(PathSegment::ArrayField { name, index });
            } else {
                // `foo[key]` — string-keyed map access.
                segments.push(PathSegment::MapAccess { field: name, key });
            }
        } else {
            segments.push(PathSegment::Field(part.to_string()));
        }
    }
    segments
}
