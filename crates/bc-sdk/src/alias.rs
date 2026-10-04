//! Commodity code renaming applied by an importer before codes cross the ABI.
//!
//! The host resolves every code exactly as an importer emits it, so a source
//! whose codes collide with another market's renames them here first.

use std::collections::BTreeMap;

use crate::Date;

/// One rename applied to commodity codes as an importer emits them.
///
/// ```json
/// { "from": "$",   "to": "AUD" }
/// { "from": "FOO", "to": "BAR", "since": "2024-01-01" }
/// ```
///
/// A code with no entry passes through unchanged. Aliases apply in one pass:
/// `A → B` and `B → C` do not turn `A` into `C`.
#[expect(
    clippy::module_name_repetitions,
    reason = "CommodityAlias is the SDK's public name for this type, re-exported at the crate root"
)]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommodityAlias {
    /// The code as the source spells it.
    pub from: String,
    /// The code to emit.
    pub to: String,
    /// First date this entry applies from, `YYYY-MM-DD`, inclusive. Absent
    /// means every date. Among matching entries the latest `since` at or
    /// before the date wins; an undated entry ranks below every dated one.
    #[serde(default)]
    pub since: Option<String>,
    /// Keys stated on this entry that the SDK does not read, kept so an
    /// importer can warn about them by name.
    #[serde(flatten, default, skip_serializing)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl CommodityAlias {
    /// Parses `since`.
    ///
    /// # Errors
    ///
    /// Returns the parse failure when `since` is not `YYYY-MM-DD`.
    #[inline]
    pub fn since_date(&self) -> Result<Option<Date>, String> {
        self.since.as_deref().map(Date::parse_iso).transpose()
    }
}

/// Describes every malformed entry in `aliases`, for an importer's `validate`.
///
/// Each problem names its entry as `commodity_aliases[i]`, the field every
/// first-party importer stores its aliases under.
///
/// # Arguments
///
/// * `aliases` - The configured entries, in profile order.
///
/// # Returns
///
/// One message per problem, empty when every entry is usable: an empty `from`
/// or `to`, a `since` that is not `YYYY-MM-DD`, or two entries mapping one
/// code from the same date.
#[must_use]
#[inline]
pub fn problems(aliases: &[CommodityAlias]) -> Vec<String> {
    let mut problems = Vec::new();
    // A malformed `since` is reported on its own; it takes no part in the
    // same-date check, so it is held apart from a genuinely undated entry.
    let mut since_dates: Vec<Option<Option<Date>>> = Vec::with_capacity(aliases.len());
    for (index, entry) in aliases.iter().enumerate() {
        if entry.from.trim().is_empty() {
            problems.push(format!(
                "commodity_aliases[{index}].from is empty, so it can match no code."
            ));
        }
        if entry.to.trim().is_empty() {
            problems.push(format!(
                "commodity_aliases[{index}].to is empty, so a matched code would post in no \
                 commodity."
            ));
        }
        match entry.since_date() {
            Ok(since) => since_dates.push(Some(since)),
            Err(detail) => {
                problems.push(format!(
                    "commodity_aliases[{index}].since is not a YYYY-MM-DD date: {detail}"
                ));
                since_dates.push(None);
            }
        }
    }
    let parsed: Vec<_> = aliases.iter().zip(&since_dates).collect();
    for (later, &(entry, since)) in parsed.iter().enumerate() {
        for (earlier, &(prior, prior_since)) in parsed.iter().enumerate().take(later) {
            if prior.from == entry.from && since.is_some() && prior_since == since {
                problems.push(format!(
                    "commodity_aliases[{earlier}] and commodity_aliases[{later}] both map {:?} \
                     from the same date, so neither can win.",
                    entry.from
                ));
            }
        }
    }
    problems
}

/// Warns about each entry that is usable but probably not what was meant.
///
/// A key the SDK does not read is dropped, so a misspelt `since` leaves the
/// entry undated and applies it to every date. An entry mapping a code to
/// itself does nothing.
///
/// # Arguments
///
/// * `aliases` - The configured entries, in profile order.
#[inline]
pub fn warn_advisories(aliases: &[CommodityAlias]) {
    for (index, entry) in aliases.iter().enumerate() {
        for key in entry.extra.keys() {
            let path = format!("commodity_aliases[{index}].{key}");
            crate::warn!(
                "unknown commodity alias key";
                key = path,
                detail = format!(
                    "{path} is not a key this importer reads, and is ignored. Check it for a \
                     typo against from, to and since."
                )
            );
        }
        if entry.from == entry.to {
            crate::warn!(
                "commodity alias maps a code to itself";
                field = format!("commodity_aliases[{index}]"),
                code = entry.from.clone()
            );
        }
    }
}

