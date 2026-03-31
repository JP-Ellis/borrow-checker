//! Asset valuation and depreciation domain types.

use rust_decimal::Decimal;

crate::define_id!(ValuationId, "valuation");

/// The authoritative source of a recorded asset market value.
///
/// Re-exported from the crate root as [`crate::ValuationSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// An estimate provided by the owner.
    ManualEstimate,
    /// A formal appraisal by a qualified valuer or surveyor.
    ProfessionalAppraisal,
    /// A government tax assessment (e.g. council rates notice).
    TaxAssessment,
    /// Market data such as an exchange price or comparable sales data.
    MarketData,
    /// A value agreed between parties (e.g. buy-sell agreement).
    AgreedValue,
}

/// Depreciation method applied to a [`ManualAsset`](crate::AccountKind::ManualAsset) account.
///
/// Re-exported from the crate root as [`crate::DepreciationPolicy`].
///
/// # Example
///
/// ```
/// use bc_models::DepreciationPolicy;
/// use rust_decimal_macros::dec;
///
/// let policy = DepreciationPolicy::StraightLine { annual_rate: dec!(0.25) };
/// let json = serde_json::to_string(&policy).unwrap();
/// let back: DepreciationPolicy = serde_json::from_str(&json).unwrap();
/// assert_eq!(policy, back);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DepreciationPolicy {
    /// No depreciation is applied to this asset.
    None,
    /// Fixed annual depreciation: `acquisition_cost × annual_rate`.
    StraightLine {
        /// Annual rate as a fraction, e.g. `0.25` = 25 % per year.
        annual_rate: Decimal,
    },
    /// Reducing-balance depreciation: `current_book_value × annual_rate`.
    DecliningBalance {
        /// Annual rate as a fraction, e.g. `0.40` = 40 % per year.
        annual_rate: Decimal,
    },
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn valuation_id_has_correct_prefix() {
        assert!(ValuationId::new().to_string().starts_with("valuation_"));
    }

    #[test]
    fn valuation_source_round_trips_via_serde() {
        let sources = [
            Source::ManualEstimate,
            Source::ProfessionalAppraisal,
            Source::TaxAssessment,
            Source::MarketData,
            Source::AgreedValue,
        ];
        for s in sources {
            let json = serde_json::to_string(&s).expect("serialise");
            let back: Source = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(s, back);
        }
    }

    #[test]
    fn depreciation_policy_none_round_trips() {
        let policy = DepreciationPolicy::None;
        let json = serde_json::to_string(&policy).expect("serialise");
        let back: DepreciationPolicy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(policy, back);
    }

    #[test]
    fn depreciation_policy_straight_line_round_trips() {
        use rust_decimal_macros::dec;
        let policy = DepreciationPolicy::StraightLine {
            annual_rate: dec!(0.25),
        };
        let json = serde_json::to_string(&policy).expect("serialise");
        let back: DepreciationPolicy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(policy, back);
    }

    #[test]
    fn depreciation_policy_declining_balance_round_trips() {
        use rust_decimal_macros::dec;
        let policy = DepreciationPolicy::DecliningBalance {
            annual_rate: dec!(0.40),
        };
        let json = serde_json::to_string(&policy).expect("serialise");
        let back: DepreciationPolicy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(policy, back);
    }
}
