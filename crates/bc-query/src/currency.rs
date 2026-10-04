//! Currency-marker resolution for amount text: `A$150`, `AUD 150`, `150AUD`.
//!
//! Codes match case-insensitively; symbols and aliases match exactly. The
//! longest marker wins, so `A$` beats `$`.

use core::cmp::Reverse;

/// A commodity as marker resolution sees it.
pub trait MarkerSource {
    /// The canonical code, matched case-insensitively.
    fn code(&self) -> &str;
    /// The display symbol, matched exactly.
    fn symbol(&self) -> Option<&str>;
    /// Further markers, matched exactly.
    fn aliases(&self) -> &[String];
}

/// An owned [`MarkerSource`] for callers without a commodity type of their own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Commodity {
    /// The canonical code.
    pub code: String,
    /// The display symbol.
    pub symbol: Option<String>,
    /// Further exact-match markers.
    pub aliases: Vec<String>,
}

impl Commodity {
    /// Creates a commodity from its code, symbol and aliases.
    ///
    /// # Arguments
    ///
    /// * `code` - The canonical code.
    /// * `symbol` - The display symbol, if any.
    /// * `aliases` - Further exact-match markers.
    #[must_use]
    pub fn new(code: impl Into<String>, symbol: Option<&str>, aliases: &[&str]) -> Self {
        Self {
            code: code.into(),
            symbol: symbol.map(str::to_owned),
            aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

impl MarkerSource for Commodity {
    fn code(&self) -> &str {
        &self.code
    }

    fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

#[cfg(feature = "ipc")]
impl MarkerSource for bc_ipc::CommodityInfo {
    fn code(&self) -> &str {
        &self.code
    }

    fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

/// Why a marker could not be resolved to a single commodity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "callers across the workspace match every marker error"
)]
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
    /// Whether this entry is the commodity's code: codes match
    /// case-insensitively, symbols and aliases exactly.
    is_code: bool,
}

/// All marker entries for a commodity: code, symbol, and aliases.
fn markers_for<C>(c: &C) -> Vec<MarkerEntry>
where
    C: MarkerSource,
{
    let mut v = vec![MarkerEntry {
        marker: c.code().to_owned(),
        code: c.code().to_owned(),
        is_code: true,
    }];
    if let Some(s) = c.symbol() {
        v.push(MarkerEntry {
            marker: s.to_owned(),
            code: c.code().to_owned(),
            is_code: false,
        });
    }
    for a in c.aliases() {
        v.push(MarkerEntry {
            marker: a.clone(),
            code: c.code().to_owned(),
            is_code: false,
        });
    }
    v
}

/// Returns whether `c` can start a number: a digit, sign, or dot.
fn starts_number(c: char) -> bool {
    c.is_ascii_digit() || c == '-' || c == '+' || c == '.'
}

/// Resolves a bare marker (no digits) to a canonical code.
///
/// Codes match case-insensitively; symbols and aliases match exactly.
///
/// # Arguments
///
/// * `currencies` - The set of known commodities to match against.
/// * `marker` - The marker string to resolve.
///
/// # Returns
///
/// The canonical commodity code.
///
/// # Errors
///
/// Returns [`MarkerError::Missing`] when `marker` is empty,
/// [`MarkerError::Unknown`] when no commodity matches, and
/// [`MarkerError::Ambiguous`] when more than one commodity matches.
pub fn resolve_marker<C>(currencies: &[C], marker: &str) -> Result<String, MarkerError>
where
    C: MarkerSource,
{
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
                hits.push(entry.code);
            }
        }
    }
    match hits.as_slice() {
        [] => Err(MarkerError::Unknown(key.to_owned())),
        [one] => Ok(one.clone()),
        _ => Err(MarkerError::Ambiguous(key.to_owned())),
    }
}

