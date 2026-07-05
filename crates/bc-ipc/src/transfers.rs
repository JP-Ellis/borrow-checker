//! Transfer-suggestion DTO for the IPC boundary.

/// A proposed transfer pair surfaced to the UI for one-click confirmation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TransferSuggestion {
    /// The outgoing (debit) leg's transaction ID.
    pub debit: String,
    /// The incoming (credit) leg's transaction ID.
    pub credit: String,
    /// The transfer magnitude.
    pub amount: crate::Amount,
    /// The debit leg's value date (ISO 8601).
    pub date_debit: String,
    /// The credit leg's value date (ISO 8601).
    pub date_credit: String,
}

impl TransferSuggestion {
    /// Creates a new [`TransferSuggestion`] DTO.
    ///
    /// # Arguments
    ///
    /// * `debit` - The outgoing (debit) leg's transaction ID.
    /// * `credit` - The incoming (credit) leg's transaction ID.
    /// * `amount` - The transfer magnitude.
    /// * `date_debit` - The debit leg's value date (ISO 8601).
    /// * `date_credit` - The credit leg's value date (ISO 8601).
    #[must_use]
    #[inline]
    pub fn new(
        debit: impl Into<String>,
        credit: impl Into<String>,
        amount: crate::Amount,
        date_debit: impl Into<String>,
        date_credit: impl Into<String>,
    ) -> Self {
        Self {
            debit: debit.into(),
            credit: credit.into(),
            amount,
            date_debit: date_debit.into(),
            date_credit: date_credit.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::TransferSuggestion;

    #[test]
    fn round_trips_through_json() {
        let s = TransferSuggestion {
            debit: "transaction_a".to_owned(),
            credit: "transaction_b".to_owned(),
            amount: crate::Amount::new(Decimal::new(10000, 2), "AUD"),
            date_debit: "2025-06-26".to_owned(),
            date_credit: "2025-06-27".to_owned(),
        };
        let json = serde_json::to_string(&s).expect("ser");
        let back: TransferSuggestion = serde_json::from_str(&json).expect("de");
        assert_eq!(s, back);
    }
}