/// Applies `aliases` to one code on one date.
///
/// An entry whose `since` does not parse is skipped; an importer's `validate`
/// should reject it before any row is read.
#[must_use]
#[inline]
pub fn resolve<'a>(aliases: &'a [CommodityAlias], code: &'a str, on: &Date) -> &'a str {
    aliases
        .iter()
        .filter(|entry| entry.from == code)
        .filter_map(|entry| match entry.since_date() {
            Ok(Some(since)) if since <= *on => Some((Some(since), entry)),
            Ok(Some(_)) | Err(_) => None,
            Ok(None) => Some((None, entry)),
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map_or(code, |(_, entry)| entry.to.as_str())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn alias(from: &str, to: &str, since: Option<&str>) -> CommodityAlias {
        CommodityAlias {
            from: from.to_owned(),
            to: to.to_owned(),
            since: since.map(str::to_owned),
            extra: BTreeMap::new(),
        }
    }

    #[rstest]
    #[case::no_entry("XTS", (2024_i32, 6_u8, 1_u8), "XTS")]
    #[case::undated("A$", (2024_i32, 6_u8, 1_u8), "AUD")]
    #[case::before_since("OLD", (2023_i32, 12_u8, 31_u8), "UNDATED")]
    #[case::on_since("OLD", (2024_i32, 1_u8, 1_u8), "NEW")]
    #[case::latest_dated_wins("OLD", (2025_i32, 6_u8, 1_u8), "NEWER")]
    #[case::single_pass("NEW", (2025_i32, 6_u8, 1_u8), "NEW")]
    fn resolves(#[case] code: &str, #[case] when: (i32, u8, u8), #[case] expected: &str) {
        let aliases = [
            alias("A$", "AUD", None),
            alias("OLD", "UNDATED", None),
            alias("OLD", "NEW", Some("2024-01-01")),
            alias("OLD", "NEWER", Some("2025-01-01")),
        ];
        let on = Date::new(when.0, when.1, when.2);
        assert_eq!(resolve(&aliases, code, &on), expected);
    }

    #[test]
    fn undated_ranks_below_dated() {
        let aliases = [
            alias("OLD", "UNDATED", None),
            alias("OLD", "NEW", Some("2024-01-01")),
        ];
        assert_eq!(resolve(&aliases, "OLD", &Date::new(2023, 1, 1)), "UNDATED");
        assert_eq!(resolve(&aliases, "OLD", &Date::new(2024, 1, 1)), "NEW");
    }

    #[test]
    fn before_every_dated_entry_the_code_passes_through() {
        let aliases = [alias("OLD", "NEW", Some("2024-01-01"))];
        assert_eq!(resolve(&aliases, "OLD", &Date::new(2023, 1, 1)), "OLD");
    }

    #[rstest]
    #[case::empty_from(&[alias(" ", "AUD", None)], &["commodity_aliases[0].from is empty"])]
    #[case::empty_to(&[alias("$", "", None)], &["commodity_aliases[0].to is empty"])]
    #[case::bad_since(&[alias("$", "AUD", Some("01/02/2024"))], &["commodity_aliases[0].since"])]
    #[case::same_date(
        &[alias("FOO", "BAR", None), alias("FOO", "BAZ", None)],
        &["commodity_aliases[0] and commodity_aliases[1] both map \"FOO\""]
    )]
    // A malformed `since` must not also read as undated and collide.
    #[case::bad_since_not_paired(
        &[alias("FOO", "BAR", Some("01/02/2024")), alias("FOO", "BAZ", None)],
        &["commodity_aliases[0].since"]
    )]
    #[case::timeline(&[alias("FOO", "BAR", None), alias("FOO", "BAZ", Some("2024-01-01"))], &[])]
    #[case::identity(&[alias("AUD", "AUD", None)], &[])]
    fn problems_name_each_malformed_entry(
        #[case] aliases: &[CommodityAlias],
        #[case] expected: &[&str],
    ) {
        let found = problems(aliases);
        assert_eq!(found.len(), expected.len(), "{found:?}");
        for (problem, fragment) in found.iter().zip(expected) {
            assert!(problem.contains(fragment), "{problem}");
        }
    }

    #[test]
    fn unparsable_since_is_skipped() {
        let aliases = [alias("OLD", "NEW", Some("01/01/2024"))];
        assert_eq!(resolve(&aliases, "OLD", &Date::new(2025, 1, 1)), "OLD");
        aliases[0].since_date().expect_err("not YYYY-MM-DD");
    }
}
