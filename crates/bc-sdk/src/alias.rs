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

    #[test]
    fn unparsable_since_is_skipped() {
        let aliases = [alias("OLD", "NEW", Some("01/01/2024"))];
        assert_eq!(resolve(&aliases, "OLD", &Date::new(2025, 1, 1)), "OLD");
        aliases[0].since_date().expect_err("not YYYY-MM-DD");
    }
}
