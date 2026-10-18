//! Deciding what applying an edited working file would do, without writing.
//!
//! [`plan`] compares each transaction in the parsed file against its sidecar
//! snapshot and the current stored state, and settles on one [`Action`] per
//! transaction. It reads no database: the caller loads the current state and
//! the protected postings, which keeps every decision here testable by hand.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::hash::BuildHasher;

use bc_models::Amount;
use bc_models::Cost;
use bc_models::MetaEntry;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::Transaction;
use bc_models::TransactionId;

use super::Sidecar;
use super::resolve::Resolver;
use super::resolve::Unresolved;
use super::snapshot::spelled_mask;
use crate::Directive;
use crate::ImportOutput;
use crate::RawTransaction;

/// Why a transaction in the file needs no write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// The file states the transaction as exported.
    Unchanged,
    /// The stored transaction already carries the file's content and is
    /// flagged: a rerun of a file already applied.
    AlreadyApplied,
}

/// What apply does with one transaction in the file.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    /// Write nothing.
    Skip {
        /// The transaction's handle; a new transaction already applied has
        /// none.
        handle: Option<String>,
        /// Why nothing is written.
        reason: SkipReason,
    },
    /// Replace `base` with `edited`, provided the stored transaction still
    /// equals `base`.
    Edit {
        /// The transaction's handle.
        handle: String,
        /// The transaction as exported.
        base: Transaction,
        /// The transaction as the file states it, with the snapshot's
        /// `PostingId`s on matched legs and every unwritten field restored.
        edited: Transaction,
    },
    /// Store a new transaction.
    Create {
        /// The transaction as the file states it, `Flagged`.
        edited: Transaction,
        /// The transaction's position among the file's transactions.
        raw_index: usize,
        /// Each leg's position among identical legs of the file's creates,
        /// counted past the slots of the transactions the file names by
        /// handle.
        /// [`plan`] leaves it empty; [`super::RoundTripService::plan_file`]
        /// fills it.
        positions: Vec<u32>,
    },
    /// Delete the transaction, provided the stored transaction still equals
    /// `base`.
    Delete {
        /// The transaction's handle.
        handle: String,
        /// The transaction as exported.
        base: Transaction,
    },
    /// Leave this transaction alone and report it; the rest still applies.
    Refuse {
        /// The transaction's handle, when it has one.
        handle: Option<String>,
        /// Why.
        reason: String,
    },
}

/// Every action an apply would take, and the warnings it raises.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::module_name_repetitions,
    reason = "the type is re-exported at `roundtrip` where the prefix is needed"
)]
#[non_exhaustive]
pub struct ApplyPlan {
    /// One action per transaction in the file, in file order.
    pub actions: Vec<Action>,
    /// Patterns worth a look that block nothing.
    pub warnings: Vec<String>,
}

/// Why the whole file is refused: every problem found, so one fix-up pass
/// covers them all.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreflightError {
    /// Each problem, as a person reads it.
    pub problems: Vec<String>,
}

impl fmt::Display for PreflightError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the file cannot be applied: {}",
            self.problems.join("; ")
        )
    }
}

impl core::error::Error for PreflightError {}

/// Why storing `tx` would fail validation, or `None` when it can be stored.
fn unwritable(tx: &Transaction) -> Option<String> {
    match crate::transaction::validate_postings(tx.postings()) {
        Ok(()) => None,
        Err(crate::BcError::BadData(reason)) => Some(reason),
        Err(other) => Some(other.to_string()),
    }
}

/// Decides what applying `output` would do.
///
/// Pre-flight refuses the whole file, listing every problem at once, when a
/// source failed to parse, a name does not resolve, a handle is not in the
/// sidecar or sits on two transactions, or a handled transaction's date
/// differs from its snapshot's.
///
/// Otherwise each file transaction gets one action, in file order. One
/// without a handle is created. One with a handle is matched leg by leg
/// against its snapshot, gets back every field the file does not spell, and
/// is classified, first rule winning:
///
/// 1. a delete marker deletes when the stored transaction is unchanged since
///    export, and is refused otherwise;
/// 2. a file equal in content to the snapshot is skipped as unchanged;
/// 3. a stored transaction that is `Flagged` and equal in content to the
///    file is skipped as already applied;
/// 4. a stored transaction changed since export, reconciliation included,
///    is refused;
/// 5. an edit the store would reject (no postings, two elided legs, a lone
///    elided leg, a negative price or cost) is refused;
/// 6. anything else is an edit.
///
/// One without a handle that the store would reject is refused the same way.
/// A sidecar transaction absent from the file gets no action.
///
/// # Arguments
///
/// * `output` - The parsed working file.
/// * `sidecar` - The export's sidecar.
/// * `resolver` - The ledger's names.
/// * `current` - The stored state of every sidecar transaction still stored.
/// * `protected` - Every posting with a live source reference.
///
/// # Errors
///
/// Returns [`PreflightError`] listing every problem that refuses the whole
/// file.
#[inline]
pub fn plan<C, P>(
    output: &ImportOutput,
    sidecar: &Sidecar,
    resolver: &Resolver,
    current: &HashMap<TransactionId, Transaction, C>,
    protected: &HashSet<PostingId, P>,
) -> Result<ApplyPlan, PreflightError>
where
    C: BuildHasher,
    P: BuildHasher,
{
    let raws: Vec<&RawTransaction> = output
        .directives
        .iter()
        .filter_map(|directive| match *directive {
            Directive::Transaction(ref raw) => Some(raw),
            Directive::Open(_) | Directive::Close(_) => None,
        })
        .collect();
    let files = preflight(output, &raws, sidecar, resolver)?;

    let context = Context {
        sidecar,
        resolver,
        current,
        protected,
    };
    let mut plan = ApplyPlan::default();
    for (index, (raw, file)) in raws.iter().zip(&files).enumerate() {
        let action = context.action(index, raw, file);
        if let Action::Delete {
            ref handle,
            ref base,
        } = action
        {
            plan.warnings
                .extend(context.hand_merges(handle, base, index, &files));
        }
        plan.actions.push(action);
    }
    Ok(plan)
}

// MARK: Pre-flight

