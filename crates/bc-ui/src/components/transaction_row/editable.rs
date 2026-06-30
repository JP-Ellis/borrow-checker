// Editor-friendly working-buffer model for the editable transaction view.
//
// Mirrors the [`bc_ipc::Transaction`] shape but uses parse-in-progress string
// fields so an in-flight edit (a half-typed amount, a malformed date) is always
// representable. Conversion back to a [`bc_ipc::EditTransaction`] (with real
// parsing and validation) lives in [`Self::to_edit_transaction`].
//
// Public items are used by the UI layer and tested extensively. When compiling
// to wasm32-unknown-unknown, tests are excluded, making items appear unused to
// clippy on that target only.

use core::fmt;

use bc_ipc::Amount;
use bc_ipc::CommodityInfo;
use bc_ipc::EditPosting;
use bc_ipc::EditTransaction;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use rust_decimal::Decimal;

use crate::components::transaction_row::currency::MarkerError;
use crate::components::transaction_row::currency::split_marked_amount;

/// A single posting in the working buffer.
///
/// `amount` is the raw text the user is editing; an empty (whitespace-only)
/// value marks an elided leg whose amount is inferred to balance.
#[derive(Clone, Debug, PartialEq)]
#[expect(clippy::module_name_repetitions, reason = "name is correct per spec")]
pub struct EditablePosting {
    /// Existing posting ID, or `None` for a newly added leg.
    pub id: Option<String>,
    /// Stable per-row identity for keyed rendering; not persisted.
    pub uid: u64,
    /// Account this posting hits.
    pub account_id: String,
    /// Account display name (for the read view and picker label).
    pub account_name: String,
    /// Raw amount text; empty means elided.
    pub amount: String,
    /// ISO currency code for `amount`.
    pub currency: String,
    /// Free-text note; empty means none.
    pub note: String,
    /// Resolved tag colon-paths attached to this posting (e.g. `"person:josh"`).
    pub tags: Vec<String>,
    /// Accrual spread start date, if set.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date, if set.
    pub spread_until: Option<jiff::civil::Date>,
}

impl EditablePosting {
    /// Builds an [`EditablePosting`] from a read-model [`Posting`].
    ///
    /// # Arguments
    ///
    /// * `p` - The source posting.
    /// * `uid` - Stable per-row identity for keyed rendering.
    ///
    /// # Returns
    ///
    /// The editor-friendly posting; elided legs map to an empty `amount`.
    #[must_use]
    pub fn from_posting(p: &Posting, uid: u64) -> Self {
        Self {
            id: Some(p.id.clone()),
            uid,
            account_id: p.account.id.clone(),
            account_name: p.account.name.clone(),
            amount: p
                .amount
                .as_ref()
                .map_or_else(String::new, |a| format!("{} {}", a.currency_code, a.value)),
            currency: p
                .amount
                .as_ref()
                .map_or_else(String::new, |a| a.currency_code.clone()),
            note: p.note.clone().unwrap_or_default(),
            tags: p.tags.clone(),
            spread_from: p.spread_from,
            spread_until: p.spread_until,
        }
    }

    /// Returns whether this posting's amount is elided (inferred to balance).
    ///
    /// # Returns
    ///
    /// `true` when the raw amount text is empty or whitespace.
    #[must_use]
    pub fn is_elided(&self) -> bool {
        self.amount.trim().is_empty()
    }
}

/// The full working buffer for an in-progress edit.
#[derive(Clone, Debug, PartialEq)]
#[expect(clippy::module_name_repetitions, reason = "name is correct per spec")]
pub struct EditableTransaction {
    /// Stable transaction ID (immutable).
    pub id: String,
    /// Raw date text (`YYYY-MM-DD`).
    pub date: String,
    /// Payee display name.
    pub payee: String,
    /// Free-text description.
    pub description: String,
    /// User's free-text note; empty means none.
    pub note: String,
    /// Reconciliation status (immutable in this view; echoed back unchanged).
    pub reconciliation: Reconciliation,
    /// Transaction-level tags.
    pub tags: Vec<String>,
    /// Extra named dates as (label, raw `YYYY-MM-DD` text) pairs; empty labels allowed.
    pub extra_dates: Vec<(String, String)>,
    /// All postings in display order.
    pub postings: Vec<EditablePosting>,
}

