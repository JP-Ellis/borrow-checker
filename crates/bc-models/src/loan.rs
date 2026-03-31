//! Loan terms and amortization schedule domain types.

use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::AccountId;

crate::define_id!(LoanId, "loan");

/// Repayment frequency for a loan account.
///
/// Re-exported from the crate root as [`crate::RepaymentFrequency`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// One payment every 7 days.
    Weekly,
    /// One payment every 14 days.
    Fortnightly,
    /// One payment per calendar month.
    Monthly,
    /// One payment per calendar quarter.
    Quarterly,
}

impl Frequency {
    /// Returns the number of payment periods per year.
    ///
    /// Used to derive the per-period interest rate from the annual rate.
    #[must_use]
    #[inline]
    pub fn periods_per_year(self) -> u32 {
        match self {
            Self::Weekly => 52,
            Self::Fortnightly => 26,
            Self::Monthly => 12,
            Self::Quarterly => 4,
        }
    }
}

/// Loan terms attached to a [`Receivable`](crate::AccountKind::Receivable) account.
///
/// # Example
///
/// ```
/// use bc_models::{AccountId, LoanTerms, RepaymentFrequency};
/// use rust_decimal_macros::dec;
///
/// let terms = LoanTerms::builder()
///     .account_id(AccountId::new())
///     .principal(dec!(100_000))
///     .annual_rate(dec!(0.065))
///     .start_date(jiff::civil::date(2026, 1, 1))
///     .term_months(360u32)
///     .repayment_frequency(RepaymentFrequency::Monthly)
///     .commodity("AUD")
///     .build();
///
/// assert_eq!(terms.term_months(), 360);
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct LoanTerms {
    /// Stable identifier for this loan terms record.
    #[builder(default)]
    id: LoanId,

    /// The [`Receivable`](crate::AccountKind::Receivable) account these terms describe.
    account_id: AccountId,

    /// Original principal amount (in `commodity`).
    principal: Decimal,

    /// Annual interest rate as a fraction, e.g. `0.065` = 6.5 % p.a.
    annual_rate: Decimal,

    /// Date the loan commenced (first payment period starts here).
    start_date: Date,

    /// Total loan term in months.
    term_months: u32,

    /// How often repayments are made.
    repayment_frequency: Frequency,

    /// Currency of this loan (e.g. `"AUD"`).
    #[builder(into)]
    commodity: String,

    /// When this record was first persisted. Defaults to now.
    #[builder(default = jiff::Timestamp::now())]
    created_at: Timestamp,
}

impl LoanTerms {
    /// Returns the loan record's ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &LoanId {
        &self.id
    }

    /// Returns the account this loan is attached to.
    #[inline]
    #[must_use]
    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    /// Returns the original principal.
    #[inline]
    #[must_use]
    pub fn principal(&self) -> Decimal {
        self.principal
    }

    /// Returns the annual interest rate.
    #[inline]
    #[must_use]
    pub fn annual_rate(&self) -> Decimal {
        self.annual_rate
    }

    /// Returns the loan start date.
    #[inline]
    #[must_use]
    pub fn start_date(&self) -> Date {
        self.start_date
    }

    /// Returns the total term in months.
    #[inline]
    #[must_use]
    pub fn term_months(&self) -> u32 {
        self.term_months
    }

    /// Returns the repayment frequency.
    #[inline]
    #[must_use]
    pub fn repayment_frequency(&self) -> Frequency {
        self.repayment_frequency
    }

    /// Returns the commodity code (e.g. `"AUD"`).
    #[inline]
    #[must_use]
    pub fn commodity(&self) -> &str {
        &self.commodity
    }

    /// Returns when this record was created.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

/// A single row in a loan amortization schedule.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct AmortizationRow {
    /// Sequential payment number, starting at 1.
    pub payment_number: u32,
    /// Scheduled payment due date.
    pub date: Date,
    /// Total payment amount (principal + interest).
    pub total_payment: Decimal,
    /// Principal portion of this payment.
    pub principal: Decimal,
    /// Interest portion of this payment.
    pub interest: Decimal,
    /// Remaining principal balance after this payment.
    pub remaining_balance: Decimal,
}

impl AmortizationRow {
    /// Creates a new [`AmortizationRow`] with all fields populated.
    ///
    /// # Arguments
    ///
    /// * `payment_number` - Sequential payment number, starting at 1.
    /// * `date` - Scheduled payment due date.
    /// * `total_payment` - Total payment amount (principal + interest).
    /// * `principal` - Principal portion of this payment.
    /// * `interest` - Interest portion of this payment.
    /// * `remaining_balance` - Remaining principal balance after this payment.
    #[must_use]
    #[inline]
    pub fn new(
        payment_number: u32,
        date: Date,
        total_payment: Decimal,
        principal: Decimal,
        interest: Decimal,
        remaining_balance: Decimal,
    ) -> Self {
        Self {
            payment_number,
            date,
            total_payment,
            principal,
            interest,
            remaining_balance,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn loan_id_has_correct_prefix() {
        assert!(LoanId::new().to_string().starts_with("loan_"));
    }

    #[test]
    fn repayment_frequency_round_trips_via_serde() {
        let freqs = [
            Frequency::Weekly,
            Frequency::Fortnightly,
            Frequency::Monthly,
            Frequency::Quarterly,
        ];
        for f in freqs {
            let json = serde_json::to_string(&f).expect("serialise");
            let back: Frequency = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(f, back);
        }
    }

    #[test]
    fn amortization_row_is_serialisable() {
        use rust_decimal_macros::dec;
        let row = AmortizationRow {
            payment_number: 1,
            date: jiff::civil::date(2026, 1, 1),
            total_payment: dec!(1234.56),
            principal: dec!(900.00),
            interest: dec!(334.56),
            remaining_balance: dec!(99100.00),
        };
        let json = serde_json::to_string(&row).expect("serialise");
        assert!(json.contains("payment_number"));
    }

    #[test]
    fn frequency_periods_per_year() {
        assert_eq!(Frequency::Weekly.periods_per_year(), 52);
        assert_eq!(Frequency::Fortnightly.periods_per_year(), 26);
        assert_eq!(Frequency::Monthly.periods_per_year(), 12);
        assert_eq!(Frequency::Quarterly.periods_per_year(), 4);
    }
}