/// Resolves every file transaction, or lists every problem that refuses the
/// whole file.
fn preflight(
    output: &ImportOutput,
    raws: &[&RawTransaction],
    sidecar: &Sidecar,
    resolver: &Resolver,
) -> Result<Vec<Transaction>, PreflightError> {
    let mut problems: Vec<String> = output
        .sources
        .iter()
        .filter_map(|source| {
            source
                .failure()
                .map(|reason| format!("cannot parse {}: {reason}", source.source.display))
        })
        .collect();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    let mut handles: Vec<(&str, usize)> = Vec::new();
    let mut files = Vec::with_capacity(raws.len());
    for raw in raws {
        match resolver.transaction(raw) {
            Ok(file) => files.push(file),
            Err(names) => {
                for name in names {
                    if !unresolved.contains(&name) {
                        unresolved.push(name);
                    }
                }
            }
        }
        let Some(handle) = handle_of(raw) else {
            continue;
        };
        if let Some(&mut (_, ref mut count)) = handles.iter_mut().find(|&&mut (h, _)| h == handle) {
            *count = count.saturating_add(1);
            continue;
        }
        handles.push((handle, 1));
        match sidecar.entry(handle) {
            None => problems.push(format!("handle `{handle}` is not in the sidecar")),
            Some(entry) if entry.transaction.date() != raw.date => problems.push(format!(
                "handle `{handle}` is dated {} but was exported dated {}: \
                 wrong sidecar or edited date",
                raw.date,
                entry.transaction.date()
            )),
            Some(_) => {}
        }
    }
    problems.extend(
        handles
            .iter()
            .filter(|&&(_, count)| count > 1)
            .map(|&(handle, count)| format!("handle `{handle}` is on {count} transactions")),
    );
    problems.extend(unresolved.iter().map(ToString::to_string));
    if problems.is_empty() {
        Ok(files)
    } else {
        Err(PreflightError { problems })
    }
}

/// The handle `raw` names, if any.
fn handle_of(raw: &RawTransaction) -> Option<&str> {
    raw.native.as_ref()?.handle.as_deref()
}

/// Whether `raw` carries the delete marker.
fn is_delete(raw: &RawTransaction) -> bool {
    raw.native.as_ref().is_some_and(|native| native.delete)
}

// MARK: Classification

/// What every per-transaction decision reads.
struct Context<'a, C, P> {
    /// The export's sidecar.
    sidecar: &'a Sidecar,
    /// The ledger's names, for reports.
    resolver: &'a Resolver,
    /// The stored state of every sidecar transaction still stored.
    current: &'a HashMap<TransactionId, Transaction, C>,
    /// Every posting with a live source reference.
    protected: &'a HashSet<PostingId, P>,
}

impl<C, P> Context<'_, C, P>
where
    C: BuildHasher,
    P: BuildHasher,
{
    /// Decides the action for the file transaction at `index`.
    fn action(&self, index: usize, raw: &RawTransaction, file: &Transaction) -> Action {
        let Some(handle) = handle_of(raw) else {
            if is_delete(raw) {
                return Action::Refuse {
                    handle: None,
                    reason: "`bc-delete` needs a `bc` handle".to_owned(),
                };
            }
            if let Some(reason) = unwritable(file) {
                return Action::Refuse {
                    handle: None,
                    reason,
                };
            }
            return Action::Create {
                edited: file.clone(),
                raw_index: index,
                positions: Vec::new(),
            };
        };
        let refuse = |reason: &str| Action::Refuse {
            handle: Some(handle.to_owned()),
            reason: reason.to_owned(),
        };
        let skip = |reason| Action::Skip {
            handle: Some(handle.to_owned()),
            reason,
        };
        let Some(snapshot) = self.sidecar.entry(handle).map(|entry| &entry.transaction) else {
            return refuse("not in the sidecar");
        };
        let current = self.current.get(snapshot.id());

        if is_delete(raw) {
            return match current {
                None => skip(SkipReason::AlreadyApplied),
                Some(stored) if unchanged_since(stored, snapshot) => Action::Delete {
                    handle: handle.to_owned(),
                    base: snapshot.clone(),
                },
                Some(_) => refuse("changed since export"),
            };
        }
        let Some(stored) = current else {
            return refuse("deleted since export");
        };
        if !same_text(file.description(), snapshot.description()) {
            return refuse("description is not editable; use metadata");
        }
        let postings = match self.match_legs(file.postings(), snapshot.postings()) {
            Ok(postings) => postings,
            Err(reason) => return refuse(&reason),
        };
        let edited = Transaction::builder()
            .id(snapshot.id().clone())
            .date(file.date())
            .description(snapshot.description())
            .postings(postings)
            .reconciliation(stored.reconciliation())
            .tag_ids(file.tag_ids().to_vec())
            .metadata(restore(snapshot.metadata(), file.metadata()))
            .created_at(*stored.created_at())
            .build();

        if same_content(&edited, snapshot) {
            skip(SkipReason::Unchanged)
        } else if stored.reconciliation() == Reconciliation::Flagged
            && same_content(stored, &edited)
        {
            skip(SkipReason::AlreadyApplied)
        } else if !unchanged_since(stored, snapshot) {
            refuse("changed since export")
        } else if let Some(reason) = unwritable(&edited) {
            refuse(&reason)
        } else {
            Action::Edit {
                handle: handle.to_owned(),
                base: snapshot.clone(),
                edited,
            }
        }
    }

    /// Pairs each file leg with a snapshot leg and carries the snapshot's
    /// identity onto it.
    ///
    /// The first pass pairs legs equal in account and amount, in order, then
    /// hands each protected snapshot leg it left unpaired the file leg of an
    /// equal unprotected one, so dropping one of two identical lines keeps
    /// the statement leg. The second pass pairs what is left by position. An
    /// unpaired file leg keeps its fresh `PostingId`.
    ///
    /// # Errors
    ///
    /// Returns the refusal when a protected snapshot leg finds no equal
    /// file leg in the first pass.
    fn match_legs(&self, file: &[Posting], snapshot: &[Posting]) -> Result<Vec<Posting>, String> {
        let mut taken = vec![false; snapshot.len()];
        let mut pairs: Vec<Option<usize>> = file
            .iter()
            .map(|leg| {
                let found = snapshot.iter().zip(&taken).position(|(old, &used)| {
                    !used
                        && old.account_id() == leg.account_id()
                        && same_amount(old.amount(), leg.amount())
                });
                if let Some(slot) = found.and_then(|j| taken.get_mut(j)) {
                    *slot = true;
                }
                found
            })
            .collect();
        self.prefer_protected(snapshot, &mut pairs, &mut taken);

        if let Some((old, _)) = snapshot
            .iter()
            .zip(&taken)
            .find(|&(old, &used)| !used && self.protected.contains(old.id()))
        {
            return Err(format!(
                "statement leg changed or removed: {}",
                self.describe(old)
            ));
        }

        let mut free = taken
            .iter()
            .enumerate()
            .filter(|&(_, &used)| !used)
            .map(|(j, _)| j);
        for pair in pairs.iter_mut().filter(|pair| pair.is_none()) {
            *pair = free.next();
        }
        Ok(file
            .iter()
            .zip(pairs)
            .map(|(leg, pair)| match pair.and_then(|j| snapshot.get(j)) {
                Some(old) => matched_leg(leg, old),
                None => leg.clone(),
            })
            .collect())
    }

    /// Moves each file leg paired with an unprotected snapshot leg onto an
    /// equal protected one the pairing left free.
    fn prefer_protected(
        &self,
        snapshot: &[Posting],
        pairs: &mut [Option<usize>],
        taken: &mut [bool],
    ) {
        for (j, old) in snapshot.iter().enumerate() {
            if taken.get(j).copied().unwrap_or(true) || !self.protected.contains(old.id()) {
                continue;
            }
            let swap = pairs.iter_mut().find_map(|pair| {
                let k = (*pair)?;
                let other = snapshot.get(k)?;
                (!self.protected.contains(other.id())
                    && other.account_id() == old.account_id()
                    && same_amount(other.amount(), old.amount()))
                .then_some((pair, k))
            });
            if let Some((pair, k)) = swap {
                *pair = Some(j);
                if let Some(slot) = taken.get_mut(k) {
                    *slot = false;
                }
                if let Some(slot) = taken.get_mut(j) {
                    *slot = true;
                }
            }
        }
    }

    /// Warns when a deleted transaction's protected leg reappears, by
    /// account, date and amount, in another file transaction.
    fn hand_merges(
        &self,
        handle: &str,
        base: &Transaction,
        index: usize,
        files: &[Transaction],
    ) -> Vec<String> {
        base.postings()
            .iter()
            .filter(|leg| self.protected.contains(leg.id()))
            .filter(|leg| {
                files.iter().enumerate().any(|(k, other)| {
                    k != index
                        && other.date() == base.date()
                        && other.postings().iter().any(|p| {
                            p.account_id() == leg.account_id()
                                && same_amount(p.amount(), leg.amount())
                        })
                })
            })
            .map(|leg| {
                format!(
                    "transaction `{handle}` is deleted while its statement leg {} on {} \
                     reappears in another transaction; this looks like a hand-made merge, \
                     which loses the leg's provenance",
                    self.describe(leg),
                    base.date()
                )
            })
            .collect()
    }

    /// A leg as a person reads it: account path and amount.
    fn describe(&self, leg: &Posting) -> String {
        let account = self
            .resolver
            .account_path(leg.account_id())
            .map_or_else(|| leg.account_id().to_string(), ToOwned::to_owned);
        match leg.amount() {
            Some(amount) => format!("{account} {} {}", amount.value(), amount.commodity()),
            None => format!("{account} (elided)"),
        }
    }
}

