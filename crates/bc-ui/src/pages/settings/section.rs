// Which Settings section shows, and the `?section=` value that selects it.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

/// Which settings section is currently shown in the main area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    /// The read-only configuration panel (financial year, display, data, plugins).
    General,
    /// The editable currency registry.
    Currencies,
    /// The editable backup settings + actions.
    Backup,
    /// The transfer-suggestion review panel.
    Transfers,
}

impl SettingsSection {
    /// Parses a `?section=` query value.
    ///
    /// # Arguments
    ///
    /// * `value` - The query value, such as `"backup"`.
    ///
    /// # Returns
    ///
    /// The named section, or `None` for any other value.
    #[must_use]
    pub(crate) fn from_query(value: &str) -> Option<Self> {
        match value {
            "general" => Some(Self::General),
            "currencies" => Some(Self::Currencies),
            "backup" => Some(Self::Backup),
            "transfers" => Some(Self::Transfers),
            _ => None,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::SettingsSection;

    #[rstest]
    #[case("general", Some(SettingsSection::General))]
    #[case("currencies", Some(SettingsSection::Currencies))]
    #[case("backup", Some(SettingsSection::Backup))]
    #[case("transfers", Some(SettingsSection::Transfers))]
    #[case("Backup", None)]
    #[case("", None)]
    fn from_query_cases(#[case] value: &str, #[case] expected: Option<SettingsSection>) {
        assert_eq!(SettingsSection::from_query(value), expected);
    }
}
