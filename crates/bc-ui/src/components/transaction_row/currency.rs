//! Pure currency-marker resolution for amount inputs. No Leptos here so it is
//! host-tested natively.

use core::cmp::Reverse;

use bc_ipc::CommodityInfo;

/// Why a marker could not be resolved to a single commodity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerError {
    /// No currency marker was present.
    Missing,
    /// A marker was present but matched no commodity.
    Unknown(String),
    /// A marker matched more than one commodity.
    Ambiguous(String),
}

/// All `(marker, code)` pairs for a commodity: code, symbol, and aliases.
fn markers_for(c: &CommodityInfo) -> Vec<(String, String)> {
    let mut v = vec![(c.code.clone(), c.code.clone())];
    if let Some(s) = &c.symbol {
        v.push((s.clone(), c.code.clone()));
    }
    for a in &c.aliases {
        v.push((a.clone(), c.code.clone()));
    }
    v
}

/// Resolves a bare marker (no digits) to a canonical code.
///
/// Codes match case-insensitively; symbols/aliases match exactly.
///
/// # Arguments
///
/// * `currencies` - The set of known commodities to match against.
/// * `marker` - The marker string to resolve (no digits, trimmed by the caller).
///
/// # Returns
///
/// The canonical commodity code.
///
/// # Errors
///
/// Returns [`MarkerError::Missing`] when `marker` is empty, [`MarkerError::Unknown`]
/// when no commodity matches, and [`MarkerError::Ambiguous`] when more than one
/// commodity matches.
pub fn resolve_marker(currencies: &[CommodityInfo], marker: &str) -> Result<String, MarkerError> {
    let key = marker.trim();
    if key.is_empty() {
        return Err(MarkerError::Missing);
    }
    let mut hits: Vec<String> = Vec::new();
    for c in currencies {
        for (m, code) in markers_for(c) {
            let matches = m == key || m.eq_ignore_ascii_case(key);
            if matches && !hits.contains(&code) {
                hits.push(code.clone());
            }
        }
    }
    match hits.len() {
        0 => Err(MarkerError::Unknown(key.to_owned())),
        1 => Ok(hits.remove(0)),
        _ => Err(MarkerError::Ambiguous(key.to_owned())),
    }
}

/// Splits a marked amount into `(numeric_text, canonical_code)`.
///
/// Accepts a leading symbol/alias/code (`$100`, `A$100`, `AUD100`) or a
/// leading/trailing whitespace-separated token (`AUD 100`, `100 AUD`). Requires
/// a resolvable marker — a bare number errors with [`MarkerError::Missing`].
///
/// # Arguments
///
/// * `currencies` - The set of known commodities to match against.
/// * `input` - The raw input string to parse.
///
/// # Returns
///
/// A `(numeric_text, canonical_code)` pair on success.
///
/// # Errors
///
/// Returns [`MarkerError::Missing`] when no marker is found, [`MarkerError::Unknown`]
/// when a marker token matches no commodity, and [`MarkerError::Ambiguous`] when a
/// marker token matches more than one commodity.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "consumed by Task 6 posting-row amount parser; no callers exist yet on wasm32"
    )
)]
#[expect(
    clippy::string_slice,
    reason = "cut is derived from ASCII-only marker suffix length; UTF-8 boundary guaranteed"
)]
pub fn split_marked_amount(
    currencies: &[CommodityInfo],
    input: &str,
) -> Result<(String, String), MarkerError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(MarkerError::Missing);
    }

    // Gather all candidate markers, longest first (so "A$" beats "$").
    let mut markers: Vec<(String, String)> = currencies.iter().flat_map(markers_for).collect();
    markers.sort_by_key(|m| Reverse(m.0.len()));

    // 1) Whitespace-separated token at either end.
    //    A non-numeric token is treated as a marker attempt: propagate errors so
    //    "XYZ 100" returns Unknown rather than falling through to Missing.
    if let Some((head, rest)) = s.split_once(char::is_whitespace) {
        let head_numeric = head
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.');
        if !head_numeric {
            let code = resolve_marker(currencies, head)?;
            return Ok((rest.trim().to_owned(), code));
        }
    }
    if let Some((rest, tail)) = s.rsplit_once(char::is_whitespace) {
        let tail_numeric = tail
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.');
        if !tail_numeric {
            let code = resolve_marker(currencies, tail)?;
            return Ok((rest.trim().to_owned(), code));
        }
    }

    // 2) Glued leading marker (symbol/alias/code immediately before the number).
    for (m, code) in &markers {
        if let Some(rest) = s.strip_prefix(m.as_str()) {
            // Only accept when what's left starts like a number (digit, sign, or dot).
            if rest
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
            {
                // Propagate ambiguity error for the matched prefix.
                drop(resolve_marker(currencies, m)?);
                return Ok((rest.trim().to_owned(), code.clone()));
            }
        }
    }

    // 3) Glued trailing code (e.g. "100AUD") — only alphabetic codes/aliases.
    for (m, code) in &markers {
        if m.chars().all(|c| c.is_ascii_alphabetic())
            && let Some(rest) = s.to_ascii_uppercase().strip_suffix(&m.to_ascii_uppercase())
            && rest
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.')
        {
            drop(resolve_marker(currencies, m)?);
            let cut = rest.len();
            return Ok((s[..cut].trim().to_owned(), code.clone()));
        }
    }

    Err(MarkerError::Missing)
}

#[cfg(test)]
mod tests {
    use bc_ipc::CommodityInfo;
    use pretty_assertions::assert_eq;

    use super::*;

    fn registry() -> Vec<CommodityInfo> {
        vec![
            CommodityInfo::new("c1", "USD", Some("$".to_owned()), vec!["US$".to_owned()]),
            CommodityInfo::new("c2", "AUD", Some("A$".to_owned()), vec!["AU$".to_owned()]),
        ]
    }

    #[test]
    fn leading_symbol_resolves() {
        assert_eq!(
            split_marked_amount(&registry(), "$100"),
            Ok(("100".to_owned(), "USD".to_owned()))
        );
    }

    #[test]
    fn longest_symbol_wins() {
        assert_eq!(
            split_marked_amount(&registry(), "A$100"),
            Ok(("100".to_owned(), "AUD".to_owned()))
        );
    }

    #[test]
    fn trailing_code_resolves() {
        assert_eq!(
            split_marked_amount(&registry(), "100 AUD"),
            Ok(("100".to_owned(), "AUD".to_owned()))
        );
    }

    #[test]
    fn leading_code_case_insensitive() {
        assert_eq!(
            split_marked_amount(&registry(), "aud 100"),
            Ok(("100".to_owned(), "AUD".to_owned()))
        );
    }

    #[test]
    fn unmarked_is_error() {
        assert_eq!(
            split_marked_amount(&registry(), "100"),
            Err(MarkerError::Missing)
        );
    }

    #[test]
    fn unknown_marker_is_error() {
        assert!(matches!(
            split_marked_amount(&registry(), "XYZ 100"),
            Err(MarkerError::Unknown(_))
        ));
    }

    #[test]
    fn ambiguous_marker_is_error() {
        let amb = vec![
            CommodityInfo::new("c1", "USD", Some("$".to_owned()), vec![]),
            CommodityInfo::new("c2", "AUD", Some("$".to_owned()), vec![]),
        ];
        assert!(matches!(
            split_marked_amount(&amb, "$100"),
            Err(MarkerError::Ambiguous(_))
        ));
    }
}