/// `file` carrying the identity of the snapshot leg `old` it was paired with.
///
/// The leg takes `old`'s `PostingId`, keeps `old`'s spelling of any amount,
/// price or cost the file states numerically equal or with only its line
/// breaks flattened, and gets back the metadata the export omitted.
fn matched_leg(file: &Posting, old: &Posting) -> Posting {
    let amount = if same_amount(file.amount(), old.amount()) {
        old.amount()
    } else {
        file.amount()
    };
    let price = if file.price() == old.price() {
        old.price()
    } else {
        file.price()
    };
    let cost = if same_cost(file.cost(), old.cost()) {
        old.cost()
    } else {
        file.cost()
    };
    Posting::builder()
        .id(old.id().clone())
        .account_id(file.account_id().clone())
        .maybe_amount(amount.cloned())
        .maybe_price(price.cloned())
        .maybe_cost(cost.cloned())
        .metadata(restore(old.metadata(), file.metadata()))
        .tag_ids(file.tag_ids().to_vec())
        .maybe_spread_from(file.spread_from())
        .maybe_spread_until(file.spread_until())
        .build()
}

/// Rebuilds metadata from the stored entries and the file's.
///
/// Walks the stored entries in order. An entry the export spelled takes the
/// file's first unused value for its key in place, and is dropped when the
/// file no longer states the key. A file value that is the stored text with
/// its line breaks flattened keeps the stored text. An entry the export omitted stays as
/// stored. File entries left over are new and follow, in file order.
/// Keeping the stored order matters because metadata compares by position.
fn restore(stored: &Metadata, file: &Metadata) -> Metadata {
    let mut unused: Vec<Option<&MetaEntry>> = file.iter().map(Some).collect();
    let mut out = Vec::with_capacity(stored.len().max(file.len()));
    for (entry, spelled) in stored.iter().zip(spelled_mask(stored)) {
        if !spelled {
            out.push(entry.clone());
            continue;
        }
        let stated = unused
            .iter_mut()
            .find(|slot| slot.is_some_and(|e| e.key() == entry.key()))
            .and_then(Option::take);
        match stated {
            Some(file_entry) if same_text_value(file_entry.value(), entry.value()) => {
                out.push(entry.clone());
            }
            other => out.extend(other.cloned()),
        }
    }
    out.extend(unused.into_iter().flatten().cloned());
    Metadata::new(out)
}

// MARK: Comparison

