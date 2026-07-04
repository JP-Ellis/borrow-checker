//! Source reference (import provenance) domain type.

use jiff::Timestamp;
use jiff::civil::Date;

use crate::AccountId;
use crate::TransactionId;
use crate::money::Amount;

crate::define_id!(SourceRefId, "source_ref");

/// A provenance record tying a transaction to the statement row that produced it.
///
/// A source reference is scoped to the [`AccountId`] whose statement the row came
/// from — the stable, true source — not to the import profile or importer version.
/// Importers sweeping the whole document hierarchy use the [`Self::fingerprint`] plus
/// [`Self::occurrence`] to recognise rows they have already imported.
///
/// Re-exported from the crate root as [`crate::SourceRef`].
///
/// # Example
///
/// ```
/// use bc_models::{Amount, CommodityCode, SourceRef, SourceRefId, TransactionId, AccountId};
/// use jiff::Timestamp;
/// use jiff::civil::date;
/// use rust_decimal::Decimal;
///
/// let sr = SourceRef::builder()
///     .id(SourceRefId::new())
///     .transaction_id(TransactionId::new())
///     .account_id(AccountId::new())
///     .date(date(2025, 6, 27))
///     .narration("SMARTBEAR")
///     .amount(Amount::new(Decimal::from(100), CommodityCode::new("AUD")))
///     .occurrence(0)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(sr.occurrence(), 0);
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SourceRef {
    /// Stable identifier. Assigned by `bc-core` on persistence.
    id: SourceRefId,

    /// The transaction this statement row produced.
    transaction_id: TransactionId,

    /// The account whose statement this row came from (scope of the fingerprint).
    /// Must be one of the transaction's posting accounts.
    account_id: AccountId,

    /// Value date of the statement row.
    date: Date,

    /// Raw imported description/narration for the row.
    #[builder(into)]
    narration: String,

    /// Amount as seen on this account's statement.
    amount: Amount,

    /// Institution-provided reference/txid, if the source supplied one.
    #[builder(required, default = None)]
    reference: Option<String>,

    /// Ordinal among same-day rows sharing an identical fingerprint. Disambiguates
    /// legitimately identical rows (e.g. two identical purchases on one day).
    occurrence: u32,

    /// Timestamp recorded when this reference was first persisted.
    created_at: Timestamp,
}

impl SourceRef {
    /// Returns the source reference ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &SourceRefId {
        &self.id
    }

    /// Returns the owning transaction's ID.
    #[inline]
    #[must_use]
    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    /// Returns the account whose statement produced this row.
    #[inline]
    #[must_use]
    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    /// Returns the value date.
    #[inline]
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Returns the raw narration.
    #[inline]
    #[must_use]
    pub fn narration(&self) -> &str {
        &self.narration
    }

    /// Returns the statement amount.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> &Amount {
        &self.amount
    }

    /// Returns the institution reference, if any.
    #[inline]
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// Returns the occurrence ordinal.
    #[inline]
    #[must_use]
    pub fn occurrence(&self) -> u32 {
        self.occurrence
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns this reference's dedup fingerprint.
    #[inline]
    #[must_use]
    pub fn fingerprint(&self) -> String {
        Self::compute_fingerprint(self.date, &self.narration, &self.amount, self.reference())
    }

    /// Computes the canonical dedup fingerprint from a row's components.
    ///
    /// The components are joined with the ASCII unit separator (`U+001F`), which
    /// never appears in bank statement text, so differing component splits cannot
    /// collide. An absent `reference` renders identically to an empty string.
    ///
    /// # Arguments
    ///
    /// * `date` - The row's value date.
    /// * `narration` - The raw imported description.
    /// * `amount` - The statement amount.
    /// * `reference` - The institution reference, if any.
    ///
    /// # Returns
    ///
    /// A canonical string usable as an exact-match dedup key.
    #[must_use]
    pub fn compute_fingerprint(
        date: Date,
        narration: &str,
        amount: &Amount,
        reference: Option<&str>,
    ) -> String {
        format!(
            "{date}\u{1f}{narration}\u{1f}{value}\u{1f}{commodity}\u{1f}{reference}",
            value = amount.value(),
            commodity = amount.commodity().as_str(),
            reference = reference.unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rust_decimal::Decimal;

    use crate::AccountId;
    use crate::CommodityCode;
    use crate::SourceRef;
    use crate::SourceRefId;
    use crate::TransactionId;
    use crate::money::Amount;

    fn amount(v: i64) -> Amount {
        Amount::new(Decimal::from(v), CommodityCode::new("AUD"))
    }

    #[test]
    fn source_ref_id_has_correct_prefix() {
        let id = SourceRefId::new();
        assert!(id.to_string().starts_with("source_ref_"));
    }

    #[test]
    fn builder_populates_all_fields() {
        let tx = TransactionId::new();
        let acct = AccountId::new();
        let sr = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx.clone())
            .account_id(acct.clone())
            .date(date(2025, 6, 27))
            .narration("SMARTBEAR")
            .amount(amount(100))
            .reference(Some("REF1".to_owned()))
            .occurrence(0)
            .created_at(Timestamp::now())
            .build();

        assert_eq!(sr.transaction_id(), &tx);
        assert_eq!(sr.account_id(), &acct);
        assert_eq!(sr.narration(), "SMARTBEAR");
        assert_eq!(sr.reference(), Some("REF1"));
        assert_eq!(sr.occurrence(), 0);
    }

    #[test]
    fn fingerprint_is_stable_and_component_sensitive() {
        let fp1 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "SMARTBEAR", &amount(100), None);
        let fp2 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "SMARTBEAR", &amount(100), None);
        assert_eq!(fp1, fp2, "same components hash identically");

        let fp3 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "SMARTBEAR", &amount(101), None);
        assert_ne!(fp1, fp3, "differing amount yields a different fingerprint");
    }

    #[test]
    fn absent_and_empty_reference_are_equal() {
        let none = SourceRef::compute_fingerprint(date(2025, 6, 27), "X", &amount(1), None);
        let empty = SourceRef::compute_fingerprint(date(2025, 6, 27), "X", &amount(1), Some(""));
        assert_eq!(none, empty, "absent reference renders same as empty string");
    }

    #[test]
    fn distinct_reference_disambiguates_identical_rows() {
        let a =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "COFFEE", &amount(5), Some("txid-a"));
        let b =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "COFFEE", &amount(5), Some("txid-b"));
        assert_ne!(a, b);
    }
}