/// Splits a marked amount into `(numeric_text, canonical_code)`.
///
/// Accepts a leading symbol, alias or code (`$100`, `A$100`, `AUD100`), or a
/// whitespace-separated token at either end (`AUD 100`, `100 AUD`). A bare
/// number errors with [`MarkerError::Missing`].
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
/// Returns [`MarkerError::Missing`] when no marker is found,
/// [`MarkerError::Unknown`] when a marker token matches no commodity, and
/// [`MarkerError::Ambiguous`] when a marker token matches more than one.
pub fn split_marked_amount<C>(
    currencies: &[C],
    input: &str,
) -> Result<(String, String), MarkerError>
where
    C: MarkerSource,
{
    let s = input.trim();
    if s.is_empty() {
        return Err(MarkerError::Missing);
    }

    // Longest first, so "A$" beats "$".
    let mut markers: Vec<MarkerEntry> = currencies.iter().flat_map(markers_for).collect();
    markers.sort_by_key(|m| Reverse(m.marker.len()));

    // 1) A whitespace-separated token at either end. A non-numeric token is a
    //    marker attempt, so its error propagates.
    if let Some((head, rest)) = s.split_once(char::is_whitespace)
        && !head.chars().next().is_some_and(starts_number)
    {
        let code = resolve_marker(currencies, head)?;
        return Ok((rest.trim().to_owned(), code));
    }
    if let Some((rest, tail)) = s.rsplit_once(char::is_whitespace)
        && !tail.chars().next().is_some_and(starts_number)
    {
        let code = resolve_marker(currencies, tail)?;
        return Ok((rest.trim().to_owned(), code));
    }

    // 2) A glued leading marker, after an optional sign ("-$50").
    let (sign, body): (&str, &str) = if let Some(rest) = s.strip_prefix('-') {
        ("-", rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        ("+", rest)
    } else {
        ("", s)
    };
    for entry in &markers {
        let stripped = if entry.is_code && entry.marker.chars().all(|c| c.is_ascii_alphabetic()) {
            body.get(..entry.marker.len())
                .filter(|p| p.eq_ignore_ascii_case(&entry.marker))
                .and_then(|_| body.get(entry.marker.len()..))
        } else {
            body.strip_prefix(entry.marker.as_str())
        };
        if let Some(rest) = stripped
            && rest.trim_start().starts_with(starts_number)
        {
            resolve_marker(currencies, &entry.marker)?;
            return Ok((format!("{sign}{}", rest.trim()), entry.code.clone()));
        }
    }

    // 3) A glued trailing alphabetic code ("100AUD").
    let upper = s.to_ascii_uppercase();
    for entry in &markers {
        if entry.marker.chars().all(|c| c.is_ascii_alphabetic())
            && let Some(rest) = upper.strip_suffix(&entry.marker.to_ascii_uppercase())
            && rest
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.')
        {
            resolve_marker(currencies, &entry.marker)?;
            let number = s.get(..rest.len()).unwrap_or_default();
            return Ok((number.trim().to_owned(), entry.code.clone()));
        }
    }

    Err(MarkerError::Missing)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// USD (`$`, `US$`) and AUD (`A$`, `AU$`).
    fn registry() -> Vec<Commodity> {
        vec![
            Commodity::new("USD", Some("$"), &["US$"]),
            Commodity::new("AUD", Some("A$"), &["AU$"]),
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
        assert_eq!(
            split_marked_amount(&registry(), "XYZ 100"),
            Err(MarkerError::Unknown("XYZ".to_owned()))
        );
    }

    #[test]
    fn symbol_exact_match_code_case_insensitive() {
        let reg = vec![Commodity::new("USD", Some("us$"), &[])];
        assert!(
            matches!(
                split_marked_amount(&reg, "US$100"),
                Err(MarkerError::Unknown(_) | MarkerError::Missing)
            ),
            "uppercase symbol variant must not match lowercase symbol"
        );
        assert_eq!(
            split_marked_amount(&reg, "us$100"),
            Ok(("100".to_owned(), "USD".to_owned()))
        );
        assert_eq!(
            split_marked_amount(&reg, "usd 100"),
            Ok(("100".to_owned(), "USD".to_owned()))
        );
        assert_eq!(
            resolve_marker(&reg, "US$"),
            Err(MarkerError::Unknown("US$".to_owned()))
        );
        assert_eq!(resolve_marker(&reg, "us$"), Ok("USD".to_owned()));
        assert_eq!(resolve_marker(&reg, "USD"), Ok("USD".to_owned()));
        assert_eq!(resolve_marker(&reg, "usd"), Ok("USD".to_owned()));
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
            Commodity::new("USD", Some("$"), &[]),
            Commodity::new("AUD", Some("$"), &[]),
        ];
        assert_eq!(
            split_marked_amount(&amb, "$100"),
            Err(MarkerError::Ambiguous("$".to_owned()))
        );
    }
}
