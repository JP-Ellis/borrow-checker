//! Source reference (import provenance) domain type.

use jiff::Timestamp;
use jiff::civil::Date;

use crate::AccountId;
use crate::ImportBatchId;
use crate::PostingId;
use crate::TransactionId;
use crate::money::Amount;

crate::define_id!(SourceRefId, "source_ref");

/// A provenance record tying a single posting (leg) to the statement row that
/// produced it.
///
/// Provenance is per-leg, not per-transaction: a multi-account source transaction
/// yields one reference per posting, each scoped to the [`AccountId`] whose
/// statement that leg's row came from — the stable, true source — not to the
/// import profile or importer version. Importers sweeping the whole document
/// hierarchy use the [`Self::fingerprint`] plus [`Self::occurrence`] to recognise
/// rows they have already imported.
///
/// Re-exported from the crate root as [`crate::SourceRef`].
///
/// # Example
///
/// ```
/// use bc_models::{Amount, CommodityCode, PostingId, SourceRef, SourceRefId, TransactionId, AccountId};
/// use jiff::Timestamp;
/// use jiff::civil::date;
/// use rust_decimal::Decimal;
///
/// let sr = SourceRef::builder()
///     .id(SourceRefId::new())
///     .transaction_id(TransactionId::new())
///     .posting_id(Some(PostingId::new()))
///     .account_id(AccountId::new())
///     .date(date(2025, 6, 27))
///     .narration("ACME")
///     .amount(Some(Amount::new(Decimal::from(100), CommodityCode::new("AUD"))))
///     .occurrence(0)
///     .import_batch_id(None)
///     .owns_posting(false)
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

    /// The specific posting this statement leg produced, or `None` once that
    /// posting has been deleted.
    ///
    /// Provenance is per-leg: a multi-account source transaction yields one
    /// reference per posting, each pointing at the leg it came from. This is
    /// what lets a later import pass attach a leg that was missing from an
    /// earlier one.
    ///
    /// A reference outlives its posting. `None` is a tombstone: the source
    /// document contained this leg and the user has since removed it. The
    /// reference remains history, keeps its occurrence slot, and so stops a
    /// re-import recreating the leg.
    #[builder(required, default = None)]
    posting_id: Option<PostingId>,

    /// The account whose statement this row came from (scope of the fingerprint).
    /// Must be the account of [`Self::posting_id`] specifically, not merely one of
    /// the transaction's other posting accounts.
    account_id: AccountId,

    /// Value date of the statement row.
    date: Date,

    /// Raw imported description/narration for the row.
    #[builder(into)]
    narration: String,

    /// Amount as seen on this account's statement, or `None` for an elided leg.
    ///
    /// An elided leg absorbs the transaction's residual, which is derived rather
    /// than stored, so it has neither a value nor a commodity of its own.
    #[builder(required, default = None)]
    amount: Option<Amount>,

    /// Institution-provided reference/txid, if the source supplied one.
    #[builder(required, default = None)]
    reference: Option<String>,

    /// Ordinal among same-day rows sharing an identical fingerprint. Disambiguates
    /// legitimately identical rows (e.g. two identical purchases on one day).
    occurrence: u32,

    /// The import run that wrote this reference, if it came from an import.
    ///
    /// Discarding that batch deletes every reference carrying its ID.
    #[builder(required, default = None)]
    import_batch_id: Option<ImportBatchId>,

    /// Whether the import that wrote this reference also created the posting it
    /// names.
    ///
    /// `false` when the posting already existed and the import recorded
    /// provenance against it — an *adoption* — and `false` for a reference
    /// attached outside an import, which created nothing. Discarding a batch
    /// deletes the postings that batch created and leaves the rest standing;
    /// which is which cannot be recovered after the fact, so it is recorded
    /// here at attach time.
    ///
    /// Deliberately has no default. Getting it wrong either destroys a posting
    /// the user wrote or strands one the import created, and the compiler
    /// cannot see either, so every caller is made to say which it means.
    owns_posting: bool,

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

    /// Returns the posting this reference points at, or `None` if that posting
    /// has been deleted.
    #[inline]
    #[must_use]
    pub fn posting_id(&self) -> Option<&PostingId> {
        self.posting_id.as_ref()
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

    /// Returns the statement amount, or `None` for an elided leg.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> Option<&Amount> {
        self.amount.as_ref()
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

    /// Returns the import run that wrote this reference, if any.
    #[inline]
    #[must_use]
    pub fn import_batch_id(&self) -> Option<&ImportBatchId> {
        self.import_batch_id.as_ref()
    }

    /// Returns whether the import that wrote this reference created its posting.
    ///
    /// # Returns
    ///
    /// `true` if the posting was inserted by that import, `false` if it already
    /// existed or the reference came from outside an import.
    #[inline]
    #[must_use]
    pub fn owns_posting(&self) -> bool {
        self.owns_posting
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
        Self::compute_fingerprint(self.date, &self.narration, self.amount(), self.reference())
    }

    /// Computes the canonical dedup fingerprint from a leg's components.
    ///
    /// The components are joined with the ASCII unit separator (`U+001F`), which
    /// never appears in bank statement text, so differing component splits
    /// cannot collide. An absent `reference` renders identically to an empty
    /// string, and an absent `amount` — an elided leg — renders as two empty
    /// components.
    ///
    /// # Arguments
    ///
    /// * `date` - The row's value date.
    /// * `narration` - The raw imported description.
    /// * `amount` - The statement amount, or `None` for an elided leg.
    /// * `reference` - The institution reference, if any.
    ///
    /// # Returns
    ///
    /// A canonical string usable as an exact-match dedup key.
    #[must_use]
    pub fn compute_fingerprint(
        date: Date,
        narration: &str,
        amount: Option<&Amount>,
        reference: Option<&str>,
    ) -> String {
        let (value, commodity) = match amount {
            Some(a) => (a.value().to_string(), a.commodity().as_str().to_owned()),
            None => (String::new(), String::new()),
        };
        format!(
            "{date}\u{1f}{narration}\u{1f}{value}\u{1f}{commodity}\u{1f}{reference}",
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
    use crate::PostingId;
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
        let posting = PostingId::new();
        let acct = AccountId::new();
        let sr = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx.clone())
            .posting_id(Some(posting.clone()))
            .account_id(acct.clone())
            .date(date(2025, 6, 27))
            .narration("ACME")
            .amount(Some(amount(100)))
            .reference(Some("REF1".to_owned()))
            .occurrence(0)
            .import_batch_id(None)
            .owns_posting(false)
            .created_at(Timestamp::now())
            .build();

        assert_eq!(sr.transaction_id(), &tx);
        assert_eq!(sr.posting_id(), Some(&posting));
        assert_eq!(sr.account_id(), &acct);
        assert_eq!(sr.narration(), "ACME");
        assert_eq!(sr.reference(), Some("REF1"));
        assert_eq!(sr.occurrence(), 0);
        assert_eq!(sr.import_batch_id(), None);
    }

    #[test]
    fn fingerprint_is_stable_and_component_sensitive() {
        let fp1 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "ACME", Some(&amount(100)), None);
        let fp2 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "ACME", Some(&amount(100)), None);
        assert_eq!(fp1, fp2, "same components hash identically");

        let fp3 =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "ACME", Some(&amount(101)), None);
        assert_ne!(fp1, fp3, "differing amount yields a different fingerprint");
    }

    #[test]
    fn absent_and_empty_reference_are_equal() {
        let none = SourceRef::compute_fingerprint(date(2025, 6, 27), "X", Some(&amount(1)), None);
        let empty =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "X", Some(&amount(1)), Some(""));
        assert_eq!(none, empty, "absent reference renders same as empty string");
    }

    #[test]
    fn distinct_reference_disambiguates_identical_rows() {
        let a = SourceRef::compute_fingerprint(
            date(2025, 6, 27),
            "COFFEE",
            Some(&amount(5)),
            Some("txid-a"),
        );
        let b = SourceRef::compute_fingerprint(
            date(2025, 6, 27),
            "COFFEE",
            Some(&amount(5)),
            Some("txid-b"),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn an_absent_amount_fingerprints_as_an_empty_component() {
        let elided = SourceRef::compute_fingerprint(date(2025, 6, 27), "SPLIT", None, None);
        let concrete =
            SourceRef::compute_fingerprint(date(2025, 6, 27), "SPLIT", Some(&amount(100)), None);
        assert_ne!(
            elided, concrete,
            "an elided leg must not collide with a concrete one"
        );
        assert!(
            elided.contains("\u{1f}\u{1f}"),
            "the absent amount renders as empty components: {elided}"
        );
    }

    #[test]
    fn builder_records_the_posting_id() {
        let posting = PostingId::new();
        let sr = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(TransactionId::new())
            .posting_id(Some(posting.clone()))
            .account_id(AccountId::new())
            .date(date(2025, 6, 27))
            .narration("ACME")
            .amount(Some(amount(100)))
            .reference(None)
            .occurrence(0)
            .import_batch_id(None)
            .owns_posting(false)
            .created_at(Timestamp::now())
            .build();

        assert_eq!(
            sr.posting_id(),
            Some(&posting),
            "provenance points at a specific leg, not just the transaction"
        );
        assert_eq!(sr.amount(), Some(&amount(100)));
    }

    #[test]
    fn builder_records_the_import_batch_id() {
        let batch = crate::ImportBatchId::new();
        let sr = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(TransactionId::new())
            .posting_id(Some(PostingId::new()))
            .account_id(AccountId::new())
            .date(date(2025, 6, 27))
            .narration("ACME")
            .amount(Some(amount(100)))
            .reference(None)
            .occurrence(0)
            .import_batch_id(Some(batch.clone()))
            .owns_posting(true)
            .created_at(Timestamp::now())
            .build();

        assert_eq!(sr.import_batch_id(), Some(&batch));
    }
}
