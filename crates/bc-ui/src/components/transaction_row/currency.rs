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

/// A single marker-to-code mapping entry for a commodity.
struct MarkerEntry {
    /// The marker string (code, symbol, or alias).
    marker: String,
    /// The canonical commodity code this marker resolves to.
    code: String,
    /// Whether this entry originated from the commodity's code field.
    ///
    /// When `true`, matching is case-insensitive. When `false` (symbol or alias),
    /// matching is exact.
    is_code: bool,
}

/// All marker entries for a commodity: code (case-insensitive), symbol, and aliases (exact).
fn markers_for(c: &CommodityInfo) -> Vec<MarkerEntry> {
    let mut v = vec![MarkerEntry {
        marker: c.code.clone(),
        code: c.code.clone(),
        is_code: true,
    }];
    if let Some(s) = &c.symbol {
        v.push(MarkerEntry {
            marker: s.clone(),
            code: c.code.clone(),
            is_code: false,
        });
    }
    for a in &c.aliases {
        v.push(MarkerEntry {
            marker: a.clone(),
            code: c.code.clone(),
            is_code: false,
        });
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
        for entry in markers_for(c) {
            let matches = if entry.is_code {
                entry.marker.eq_ignore_ascii_case(key)
            } else {
                entry.marker == key
            };
            if matches && !hits.contains(&entry.code) {
                hits.push(entry.code.clone());
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
    let mut markers: Vec<MarkerEntry> = currencies.iter().flat_map(markers_for).collect();
    markers.sort_by_key(|m| Reverse(m.marker.len()));

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
    //    Allow an optional leading sign before the marker (e.g. "-$50"): strip it,
    //    match the marker, then re-attach the sign to the numeric remainder.
    let (sign, body): (&str, &str) = if let Some(rest) = s.strip_prefix('-') {
        ("-", rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        ("+", rest)
    } else {
        ("", s)
    };
    for entry in &markers {
        // Alphabetic CODE markers match case-insensitively; symbols/aliases stay exact.
        let stripped = if entry.is_code && entry.marker.chars().all(|c| c.is_ascii_alphabetic()) {
            body.get(..entry.marker.len())
                .filter(|p| p.eq_ignore_ascii_case(&entry.marker))
                .map(|_| &body[entry.marker.len()..])
        } else {
            body.strip_prefix(entry.marker.as_str())
        };
        if let Some(rest) = stripped {
            // Only accept when what's left starts like a number (digit, sign, or dot).
            if rest
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
            {
                // Propagate ambiguity error for the matched prefix.
                drop(resolve_marker(currencies, &entry.marker)?);
                return Ok((format!("{sign}{}", rest.trim()), entry.code.clone()));
            }
        }
    }

    // 3) Glued trailing code (e.g. "100AUD") — only alphabetic codes/aliases.
    for entry in &markers {
        if entry.marker.chars().all(|c| c.is_ascii_alphabetic())
            && let Some(rest) = s
                .to_ascii_uppercase()
                .strip_suffix(&entry.marker.to_ascii_uppercase())
            && rest
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.')
        {
            drop(resolve_marker(currencies, &entry.marker)?);
            let cut = rest.len();
            return Ok((s[..cut].trim().to_owned(), entry.code.clone()));
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

    /// Codes match case-insensitively; symbols/aliases must match exactly.
    #[test]
    fn symbol_exact_match_code_case_insensitive() {
        // Registry: code "USD", symbol "us$" (lowercase).
        let reg = vec![CommodityInfo::new(
            "c1",
            "USD",
            Some("us$".to_owned()),
            vec![],
        )];

        // "US$100" — uppercase "US$" does NOT match the lowercase symbol "us$".
        assert!(
            matches!(
                split_marked_amount(&reg, "US$100"),
                Err(MarkerError::Unknown(_) | MarkerError::Missing)
            ),
            "uppercase symbol variant must not match lowercase symbol"
        );

        // "us$100" — exact lowercase symbol DOES match.
        assert_eq!(
            split_marked_amount(&reg, "us$100"),
            Ok(("100".to_owned(), "USD".to_owned())),
            "exact-case symbol must resolve"
        );

        // "usd 100" — code match is case-insensitive.
        assert_eq!(
            split_marked_amount(&reg, "usd 100"),
            Ok(("100".to_owned(), "USD".to_owned())),
            "code must resolve case-insensitively"
        );

        // resolve_marker bare API.
        assert!(
            matches!(resolve_marker(&reg, "US$"), Err(MarkerError::Unknown(_))),
            "resolve_marker must not match wrong-case symbol"
        );
        assert_eq!(
            resolve_marker(&reg, "us$"),
            Ok("USD".to_owned()),
            "resolve_marker must match exact-case symbol"
        );
        assert_eq!(
            resolve_marker(&reg, "USD"),
            Ok("USD".to_owned()),
            "resolve_marker must match code exactly"
        );
        assert_eq!(
            resolve_marker(&reg, "usd"),
            Ok("USD".to_owned()),
            "resolve_marker must match code case-insensitively"
        );
    }

    #[test]
    fn negative_leading_symbol_resolves() {
        assert_eq!(
            split_marked_amount(&registry(), "-$50"),
            Ok(("-50".to_owned(), "USD".to_owned()))
        );
    }

    #[test]
    fn negative_leading_multichar_symbol_resolves() {
        assert_eq!(
            split_marked_amount(&registry(), "-A$100"),
            Ok(("-100".to_owned(), "AUD".to_owned()))
        );
    }

    #[test]
    fn sign_after_symbol_still_resolves() {
        assert_eq!(
            split_marked_amount(&registry(), "$-100"),
            Ok(("-100".to_owned(), "USD".to_owned()))
        );
    }

    #[test]
    fn leading_glued_code_case_insensitive() {
        assert_eq!(
            split_marked_amount(&registry(), "usd100"),
            Ok(("100".to_owned(), "USD".to_owned()))
        );
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
