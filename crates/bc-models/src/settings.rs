//! Global user preference types.

use jiff::civil::Date;

use crate::money::CommodityCode;

/// Application-wide settings stored once per database.
#[expect(
    clippy::module_name_repetitions,
    reason = "GlobalSettings is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GlobalSettings {
    /// Month the financial year starts (1-based; default 7 = July).
    pub financial_year_start_month: u8,
    /// Day of month the financial year starts (1-based; default 1).
    pub financial_year_start_day: u8,
    /// Anchor date for fortnightly budget periods.
    pub fortnightly_anchor: Option<Date>,
    /// Currency used for display normalisation.
    pub display_commodity: CommodityCode,
}

impl GlobalSettings {
    /// Creates a new [`GlobalSettings`] with all fields.
    ///
    /// This constructor is required because the struct is `#[non_exhaustive]`.
    #[inline]
    #[must_use]
    pub fn new(
        financial_year_start_month: u8,
        financial_year_start_day: u8,
        fortnightly_anchor: Option<Date>,
        display_commodity: CommodityCode,
    ) -> Self {
        Self {
            financial_year_start_month,
            financial_year_start_day,
            fortnightly_anchor,
            display_commodity,
        }
    }
}

impl Default for GlobalSettings {
    #[inline]
    fn default() -> Self {
        Self {
            financial_year_start_month: 7,
            financial_year_start_day: 1,
            fortnightly_anchor: None,
            display_commodity: CommodityCode::new("USD"),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn global_settings_default_fy_start() {
        let s = GlobalSettings::default();
        assert_eq!(s.financial_year_start_month, 7);
        assert_eq!(s.financial_year_start_day, 1);
    }
}