/// `text` as the Beancount writer spells it: each `\n` and each `\r` becomes
/// one space. The plugin's `quoted` applies the same rule.
fn flattened(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// Whether the file's `file` states the stored `stored`: equal, or equal
/// once the stored text's line breaks are flattened.
fn same_text(file: &str, stored: &str) -> bool {
    file == stored || (stored.contains(['\n', '\r']) && file == flattened(stored))
}

/// Whether two metadata values are text the file states as stored, by
/// [`same_text`].
fn same_text_value(file: &MetaValue, stored: &MetaValue) -> bool {
    match (file, stored) {
        (MetaValue::Text(f), MetaValue::Text(s)) => same_text(f, s),
        _ => false,
    }
}

/// Whether the file's cost states the stored one: equal, or with only the
/// stored label's line breaks flattened.
fn same_cost(file: Option<&Cost>, stored: Option<&Cost>) -> bool {
    match (file, stored) {
        (Some(f), Some(s)) => {
            f.basis() == s.basis()
                && f.date() == s.date()
                && match (f.label(), s.label()) {
                    (Some(fl), Some(sl)) => same_text(fl, sl),
                    (None, None) => true,
                    _ => false,
                }
        }
        (None, None) => true,
        _ => false,
    }
}

/// Whether two amounts agree numerically, in the same commodity, or are both
/// elided.
fn same_amount(a: Option<&Amount>, b: Option<&Amount>) -> bool {
    a == b
}

/// Whether two transactions agree in content: date, description, tags,
/// metadata and legs in order. Transaction and posting IDs, reconciliation
/// and `created_at` are ignored, and amounts compare numerically.
fn same_content(a: &Transaction, b: &Transaction) -> bool {
    a.date() == b.date()
        && a.description() == b.description()
        && a.tag_ids() == b.tag_ids()
        && a.metadata().eq_ignoring_mismatched(b.metadata())
        && a.postings().len() == b.postings().len()
        && a.postings()
            .iter()
            .zip(b.postings())
            .all(|(x, y)| same_leg(x, y))
}

/// Whether two legs agree in content, ignoring their IDs.
fn same_leg(a: &Posting, b: &Posting) -> bool {
    a.account_id() == b.account_id()
        && same_amount(a.amount(), b.amount())
        && a.price() == b.price()
        && a.cost() == b.cost()
        && a.tag_ids() == b.tag_ids()
        && a.metadata().eq_ignoring_mismatched(b.metadata())
        && a.spread_from() == b.spread_from()
        && a.spread_until() == b.spread_until()
}

/// Whether `stored` is still what was exported: equal content, the same
/// `PostingId`s in order and the same reconciliation.
pub(crate) fn unchanged_since(stored: &Transaction, snapshot: &Transaction) -> bool {
    same_content(stored, snapshot)
        && stored.reconciliation() == snapshot.reconciliation()
        && stored
            .postings()
            .iter()
            .map(Posting::id)
            .eq(snapshot.postings().iter().map(Posting::id))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::Commodity;
    use bc_models::CommodityCode;
    use bc_models::Cost;
    use bc_models::MetaEntry;
    use bc_models::MetaKey;
    use bc_models::MetaType;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::Posting;
    use bc_models::Reconciliation;
    use bc_models::Tag;
    use bc_models::TagId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::Directive;
    use crate::NativePosting;
    use crate::NativeTransaction;
    use crate::RawMetaEntry;
    use crate::RawMetaValue;
    use crate::RawPosting;
    use crate::RawTransaction;
    use crate::roundtrip::SidecarEntry;
    use crate::roundtrip::snapshot::spelled_mask;

    // MARK: Fixtures

    /// The ledger every test plans against.
    struct Ledger {
        resolver: Resolver,
        bank: AccountId,
        food: AccountId,
        fees: AccountId,
        equity: AccountId,
        trip: TagId,
        paths: HashMap<AccountId, String>,
        tag_paths: HashMap<TagId, String>,
    }

    fn key(name: &str) -> MetaKey {
        MetaKey::new(name).expect("valid key")
    }

    fn text(name: &str, value: &str) -> MetaEntry {
        MetaEntry::new(key(name), MetaValue::Text(value.to_owned()))
    }

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, CommodityCode::new("AUD"))
    }

    fn ledger() -> Ledger {
        let bank = AccountId::new();
        let food = AccountId::new();
        let fees = AccountId::new();
        let equity = AccountId::new();
        let accounts = HashMap::from([
            ("Assets:Bank".to_owned(), bank.clone()),
            ("Expenses:Food".to_owned(), food.clone()),
            ("Expenses:Fees".to_owned(), fees.clone()),
            ("Equity".to_owned(), equity.clone()),
        ]);
        let tag = Tag::builder().name("trip").build();
        let trip = tag.id().clone();
        let resolver = Resolver::new(
            accounts.clone(),
            &[tag],
            &[Commodity::builder().code("AUD").decimals(2).build()],
            HashMap::from([
                (key("payee"), MetaType::Text),
                (key("note"), MetaType::Text),
                (key("receipt"), MetaType::Text),
                (key("seen"), MetaType::Timestamp),
                (key("via"), MetaType::Account),
            ]),
        );
        Ledger {
            resolver,
            bank,
            food,
            fees,
            equity,
            tag_paths: HashMap::from([(trip.clone(), "trip".to_owned())]),
            trip,
            paths: accounts.into_iter().map(|(path, id)| (id, path)).collect(),
        }
    }

    fn day() -> Date {
        date(2026, 3, 4)
    }

    fn leg(account: &AccountId, amount: Decimal) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .amount(aud(amount))
            .build()
    }

    fn leg_with_meta(account: &AccountId, amount: Decimal, metadata: Metadata) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .amount(aud(amount))
            .metadata(metadata)
            .build()
    }

    /// A stored transaction on [`day`].
    fn stored(postings: Vec<Posting>, metadata: Metadata) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(day())
            .description("Woolworths")
            .postings(postings)
            .reconciliation(Reconciliation::Unreconciled)
            .metadata(metadata)
            .created_at(Timestamp::UNIX_EPOCH)
            .build()
    }

    /// `tx` with its reconciliation, metadata and postings replaced.
    fn copy_of(
        tx: &Transaction,
        reconciliation: Reconciliation,
        metadata: Metadata,
        postings: Vec<Posting>,
    ) -> Transaction {
        Transaction::builder()
            .id(tx.id().clone())
            .date(tx.date())
            .description(tx.description())
            .postings(postings)
            .reconciliation(reconciliation)
            .tag_ids(tx.tag_ids().to_vec())
            .metadata(metadata)
            .created_at(*tx.created_at())
            .build()
    }

    /// `tx` with only its reconciliation replaced.
    fn reconciled_as(tx: &Transaction, reconciliation: Reconciliation) -> Transaction {
        copy_of(
            tx,
            reconciliation,
            tx.metadata().clone(),
            tx.postings().to_vec(),
        )
    }

    /// The metadata the writer spells, as the reader parses it back.
    fn raw_meta(metadata: &Metadata, l: &Ledger) -> Vec<RawMetaEntry> {
        metadata
            .iter()
            .zip(spelled_mask(metadata))
            .filter(|&(_, spelled)| spelled)
            .map(|(entry, _)| match *entry.value() {
                MetaValue::Account(ref id) => {
                    let path = l.paths.get(id).expect("known account").clone();
                    if path.contains(':') {
                        RawMetaEntry::account_path(entry.key().clone(), path)
                    } else {
                        RawMetaEntry::resolved(entry.key().clone(), MetaValue::Text(path))
                    }
                }
                MetaValue::Timestamp(stamp) => {
                    RawMetaEntry::resolved(entry.key().clone(), MetaValue::Text(stamp.to_string()))
                }
                ref value @ (MetaValue::Text(_)
                | MetaValue::Number(_)
                | MetaValue::Boolean(_)
                | MetaValue::Date(_)
                | MetaValue::Amount(_)) => {
                    RawMetaEntry::resolved(entry.key().clone(), value.clone())
                }
            })
            .collect()
    }

    /// `tx` as an export writes it and the importer reads it back.
    fn raw_from(tx: &Transaction, handle: Option<&str>, l: &Ledger) -> RawTransaction {
        let tags = |ids: &[TagId]| -> Vec<String> {
            ids.iter()
                .map(|id| l.tag_paths.get(id).expect("known tag").clone())
                .collect()
        };
        let postings = tx
            .postings()
            .iter()
            .map(|p| {
                RawPosting::builder()
                    .account(l.paths.get(p.account_id()).expect("known account").clone())
                    .maybe_amount(p.amount().cloned())
                    .maybe_price(p.price().cloned())
                    .maybe_cost(p.cost().cloned())
                    .tags(tags(p.tag_ids()))
                    .metadata(raw_meta(p.metadata(), l))
                    .native(
                        NativePosting::builder()
                            .maybe_spread_from(p.spread_from())
                            .maybe_spread_until(p.spread_until())
                            .build(),
                    )
                    .build()
            })
            .collect();
        RawTransaction::builder()
            .date(tx.date())
            .description(tx.description())
            .tags(tags(tx.tag_ids()))
            .metadata(raw_meta(tx.metadata(), l))
            .postings(postings)
            .native(
                NativeTransaction::builder()
                    .maybe_handle(handle.map(str::to_owned))
                    .reconciliation(tx.reconciliation())
                    .build(),
            )
            .build()
    }

    /// `raw` with every line break the writer flattens written as a space.
    fn flatten_raw(raw: &mut RawTransaction) {
        let flatten = |text: &str| text.replace(['\n', '\r'], " ");
        let flatten_meta = |entries: &mut Vec<RawMetaEntry>| {
            for entry in entries {
                if let RawMetaValue::Resolved(MetaValue::Text(ref mut text)) = entry.value {
                    *text = flatten(text);
                }
            }
        };
        raw.description = flatten(&raw.description);
        flatten_meta(&mut raw.metadata);
        for posting in &mut raw.postings {
            flatten_meta(&mut posting.metadata);
            if let Some(cost) = posting.cost.take() {
                posting.cost = Some(
                    Cost::builder()
                        .basis(cost.basis().clone())
                        .maybe_date(cost.date())
                        .maybe_label(cost.label().map(flatten))
                        .build(),
                );
            }
        }
    }

    fn raw_leg(account: &str, amount: Decimal) -> RawPosting {
        RawPosting::builder()
            .account(account)
            .amount(aud(amount))
            .build()
    }

    fn mark_deleted(raw: &mut RawTransaction) {
        if let Some(native) = raw.native.as_mut() {
            native.delete = true;
        }
    }

    /// Plans `raws` against a sidecar of `exported` and the `current` state.
    fn run(
        l: &Ledger,
        raws: Vec<RawTransaction>,
        exported: &[(&str, &Transaction)],
        current: &[&Transaction],
        protected: &[&PostingId],
    ) -> Result<ApplyPlan, PreflightError> {
        let output = ImportOutput::from(
            raws.into_iter()
                .map(Directive::Transaction)
                .collect::<Vec<_>>(),
        );
        let sidecar = Sidecar::new(
            "db".to_owned(),
            Timestamp::UNIX_EPOCH,
            exported
                .iter()
                .map(|&(handle, tx)| SidecarEntry::new(handle.to_owned(), tx.clone()))
                .collect(),
        );
        let stored: HashMap<_, _> = current
            .iter()
            .map(|&tx| (tx.id().clone(), tx.clone()))
            .collect();
        let sourced: HashSet<_> = protected.iter().map(|&id| id.clone()).collect();
        plan(&output, &sidecar, &l.resolver, &stored, &sourced)
    }

    /// The only action of a one-transaction plan.
    fn only(plan: &ApplyPlan) -> &Action {
        assert_eq!(plan.actions.len(), 1, "{plan:#?}");
        plan.actions.first().expect("one action")
    }

    fn edit_of(action: &Action) -> (&Transaction, &Transaction) {
        let Action::Edit { base, edited, .. } = action else {
            panic!("expected an edit, got {action:#?}");
        };
        (base, edited)
    }

    fn refusal_of(action: &Action) -> &str {
        let Action::Refuse { reason, .. } = action else {
            panic!("expected a refusal, got {action:#?}");
        };
        reason
    }

    fn skip_of(action: &Action) -> SkipReason {
        let Action::Skip { reason, .. } = action else {
            panic!("expected a skip, got {action:#?}");
        };
        *reason
    }

    fn ids(tx: &Transaction) -> Vec<PostingId> {
        tx.postings().iter().map(|p| p.id().clone()).collect()
    }

    /// A two-leg grocery purchase.
    fn purchase(l: &Ledger) -> Transaction {
        stored(
            vec![leg(&l.bank, dec!(-50.00)), leg(&l.food, dec!(50.00))],
            Metadata::new(vec![text("payee", "Woolworths")]),
        )
    }

    // MARK: Pre-flight

    #[test]
    fn unknown_names_refuse_whole_file_listing_all() {
        let l = ledger();
        let tx = purchase(&l);
        let mut first = raw_from(&tx, Some("a1b"), &l);
        first.postings.push(raw_leg("Expenses:Fod", dec!(1.00)));
        first.tags.push("holiday".to_owned());
        let mut second = raw_from(&tx, None, &l);
        second.metadata.push(RawMetaEntry::resolved(
            key("mystery"),
            MetaValue::Boolean(true),
        ));
        second.postings.push(
            RawPosting::builder()
                .account("Expenses:Food")
                .amount(Amount::new(dec!(1.00), CommodityCode::new("XYZ")))
                .build(),
        );
        let err = run(&l, vec![first, second], &[("a1b", &tx)], &[&tx], &[]).expect_err("refused");
        assert_eq!(
            err.problems,
            vec![
                "unknown tag `holiday`".to_owned(),
                "unknown account `Expenses:Fod`".to_owned(),
                "unknown metadata key `mystery`".to_owned(),
                "unknown commodity `XYZ`".to_owned(),
            ]
        );
    }

    #[test]
    fn an_unreadable_spread_refuses_the_file_and_keeps_the_stored_spread() {
        let l = ledger();
        let spread = Posting::builder()
            .id(PostingId::new())
            .account_id(l.food.clone())
            .amount(aud(dec!(50.00)))
            .spread_from(date(2026, 3, 1))
            .spread_until(date(2026, 3, 31))
            .build();
        let tx = stored(
            vec![leg(&l.bank, dec!(-50.00)), spread],
            Metadata::default(),
        );
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        if let Some(food) = raw.postings.get_mut(1) {
            food.native = Some(
                NativePosting::builder()
                    .spread_until(date(2026, 3, 31))
                    .build(),
            );
            food.metadata.push(RawMetaEntry::resolved(
                key("bc-spread-from"),
                MetaValue::Text("2026-03-01".to_owned()),
            ));
        }
        let err = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect_err("refused");
        assert_eq!(
            err.problems,
            vec!["`bc-spread-from` must be a date".to_owned()]
        );
    }

    #[test]
    fn unknown_handle_refused() {
        let l = ledger();
        let tx = purchase(&l);
        let err = run(
            &l,
            vec![raw_from(&tx, Some("zzz"), &l)],
            &[("a1b", &tx)],
            &[&tx],
            &[],
        )
        .expect_err("refused");
        assert_eq!(
            err.problems,
            vec!["handle `zzz` is not in the sidecar".to_owned()]
        );
    }

    #[test]
    fn duplicate_handle_refused() {
        let l = ledger();
        let tx = purchase(&l);
        let raw = raw_from(&tx, Some("a1b"), &l);
        let err =
            run(&l, vec![raw.clone(), raw], &[("a1b", &tx)], &[&tx], &[]).expect_err("refused");
        assert_eq!(
            err.problems,
            vec!["handle `a1b` is on 2 transactions".to_owned()]
        );
    }

    #[test]
    fn date_mismatch_refused_as_wrong_sidecar() {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.date = date(2026, 3, 5);
        let err = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect_err("refused");
        assert_eq!(
            err.problems,
            vec![
                "handle `a1b` is dated 2026-03-05 but was exported dated 2026-03-04: \
                 wrong sidecar or edited date"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn parse_failure_refuses_whole_file() {
        let l = ledger();
        let mut output = ImportOutput::default();
        output.sources.push(crate::SourceOutcome::failed(
            "slice.bean",
            "line 3: bad posting",
        ));
        let sidecar = Sidecar::new("db".to_owned(), Timestamp::UNIX_EPOCH, Vec::new());
        let err = plan(
            &output,
            &sidecar,
            &l.resolver,
            &HashMap::new(),
            &HashSet::new(),
        )
        .expect_err("refused");
        assert_eq!(
            err.problems,
            vec!["cannot parse slice.bean: line 3: bad posting".to_owned()]
        );
    }

    // MARK: Classification

    #[rstest]
    #[case::same_flag(Reconciliation::Unreconciled)]
    #[case::flag_changed_in_file(Reconciliation::Reconciled)]
    fn unchanged_is_skipped(#[case] written: Reconciliation) {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        if let Some(native) = raw.native.as_mut() {
            native.reconciliation = Some(written);
        }
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);
        assert_eq!(plan.warnings, Vec::<String>::new());
    }

    #[test]
    fn rescaled_amount_is_unchanged() {
        let l = ledger();
        let tx = stored(
            vec![leg(&l.bank, dec!(-985.6300)), leg(&l.food, dec!(985.6300))],
            Metadata::default(),
        );
        let bank = ids(&tx).first().cloned().expect("bank leg");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        for posting in &mut raw.postings {
            let value = posting.amount.as_ref().expect("amount").value().normalize();
            posting.amount = Some(aud(value));
        }
        assert_eq!(
            raw.postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| a.value().to_string()),
            Some("-985.63".to_owned())
        );
        let plan = run(&l, vec![raw.clone()], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);

        raw.postings.push(raw_leg("Expenses:Fees", dec!(0.00)));
        let widened = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        let (_, edited) = edit_of(only(&widened));
        let amounts: Vec<String> = edited
            .postings()
            .iter()
            .map(|p| p.amount().expect("amount").value().to_string())
            .collect();
        assert_eq!(amounts, vec!["-985.6300", "985.6300", "0.00"]);
    }

    #[test]
    fn added_leg_is_edit_keeping_posting_ids() {
        let l = ledger();
        let tx = purchase(&l);
        let [bank, food] = <[PostingId; 2]>::try_from(ids(&tx)).expect("two legs");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        if let Some(food_leg) = raw.postings.get_mut(1) {
            food_leg.amount = Some(aud(dec!(45.00)));
        }
        raw.postings.push(raw_leg("Expenses:Fees", dec!(5.00)));
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        let (base, edited) = edit_of(only(&plan));
        assert_eq!(base.id(), tx.id());
        assert_eq!(edited.id(), tx.id());
        let edited_ids = ids(edited);
        assert_eq!(
            edited_ids.get(..2),
            Some([bank.clone(), food.clone()].as_slice())
        );
        let added = edited_ids.get(2).expect("added leg");
        assert!(added != &bank && added != &food);
        assert_eq!(
            edited.postings().get(2).map(Posting::account_id),
            Some(&l.fees)
        );
        assert_eq!(edited.metadata(), tx.metadata());
        assert_eq!(edited.created_at(), tx.created_at());
        assert_eq!(edited.reconciliation(), tx.reconciliation());
    }

    #[test]
    fn reordered_legs_keep_ids() {
        let l = ledger();
        let tx = purchase(&l);
        let [bank, food] = <[PostingId; 2]>::try_from(ids(&tx)).expect("two legs");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.postings.reverse();
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        let (_, edited) = edit_of(only(&plan));
        assert_eq!(ids(edited), vec![food, bank]);
        assert_eq!(
            edited
                .postings()
                .iter()
                .map(Posting::account_id)
                .collect::<Vec<_>>(),
            vec![&l.food, &l.bank]
        );
    }

    #[rstest]
    #[case::amount_changed(Some(dec!(-55.00)), "Assets:Bank")]
    #[case::account_changed(Some(dec!(-50.00)), "Expenses:Fees")]
    #[case::removed(None, "")]
    fn changed_statement_leg_refuses_transaction(
        #[case] amount: Option<Decimal>,
        #[case] account: &str,
    ) {
        let l = ledger();
        let tx = purchase(&l);
        let other = purchase(&l);
        let bank = ids(&tx).first().cloned().expect("bank leg");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        match amount {
            Some(value) => {
                if let Some(first) = raw.postings.first_mut() {
                    first.amount = Some(aud(value));
                    account.clone_into(&mut first.account);
                }
            }
            None => {
                raw.postings.remove(0);
            }
        }
        let mut edited_other = raw_from(&other, Some("c2d"), &l);
        edited_other.metadata.push(RawMetaEntry::resolved(
            key("note"),
            MetaValue::Text("split later".to_owned()),
        ));
        let plan = run(
            &l,
            vec![raw, edited_other],
            &[("a1b", &tx), ("c2d", &other)],
            &[&tx, &other],
            &[&bank],
        )
        .expect("plans");
        let [refused, applied] = plan.actions.as_slice() else {
            panic!("two actions: {plan:#?}");
        };
        assert_eq!(
            refusal_of(refused),
            "statement leg changed or removed: Assets:Bank -50.00 AUD"
        );
        edit_of(applied);
    }

    /// Of two identical bank lines, one a statement leg, the file drops
    /// one: the statement leg keeps its line whichever order the snapshot
    /// holds them in.
    #[rstest]
    #[case::statement_leg_second(1)]
    #[case::statement_leg_first(0)]
    fn dropping_one_of_two_identical_legs_keeps_the_statement_leg(#[case] statement: usize) {
        let l = ledger();
        let tx = stored(
            vec![
                leg(&l.bank, dec!(-10.00)),
                leg(&l.bank, dec!(-10.00)),
                leg(&l.food, dec!(20.00)),
            ],
            Metadata::new(vec![text("payee", "Woolworths")]),
        );
        let [first, second, food] = <[PostingId; 3]>::try_from(ids(&tx)).expect("three legs");
        let (kept, dropped) = if statement == 0 {
            (first, second)
        } else {
            (second, first)
        };
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.postings.remove(0);

        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[&kept]).expect("plans");

        let (_, edited) = edit_of(only(&plan));
        assert_eq!(ids(edited), vec![kept, food]);
        assert!(!ids(edited).contains(&dropped));
    }

    #[test]
    fn placeholder_leg_change_allowed() {
        let l = ledger();
        let tx = purchase(&l);
        let [bank, food] = <[PostingId; 2]>::try_from(ids(&tx)).expect("two legs");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        if let Some(placeholder) = raw.postings.get_mut(1) {
            "Expenses:Fees".clone_into(&mut placeholder.account);
        }
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        let (_, edited) = edit_of(only(&plan));
        assert_eq!(ids(edited), vec![bank, food]);
        assert_eq!(
            edited.postings().get(1).map(Posting::account_id),
            Some(&l.fees)
        );
    }

    #[rstest]
    #[case::reconciled_since(true)]
    #[case::edited_since(false)]
    fn stale_refused_including_reconciliation(#[case] reconciled: bool) {
        let l = ledger();
        let tx = purchase(&l);
        let current = if reconciled {
            reconciled_as(&tx, Reconciliation::Reconciled)
        } else {
            copy_of(
                &tx,
                tx.reconciliation(),
                Metadata::new(vec![
                    text("payee", "Woolworths"),
                    text("note", "from the UI"),
                ]),
                tx.postings().to_vec(),
            )
        };
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.metadata.push(RawMetaEntry::resolved(
            key("receipt"),
            MetaValue::Text("R-1".to_owned()),
        ));
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&current], &[]).expect("plans");
        assert_eq!(refusal_of(only(&plan)), "changed since export");
    }

    #[test]
    fn already_applied_skipped() {
        let l = ledger();
        let tx = purchase(&l);
        let bank = ids(&tx).first().cloned().expect("bank leg");
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.postings.push(raw_leg("Expenses:Fees", dec!(0.00)));
        let first = run(&l, vec![raw.clone()], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        let (_, edited) = edit_of(only(&first));
        let applied = reconciled_as(edited, Reconciliation::Flagged);

        let rerun = run(
            &l,
            vec![raw.clone()],
            &[("a1b", &tx)],
            &[&applied],
            &[&bank],
        )
        .expect("plans");
        assert_eq!(skip_of(only(&rerun)), SkipReason::AlreadyApplied);

        let unflagged = reconciled_as(edited, Reconciliation::Reconciled);
        let reconciled =
            run(&l, vec![raw], &[("a1b", &tx)], &[&unflagged], &[&bank]).expect("plans");
        assert_eq!(refusal_of(only(&reconciled)), "changed since export");
    }

    #[test]
    fn missing_transaction_ignored() {
        let l = ledger();
        let kept = purchase(&l);
        let dropped = purchase(&l);
        let plan = run(
            &l,
            vec![raw_from(&kept, Some("a1b"), &l)],
            &[("a1b", &kept), ("c2d", &dropped)],
            &[&kept, &dropped],
            &[],
        )
        .expect("plans");
        let Action::Skip { handle, .. } = only(&plan) else {
            panic!("expected a skip");
        };
        assert_eq!(handle.as_deref(), Some("a1b"));
    }

    #[test]
    fn edit_of_transaction_deleted_since_export_refused() {
        let l = ledger();
        let tx = purchase(&l);
        let plan = run(
            &l,
            vec![raw_from(&tx, Some("a1b"), &l)],
            &[("a1b", &tx)],
            &[],
            &[],
        )
        .expect("plans");
        assert_eq!(refusal_of(only(&plan)), "deleted since export");
    }

    #[test]
    fn changed_description_refused() {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        "Woolworths Metro".clone_into(&mut raw.description);
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(
            refusal_of(only(&plan)),
            "description is not editable; use metadata"
        );
    }

    // MARK: Delete

    #[test]
    fn delete_marker_deletes() {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        mark_deleted(&mut raw);
        raw.metadata.push(RawMetaEntry::resolved(
            key("note"),
            MetaValue::Text("duplicate".to_owned()),
        ));
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        let Action::Delete { handle, base } = only(&plan) else {
            panic!("expected a delete: {plan:#?}");
        };
        assert_eq!(handle, "a1b");
        assert_eq!(base.id(), tx.id());
        assert_eq!(ids(base), ids(&tx));
        assert_eq!(plan.warnings, Vec::<String>::new());
    }

    #[rstest]
    #[case::reconciled_since(Reconciliation::Reconciled)]
    #[case::flagged_since(Reconciliation::Flagged)]
    fn stale_delete_refused(#[case] since: Reconciliation) {
        let l = ledger();
        let tx = purchase(&l);
        let current = reconciled_as(&tx, since);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        mark_deleted(&mut raw);
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&current], &[]).expect("plans");
        assert_eq!(refusal_of(only(&plan)), "changed since export");
    }

    #[test]
    fn delete_already_applied_skipped() {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        mark_deleted(&mut raw);
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::AlreadyApplied);
    }

    #[test]
    fn delete_marker_without_handle_refused() {
        let l = ledger();
        let tx = purchase(&l);
        let mut raw = raw_from(&tx, None, &l);
        mark_deleted(&mut raw);
        let plan = run(&l, vec![raw], &[], &[], &[]).expect("plans");
        let Action::Refuse { handle, reason } = only(&plan) else {
            panic!("expected a refusal");
        };
        assert_eq!(handle, &None);
        assert_eq!(reason, "`bc-delete` needs a `bc` handle");
    }

    #[test]
    fn hand_merge_warns() {
        let l = ledger();
        let tx = purchase(&l);
        let bank = ids(&tx).first().cloned().expect("bank leg");
        let mut deleted = raw_from(&tx, Some("a1b"), &l);
        mark_deleted(&mut deleted);
        let merged = RawTransaction::builder()
            .date(day())
            .description("Transfer")
            .postings(vec![
                raw_leg("Assets:Bank", dec!(-50.00)),
                raw_leg("Expenses:Food", dec!(50.00)),
            ])
            .build();
        let plan =
            run(&l, vec![deleted, merged], &[("a1b", &tx)], &[&tx], &[&bank]).expect("plans");
        assert!(matches!(
            plan.actions.as_slice(),
            [Action::Delete { .. }, Action::Create { .. }]
        ));
        assert_eq!(
            plan.warnings,
            vec![
                "transaction `a1b` is deleted while its statement leg Assets:Bank -50.00 AUD \
                 on 2026-03-04 reappears in another transaction; this looks like a hand-made \
                 merge, which loses the leg's provenance"
                    .to_owned()
            ]
        );
    }

    // MARK: Field preservation

    #[test]
    fn omitted_metadata_restored() {
        let l = ledger();
        let tx_meta = Metadata::new(vec![
            MetaEntry::mismatch(key("seen"), "last tuesday"),
            text("payee", "Woolworths"),
            text("note", "first"),
            text("note", "second"),
        ]);
        let leg_meta = Metadata::new(vec![
            text("receipt", "R-1"),
            text("receipt", "R-2"),
            text("note", "leg"),
        ]);
        let tx = stored(
            vec![
                leg_with_meta(&l.bank, dec!(-50.00), leg_meta),
                leg(&l.food, dec!(50.00)),
            ],
            tx_meta,
        );
        let raw = raw_from(&tx, Some("a1b"), &l);
        let plan = run(&l, vec![raw.clone()], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);

        let mut edited_raw = raw;
        edited_raw.metadata = vec![
            RawMetaEntry::resolved(key("payee"), MetaValue::Text("Woolworths".to_owned())),
            RawMetaEntry::resolved(key("note"), MetaValue::Text("changed".to_owned())),
            RawMetaEntry::resolved(key("receipt"), MetaValue::Text("R-9".to_owned())),
        ];
        if let Some(first) = edited_raw.postings.first_mut() {
            first.metadata = vec![RawMetaEntry::resolved(
                key("receipt"),
                MetaValue::Text("R-1".to_owned()),
            )];
        }
        let changed = run(&l, vec![edited_raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        let (_, edited) = edit_of(only(&changed));
        assert_eq!(
            edited.metadata(),
            &Metadata::new(vec![
                MetaEntry::mismatch(key("seen"), "last tuesday"),
                text("payee", "Woolworths"),
                text("note", "changed"),
                text("note", "second"),
                text("receipt", "R-9"),
            ])
        );
        assert_eq!(
            edited.postings().first().map(Posting::metadata),
            Some(&Metadata::new(vec![
                text("receipt", "R-1"),
                text("receipt", "R-2")
            ]))
        );
    }

    #[test]
    fn quoted_timestamp_and_colonless_account_round_trip_unchanged() {
        let l = ledger();
        let stamp = Timestamp::from_second(1_700_000_000).expect("valid timestamp");
        let tx = stored(
            vec![leg(&l.bank, dec!(-50.00)), leg(&l.food, dec!(50.00))],
            Metadata::new(vec![
                MetaEntry::new(key("seen"), MetaValue::Timestamp(stamp)),
                MetaEntry::new(key("via"), MetaValue::Account(l.equity.clone())),
                MetaEntry::new(key("note"), MetaValue::Text("Assets:Bank".to_owned())),
            ]),
        );
        let raw = raw_from(&tx, Some("a1b"), &l);
        assert_eq!(
            raw.metadata.get(1).map(|e| &e.value),
            Some(&RawMetaValue::Resolved(MetaValue::Text(
                "Equity".to_owned()
            )))
        );
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);
    }

    #[test]
    fn tags_spread_and_price_compare_by_content() {
        let l = ledger();
        let tagged = Posting::builder()
            .id(PostingId::new())
            .account_id(l.food.clone())
            .amount(aud(dec!(50.00)))
            .price(bc_models::Quote::PerUnit(aud(dec!(1.0))))
            .tag_ids(vec![l.trip.clone()])
            .spread_from(date(2026, 3, 1))
            .spread_until(date(2026, 3, 31))
            .build();
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(day())
            .description("Woolworths")
            .postings(vec![leg(&l.bank, dec!(-50.00)), tagged])
            .reconciliation(Reconciliation::Unreconciled)
            .tag_ids(vec![l.trip.clone()])
            .created_at(Timestamp::UNIX_EPOCH)
            .build();
        let raw = raw_from(&tx, Some("a1b"), &l);
        let plan = run(&l, vec![raw.clone()], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);

        let mut untagged = raw;
        untagged.tags.clear();
        let untagged_plan = run(&l, vec![untagged], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        let (_, edited) = edit_of(only(&untagged_plan));
        assert_eq!(edited.tag_ids(), Vec::<TagId>::new());
        assert_eq!(
            edited.postings().get(1).map(Posting::spread_until),
            Some(Some(date(2026, 3, 31)))
        );
    }

    #[test]
    fn flattened_line_breaks_are_unchanged_and_keep_the_stored_spelling() {
        let l = ledger();
        let lot = Cost::builder()
            .basis(bc_models::Quote::PerUnit(aud(dec!(5.00))))
            .label("lot\r\none")
            .build();
        let costed = Posting::builder()
            .id(PostingId::new())
            .account_id(l.food.clone())
            .amount(aud(dec!(50.00)))
            .cost(lot)
            .metadata(Metadata::new(vec![text("note", "leg\nnote")]))
            .build();
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(day())
            .description("Woolworths\nMetro")
            .postings(vec![leg(&l.bank, dec!(-50.00)), costed])
            .reconciliation(Reconciliation::Unreconciled)
            .metadata(Metadata::new(vec![
                text("payee", "Woolworths\rMetro"),
                text("receipt", "R-1\nR-2"),
            ]))
            .created_at(Timestamp::UNIX_EPOCH)
            .build();
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        flatten_raw(&mut raw);
        assert_eq!(raw.description, "Woolworths Metro");
        let plan = run(&l, vec![raw.clone()], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        assert_eq!(skip_of(only(&plan)), SkipReason::Unchanged);

        raw.postings.push(raw_leg("Expenses:Fees", dec!(0.00)));
        let widened = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        let (_, edited) = edit_of(only(&widened));
        assert_eq!(edited.description(), "Woolworths\nMetro");
        assert_eq!(edited.metadata(), tx.metadata());
        let kept = edited.postings().get(1).expect("costed leg");
        assert_eq!(kept.cost().and_then(Cost::label), Some("lot\r\none"));
        assert_eq!(
            kept.metadata(),
            &Metadata::new(vec![text("note", "leg\nnote")])
        );
    }

    #[test]
    fn a_real_text_edit_beside_a_line_break_still_edits() {
        let l = ledger();
        let tx = stored(
            vec![leg(&l.bank, dec!(-50.00)), leg(&l.food, dec!(50.00))],
            Metadata::new(vec![text("receipt", "R-1\nR-2")]),
        );
        let mut raw = raw_from(&tx, Some("a1b"), &l);
        raw.metadata = vec![RawMetaEntry::resolved(
            key("receipt"),
            MetaValue::Text("R-1 R-3".to_owned()),
        )];
        let plan = run(&l, vec![raw], &[("a1b", &tx)], &[&tx], &[]).expect("plans");
        let (_, edited) = edit_of(only(&plan));
        assert_eq!(
            edited.metadata(),
            &Metadata::new(vec![text("receipt", "R-1 R-3")])
        );
    }

    #[rstest]
    #[case::none("plain", "plain")]
    #[case::newline("a\nb", "a b")]
    #[case::carriage_return("a\rb", "a b")]
    #[case::crlf("a\r\nb", "a  b")]
    #[case::trailing("end\n", "end ")]
    fn line_breaks_flatten_as_the_beancount_writer_spells_them(
        #[case] stored: &str,
        #[case] written: &str,
    ) {
        assert_eq!(flattened(stored), written);
    }

    // MARK: Create

    #[test]
    fn no_handle_creates() {
        let l = ledger();
        let raw = RawTransaction::builder()
            .date(day())
            .description("Coles")
            .postings(vec![
                raw_leg("Assets:Bank", dec!(-12.00)),
                raw_leg("Expenses:Food", dec!(12.00)),
            ])
            .build();
        let plan = run(&l, vec![raw], &[], &[], &[]).expect("plans");
        let Action::Create {
            edited, raw_index, ..
        } = only(&plan)
        else {
            panic!("expected a create");
        };
        assert_eq!(*raw_index, 0);
        assert_eq!(edited.description(), "Coles");
        assert_eq!(edited.reconciliation(), Reconciliation::Flagged);
        assert_eq!(edited.postings().len(), 2);
    }
}