impl EditableTransaction {
    /// Builds an [`EditableTransaction`] from a read-model [`Transaction`].
    ///
    /// # Arguments
    ///
    /// * `tx` - The source transaction.
    ///
    /// # Returns
    ///
    /// The working buffer seeded from `tx`.
    #[must_use]
    pub fn from_transaction(tx: &Transaction) -> Self {
        Self {
            id: tx.id.clone(),
            date: tx.date.to_string(),
            payee: tx.payee.clone(),
            description: tx.description.clone(),
            note: tx.note.clone().unwrap_or_default(),
            reconciliation: tx.reconciliation,
            tags: tx.tags.clone(),
            extra_dates: tx
                .extra_dates
                .iter()
                .map(|(label, d)| (label.clone(), d.to_string()))
                .collect(),
            postings: tx
                .postings
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    EditablePosting::from_posting(p, u64::try_from(i).unwrap_or(u64::MAX))
                })
                .collect(),
        }
    }

    /// Returns the currency of the first posting carrying a concrete amount.
    ///
    /// Used to seed the currency of newly added legs.
    ///
    /// # Returns
    ///
    /// The first present-amount currency code, or an empty string if none.
    #[must_use]
    pub fn default_currency(&self) -> String {
        self.postings
            .iter()
            .find(|p| !p.is_elided())
            .map(|p| p.currency.clone())
            .unwrap_or_default()
    }

    /// Appends a blank posting seeded with the buffer's default currency and a
    /// fresh `uid` (max existing + 1), returning the new uid.
    ///
    /// # Returns
    ///
    /// The `uid` assigned to the new posting.
    pub fn push_blank_posting(&mut self) -> u64 {
        let uid = self
            .postings
            .iter()
            .map(|p| p.uid)
            .max()
            .map_or(0, |m| m.saturating_add(1));
        let currency = self.default_currency();
        self.postings.push(EditablePosting {
            id: None,
            uid,
            account_id: String::new(),
            account_name: String::new(),
            amount: String::new(),
            currency,
            note: String::new(),
            tags: vec![],
            spread_from: None,
            spread_until: None,
        });
        uid
    }

    /// Serialises the working buffer into a [`EditTransaction`] for submission.
    ///
    /// # Arguments
    ///
    /// * `currencies` - The set of known commodities used to resolve amount markers.
    ///
    /// # Returns
    ///
    /// The desired transaction state.
    ///
    /// # Errors
    ///
    /// Returns [`EditError`] when the date or an amount fails to parse (including
    /// when the amount marker is missing or unknown), a posting has no account,
    /// or more than one leg is elided. An unbalanced (but otherwise representable)
    /// transaction is **not** an error.
    pub fn to_edit_transaction(
        &self,
        currencies: &[CommodityInfo],
    ) -> Result<EditTransaction, EditError> {
        let date = self
            .date
            .trim()
            .parse::<jiff::civil::Date>()
            .map_err(|e| EditError::Date(e.to_string()))?;

        let mut elided = 0_usize;
        let mut postings = Vec::with_capacity(self.postings.len());
        for (index, p) in self.postings.iter().enumerate() {
            if p.account_id.trim().is_empty() {
                return Err(EditError::MissingAccount { index });
            }
            let amount = if p.is_elided() {
                elided = elided.saturating_add(1);
                None
            } else {
                let (value, code) = parse_amount(currencies, &p.amount)
                    .map_err(|message| EditError::Amount { index, message })?;
                Some(Amount::new(value, code))
            };
            postings.push(EditPosting::new(
                p.id.clone(),
                p.account_id.clone(),
                amount,
                non_empty(&p.note),
                p.tags.clone(),
                p.spread_from,
                p.spread_until,
            ));
        }
        if elided >= 2 {
            return Err(EditError::Ambiguous);
        }

        let mut extra_dates = Vec::with_capacity(self.extra_dates.len());
        for (index, (label, raw)) in self.extra_dates.iter().enumerate() {
            let trimmed = raw.trim();
            // Prune not-yet-filled rows: clicking "+ date" inserts a blank row,
            // and saving without filling it must not block the save. A non-empty
            // but unparsable date still errors below.
            if trimmed.is_empty() {
                continue;
            }
            let parsed =
                trimmed
                    .parse::<jiff::civil::Date>()
                    .map_err(|e| EditError::ExtraDate {
                        index,
                        message: e.to_string(),
                    })?;
            extra_dates.push((label.clone(), parsed));
        }

        Ok(EditTransaction::new(
            self.id.clone(),
            date,
            self.payee.clone(),
            self.description.clone(),
            non_empty(&self.note),
            self.reconciliation,
            self.tags.clone(),
            postings,
            extra_dates,
        ))
    }
}

