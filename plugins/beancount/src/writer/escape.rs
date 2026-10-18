//! Spelling helpers for the writer: string escaping and the name grammars.

/// Quotes `text` as a Beancount string.
///
/// A line break becomes a space, because a model edits a string that spans
/// lines poorly. `\` and `"` are backslash-escaped, which is what the
/// importer's string reader undoes.
///
/// # Arguments
///
/// * `text` - The raw text.
///
/// # Returns
///
/// The text wrapped in double quotes.
pub(super) fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(2));
    out.push('"');
    let mut flattened = false;
    for c in text.chars() {
        match c {
            '\n' | '\r' => {
                flattened = true;
                out.push(' ');
            }
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    if flattened {
        bc_sdk::warn!("wrote a line break as a space"; text = text);
    }
    out
}

/// Reports whether `segment` is a valid account segment.
fn is_account_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Reports whether `path` is a valid account path.
///
/// Every `:`-separated segment matches `[A-Z0-9][A-Za-z0-9-]*`.
pub(super) fn is_account(path: &str) -> bool {
    path.split(':').all(is_account_segment)
}

/// Reports whether `name` is a valid tag (or link) name: `[A-Za-z0-9-_/.]+`.
pub(super) fn is_tag(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
}

/// Reports whether `code` is a valid commodity: `[A-Z][A-Z0-9'._-]{0,22}[A-Z0-9]`.
pub(super) fn is_commodity(code: &str) -> bool {
    let chars: Vec<char> = code.chars().collect();
    let (Some(first), Some(last)) = (chars.first(), chars.last()) else {
        return false;
    };
    let body_ok = |c: &char| c.is_ascii_uppercase() || c.is_ascii_digit() || "'._-".contains(*c);
    (2..=24).contains(&chars.len())
        && first.is_ascii_uppercase()
        && (last.is_ascii_uppercase() || last.is_ascii_digit())
        && chars.iter().all(body_ok)
}

/// Reports whether `key` is a valid metadata key: `[a-z][A-Za-z0-9_-]*`.
pub(super) fn is_meta_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Reports whether `label` survives the importer's cost-block reader.
///
/// The reader cuts a posting line at `;` and splits a cost block at `,`, so a
/// label holding either, or a brace, would read back changed.
pub(super) fn is_cost_label(label: &str) -> bool {
    !label.contains([';', ',', '{', '}'])
}