/// Parses a marked amount string into `(value, canonical_code)`.
///
/// Requires a resolvable currency marker (`$100`, `AUD 100`, `100 AUD`); a bare
/// number is an error. The numeric remainder may carry comma/space grouping.
///
/// # Arguments
///
/// * `currencies` - The set of known commodities to match against.
/// * `input` - The raw amount text.
///
/// # Returns
///
/// A `(value, canonical_code)` pair on success.
///
/// # Errors
///
/// Returns a human-readable message when the marker is missing/unknown/ambiguous
/// or the numeric part does not parse.
pub fn parse_amount(
    currencies: &[CommodityInfo],
    input: &str,
) -> Result<(Decimal, String), String> {
    let (number, code) = split_marked_amount(currencies, input).map_err(|e| match e {
        MarkerError::Missing => "amount needs a currency (e.g. A$100)".to_owned(),
        MarkerError::Unknown(m) => format!("unknown currency '{m}'"),
        MarkerError::Ambiguous(m) => format!("ambiguous currency '{m}'"),
    })?;
    let cleaned: String = number
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    if cleaned.is_empty() {
        return Err("empty amount".to_owned());
    }
    let value = cleaned.parse::<Decimal>().map_err(|e| e.to_string())?;
    Ok((value, code))
}

/// Parses a comma-separated tag buffer into a list of trimmed, non-empty tags.
///
/// Test-only helper.
#[cfg(test)]
#[must_use]
fn parse_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// A hard error that prevents serialising the working buffer.
///
/// Only the genuinely unrepresentable is an error; an unbalanced transaction is
/// not (it commits with a warning).
#[derive(Clone, Debug, PartialEq)]
pub enum EditError {
    /// The transaction date does not parse.
    Date(String),
    /// A posting amount does not parse.
    Amount {
        /// Index of the offending posting.
        index: usize,
        /// Parser message.
        message: String,
    },
    /// A posting has no account selected.
    MissingAccount {
        /// Index of the offending posting.
        index: usize,
    },
    /// A present-amount posting has no currency.
    ///
    /// Kept for API compatibility; superseded by the marker requirement in
    /// [`parse_amount`] which errors before this variant can be constructed.
    #[expect(
        dead_code,
        reason = "marker requirement supersedes this path; kept for API safety"
    )]
    MissingCurrency {
        /// Index of the offending posting.
        index: usize,
    },
    /// More than one elided leg — the balancing remainder is ambiguous.
    Ambiguous,
    /// An extra date does not parse.
    ExtraDate {
        /// Index of the offending extra date.
        index: usize,
        /// Parser message.
        message: String,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(m) => write!(f, "invalid date: {m}"),
            Self::Amount { index, message } => {
                write!(
                    f,
                    "invalid amount on posting {}: {message}",
                    index.saturating_add(1)
                )
            }
            Self::MissingAccount { index } => {
                write!(f, "posting {} has no account", index.saturating_add(1))
            }
            Self::MissingCurrency { index } => {
                write!(f, "posting {} has no currency", index.saturating_add(1))
            }
            Self::Ambiguous => write!(f, "more than one leg has a blank amount"),
            Self::ExtraDate { index, message } => {
                write!(
                    f,
                    "invalid extra date {}: {message}",
                    index.saturating_add(1)
                )
            }
        }
    }
}

/// Converts an empty-or-whitespace string to `None`, else `Some(trimmed-owned)`.
fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// The balance state of a working buffer, for the live balance indicator.
#[derive(Clone, Debug, PartialEq)]
pub enum BalanceState {
    /// All concrete legs net to zero.
    Balanced,
    /// Exactly one elided leg; `remainder` is the value it will infer.
    Inferred {
        /// The amount the elided leg will take to balance.
        remainder: Decimal,
        /// Currency of the inferred remainder.
        currency: String,
    },
    /// All amounts present but they do not net to zero (commits with a warning).
    Unbalanced {
        /// The non-zero net (positive means a surplus).
        delta: Decimal,
        /// Currency of the delta.
        currency: String,
    },
    /// More than one elided leg — inference is ambiguous (hard error).
    Ambiguous,
    /// At least one concrete amount failed to parse (hard error).
    Invalid,
    /// No concrete amounts to balance against.
    Empty,
}

/// Derives the [`BalanceState`] of a working buffer.
///
/// Mirrors `bc_models::Transaction::balanced` semantics: two-or-more elided legs
/// are ambiguous; a single elided leg infers the remainder; otherwise the
/// concrete legs must net to zero. Amounts are summed using the first present
/// currency (mixed-currency transactions show the raw net).
///
/// # Arguments
///
/// * `working` - The working buffer.
/// * `currencies` - The set of known commodities used to resolve amount markers.
///
/// # Returns
///
/// The derived balance state.
#[must_use]
pub fn derive_balance(working: &EditableTransaction, currencies: &[CommodityInfo]) -> BalanceState {
    let elided = working.postings.iter().filter(|p| p.is_elided()).count();
    if elided >= 2 {
        return BalanceState::Ambiguous;
    }
    let mut total = Decimal::ZERO;
    let mut currency = String::new();
    let mut any = false;
    for p in working.postings.iter().filter(|p| !p.is_elided()) {
        match parse_amount(currencies, &p.amount) {
            Ok((v, code)) => {
                if currency.is_empty() {
                    currency = code;
                }
                total = total.saturating_add(v);
                any = true;
            }
            Err(_) => return BalanceState::Invalid,
        }
    }
    if !any {
        return BalanceState::Empty;
    }
    if elided == 1 {
        return BalanceState::Inferred {
            remainder: Decimal::ZERO.saturating_sub(total),
            currency,
        };
    }
    if total.is_zero() {
        BalanceState::Balanced
    } else {
        BalanceState::Unbalanced {
            delta: total,
            currency,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::CommodityInfo;
    use bc_ipc::Posting;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rust_decimal::Decimal;

    use super::BalanceState;
    use super::EditError;
    use super::EditablePosting;
    use super::EditableTransaction;
    use super::derive_balance;
    use super::parse_amount;
    use super::parse_tags;

    fn two_balanced_postings() -> Vec<Posting> {
        vec![
            Posting::new(
                "p1",
                AccountRef::new("checking", "Checking"),
                Some(Amount::new(Decimal::new(-8_420, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
            Posting::new(
                "p2",
                AccountRef::new("groceries", "Groceries"),
                Some(Amount::new(Decimal::new(8_420, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
        ]
    }

    /// A one-entry registry containing AUD with symbol "A$".
    fn registry() -> Vec<CommodityInfo> {
        vec![CommodityInfo::new(
            "c2",
            "AUD",
            Some("A$".to_owned()),
            vec![],
        )]
    }

    fn sample_tx() -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "weekly shop",
            Some("remember"),
            vec![("cleared".to_owned(), Date::constant(2026, 5, 1))],
            Reconciliation::Unreconciled,
            vec!["work".to_owned()],
            vec![
                Posting::new(
                    "p-1",
                    AccountRef::new("acct-checking", "Assets :: Checking"),
                    Some(Amount::new(Decimal::new(-8_420, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                Posting::new(
                    "p-2",
                    AccountRef::new("acct-groceries", "Expenses :: Groceries"),
                    None,
                    Some("split"),
                    vec!["tag-x".to_owned()],
                    None,
                    None,
                ),
            ],
            vec![],
        )
    }

    /// A two-posting transaction with concrete AUD amounts on both legs.
    fn sample_two_posting_tx() -> Transaction {
        Transaction::new(
            "tx-2",
            Date::constant(2026, 4, 30),
            "Coles",
            "weekly shop",
            None::<&str>,
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            vec![
                Posting::new(
                    "p-1",
                    AccountRef::new("acct-checking", "Assets :: Checking"),
                    Some(Amount::new(Decimal::new(-8_420, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                Posting::new(
                    "p-2",
                    AccountRef::new("acct-groceries", "Expenses :: Groceries"),
                    Some(Amount::new(Decimal::new(8_420, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
            ],
            vec![],
        )
    }

    #[test]
    fn from_transaction_maps_scalars_and_postings() {
        let e = EditableTransaction::from_transaction(&sample_tx());
        assert_eq!(e.id, "tx-1");
        assert_eq!(e.date, "2026-04-30");
        assert_eq!(e.payee, "Coles");
        assert_eq!(e.description, "weekly shop");
        assert_eq!(e.note, "remember");
        assert_eq!(e.tags, vec!["work".to_owned()]);
        assert_eq!(e.postings.len(), 2);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn present_amount_posting_round_trips_fields() {
        let e = EditableTransaction::from_transaction(&sample_tx());
        let p = &e.postings[0];
        assert_eq!(p.id.as_deref(), Some("p-1"));
        assert_eq!(p.account_id, "acct-checking");
        assert_eq!(p.account_name, "Assets :: Checking");
        assert_eq!(p.amount, "AUD -84.20");
        assert_eq!(p.currency, "AUD");
        assert!(!p.is_elided());
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn elided_posting_has_blank_amount() {
        let e = EditableTransaction::from_transaction(&sample_tx());
        let p = &e.postings[1];
        assert_eq!(p.amount, "");
        assert!(p.is_elided());
        assert_eq!(p.note, "split");
        assert_eq!(p.tags, vec!["tag-x".to_owned()]);
    }

    #[test]
    fn default_currency_is_first_present() {
        let e = EditableTransaction::from_transaction(&sample_tx());
        assert_eq!(e.default_currency(), "AUD");
    }

    #[test]
    fn from_posting_builds_editable_posting() {
        let p = Posting::new(
            "p-9",
            AccountRef::new("a", "A :: B"),
            Some(Amount::new(Decimal::new(1_250, 2), "USD")),
            None::<&str>,
            vec![],
            Some(Date::constant(2026, 1, 1)),
            Some(Date::constant(2026, 1, 31)),
        );
        let ep = EditablePosting::from_posting(&p, 0);
        assert_eq!(ep.amount, "USD 12.50");
        assert_eq!(ep.currency, "USD");
        assert_eq!(ep.spread_from, Some(Date::constant(2026, 1, 1)));
        assert_eq!(ep.spread_until, Some(Date::constant(2026, 1, 31)));
    }

    fn ep(amount: &str, currency: &str) -> EditablePosting {
        EditablePosting {
            id: Some("p".to_owned()),
            uid: 0,
            account_id: "a".to_owned(),
            account_name: "A".to_owned(),
            amount: amount.to_owned(),
            currency: currency.to_owned(),
            note: String::new(),
            tags: vec![],
            spread_from: None,
            spread_until: None,
        }
    }

    fn et(postings: Vec<EditablePosting>) -> EditableTransaction {
        EditableTransaction {
            id: "tx".to_owned(),
            date: "2026-04-30".to_owned(),
            payee: "P".to_owned(),
            description: String::new(),
            note: String::new(),
            reconciliation: Reconciliation::Unreconciled,
            tags: vec![],
            extra_dates: vec![],
            postings,
        }
    }

    #[test]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "unwrap_err is banned in tests by config; is_err is the correct check here"
    )]
    fn parse_amount_handles_sign_commas_spaces() {
        assert_eq!(
            parse_amount(&registry(), "AUD -1,234.50").map(|(v, _)| v),
            Ok(Decimal::new(-123_450, 2))
        );
        assert_eq!(
            parse_amount(&registry(), "AUD 8420.00").map(|(v, _)| v),
            Ok(Decimal::new(842_000, 2))
        );
        assert!(parse_amount(&registry(), "").is_err());
        assert!(parse_amount(&registry(), "abc").is_err());
    }

    #[test]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "is_err is the correct check for the missing-marker case"
    )]
    fn parse_amount_requires_marker() {
        assert!(parse_amount(&registry(), "100").is_err());
        assert_eq!(
            parse_amount(&registry(), "A$100"),
            Ok((rust_decimal::Decimal::new(100, 0), "AUD".to_owned()))
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "is_ok is the correct check for the round-trip case"
    )]
    fn from_transaction_seeds_marked_amount() {
        let t = sample_two_posting_tx();
        let e = EditableTransaction::from_transaction(&t);
        assert!(parse_amount(&registry(), &e.postings[0].amount).is_ok());
    }

    #[test]
    fn parse_tags_splits_and_trims() {
        assert_eq!(
            parse_tags("work,  income "),
            vec!["work".to_owned(), "income".to_owned()]
        );
        assert_eq!(parse_tags("  "), Vec::<String>::new());
    }

    #[test]
    fn balance_zero_sum_is_balanced() {
        let s = derive_balance(
            &et(vec![ep("AUD -84.20", "AUD"), ep("AUD 84.20", "AUD")]),
            &registry(),
        );
        assert_eq!(s, BalanceState::Balanced);
    }

    #[test]
    fn balance_single_elided_infers_remainder() {
        let s = derive_balance(
            &et(vec![ep("AUD -84.20", "AUD"), ep("", "AUD")]),
            &registry(),
        );
        assert_eq!(
            s,
            BalanceState::Inferred {
                remainder: Decimal::new(84_20, 2),
                currency: "AUD".to_owned()
            }
        );
    }

    #[test]
    fn balance_two_elided_is_ambiguous() {
        let s = derive_balance(&et(vec![ep("", "AUD"), ep("", "AUD")]), &registry());
        assert_eq!(s, BalanceState::Ambiguous);
    }

    #[test]
    fn balance_nonzero_sum_is_unbalanced() {
        let s = derive_balance(
            &et(vec![ep("AUD -84.20", "AUD"), ep("AUD 80.00", "AUD")]),
            &registry(),
        );
        assert_eq!(
            s,
            BalanceState::Unbalanced {
                delta: Decimal::new(-4_20, 2),
                currency: "AUD".to_owned()
            }
        );
    }

    #[test]
    fn balance_unparsable_amount_is_invalid() {
        let s = derive_balance(
            &et(vec![ep("xx", "AUD"), ep("AUD 1.00", "AUD")]),
            &registry(),
        );
        assert_eq!(s, BalanceState::Invalid);
    }

    #[test]
    fn balance_no_concrete_amounts_is_empty() {
        let s = derive_balance(&et(vec![ep("", "AUD")]), &registry());
        assert_eq!(s, BalanceState::Empty);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn to_edit_maps_present_and_elided() {
        let w = et(vec![ep("AUD -84.20", "AUD"), ep("", "AUD")]);
        let out = w.to_edit_transaction(&registry()).expect("valid");
        assert_eq!(out.id, "tx");
        assert_eq!(out.date, jiff::civil::Date::constant(2026, 4, 30));
        assert_eq!(out.postings.len(), 2);
        assert_eq!(
            out.postings[0].amount,
            Some(Amount::new(Decimal::new(-84_20, 2), "AUD"))
        );
        assert_eq!(out.postings[1].amount, None);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn to_edit_preserves_existing_ids_and_new_leg_none() {
        let mut w = et(vec![ep("AUD -10.00", "AUD"), ep("AUD 10.00", "AUD")]);
        w.postings[1].id = None;
        let out = w.to_edit_transaction(&registry()).expect("valid");
        assert_eq!(out.postings[0].id.as_deref(), Some("p"));
        assert_eq!(out.postings[1].id, None);
    }

    #[test]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "unwrap_err is banned in tests by config; is_ok is the correct check here"
    )]
    fn to_edit_unbalanced_still_succeeds() {
        let w = et(vec![ep("AUD -84.20", "AUD"), ep("AUD 1.00", "AUD")]);
        assert!(w.to_edit_transaction(&registry()).is_ok());
    }

    #[test]
    fn to_edit_bad_date_errors() {
        let mut w = et(vec![ep("AUD -1.00", "AUD"), ep("AUD 1.00", "AUD")]);
        w.date = "not-a-date".to_owned();
        assert!(matches!(
            w.to_edit_transaction(&registry()),
            Err(EditError::Date(_))
        ));
    }

    #[test]
    fn to_edit_two_elided_errors() {
        let w = et(vec![ep("", "AUD"), ep("", "AUD")]);
        assert_eq!(
            w.to_edit_transaction(&registry()),
            Err(EditError::Ambiguous)
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn to_edit_missing_account_errors() {
        let mut w = et(vec![ep("AUD -1.00", "AUD"), ep("AUD 1.00", "AUD")]);
        w.postings[0].account_id = String::new();
        assert_eq!(
            w.to_edit_transaction(&registry()),
            Err(EditError::MissingAccount { index: 0 })
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn to_edit_bad_amount_errors() {
        let mut w = et(vec![ep("xx", "AUD"), ep("AUD 1.00", "AUD")]);
        w.postings[0].amount = "xx".to_owned();
        assert!(matches!(
            w.to_edit_transaction(&registry()),
            Err(EditError::Amount { index: 0, .. })
        ));
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn from_transaction_assigns_unique_uids() {
        let t = sample_two_posting_tx();
        let e = EditableTransaction::from_transaction(&t);
        assert_eq!(e.postings.len(), 2);
        assert_ne!(e.postings[0].uid, e.postings[1].uid);
    }

    #[test]
    fn push_blank_posting_returns_fresh_uid() {
        let t = sample_two_posting_tx();
        let mut e = EditableTransaction::from_transaction(&t);
        let max_before = e.postings.iter().map(|p| p.uid).max().unwrap_or(0);
        let new_uid = e.push_blank_posting();
        assert_eq!(e.postings.len(), 3);
        assert!(new_uid > max_before);
        assert_eq!(e.postings.last().map(|p| p.uid), Some(new_uid));
    }

    #[test]
    fn extra_dates_round_trip() {
        // extra_dates is the 6th arg of Transaction::new (before reconciliation/tags/postings/audit)
        let t = Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "",
            None::<&str>,
            vec![("cleared".to_owned(), Date::constant(2026, 5, 2))], // extra_dates
            Reconciliation::Unreconciled,
            vec![],
            two_balanced_postings(),
            vec![],
        );
        let e = EditableTransaction::from_transaction(&t);
        assert_eq!(
            e.extra_dates,
            vec![("cleared".to_owned(), "2026-05-02".to_owned())]
        );
        let edit = e.to_edit_transaction(&registry()).expect("valid");
        assert_eq!(
            edit.extra_dates,
            vec![("cleared".to_owned(), Date::constant(2026, 5, 2))]
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn malformed_extra_date_is_rejected() {
        let t = Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "",
            None::<&str>,
            vec![("cleared".to_owned(), Date::constant(2026, 5, 2))], // extra_dates
            Reconciliation::Unreconciled,
            vec![],
            two_balanced_postings(),
            vec![],
        );
        let mut e = EditableTransaction::from_transaction(&t);
        e.extra_dates[0].1 = "not-a-date".to_owned();
        assert!(matches!(
            e.to_edit_transaction(&registry()),
            Err(EditError::ExtraDate { index: 0, .. })
        ));
    }

    #[test]
    fn empty_extra_date_row_is_dropped_not_blocking() {
        let mut e = et(vec![ep("AUD -1.00", "AUD"), ep("AUD 1.00", "AUD")]);
        e.extra_dates = vec![(String::new(), String::new())];
        let edit = e
            .to_edit_transaction(&registry())
            .expect("blank extra-date row is pruned");
        assert!(edit.extra_dates.is_empty());
    }

    #[test]
    fn filled_extra_date_row_is_kept_among_blanks() {
        let mut e = et(vec![ep("AUD -1.00", "AUD"), ep("AUD 1.00", "AUD")]);
        e.extra_dates = vec![
            ("cleared".to_owned(), "2026-05-02".to_owned()),
            (String::new(), "  ".to_owned()),
        ];
        let edit = e
            .to_edit_transaction(&registry())
            .expect("blank row pruned, filled kept");
        assert_eq!(
            edit.extra_dates,
            vec![("cleared".to_owned(), Date::constant(2026, 5, 2))]
        );
    }

    #[test]
    fn nonempty_malformed_extra_date_still_errors() {
        let mut e = et(vec![ep("AUD -1.00", "AUD"), ep("AUD 1.00", "AUD")]);
        e.extra_dates = vec![("cleared".to_owned(), "not-a-date".to_owned())];
        assert!(matches!(
            e.to_edit_transaction(&registry()),
            Err(EditError::ExtraDate { index: 0, .. })
        ));
    }
}
