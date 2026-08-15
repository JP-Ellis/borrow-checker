//! Category reporting: postings aggregated by account over a period and rolled
//! up the account tree.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use rust_decimal::Decimal;

use crate::BcResult;
use crate::search::TransactionQuery;

// MARK: Output

/// One account's contribution to a category report.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The account this row totals.
    pub account_id: AccountId,
    /// Colon-separated path from the account's root, e.g. `Income:Interest`.
    pub path: String,
    /// Depth in the account tree, counting from a root at zero.
    ///
    /// Every ancestor of a reported account is itself reported, so indenting on
    /// this never leaves a gap.
    pub depth: usize,
    /// Sum of postings directly to this account.
    pub own: Amount,
    /// `own` plus every descendant's `own`.
    pub rolled_up: Amount,
}

impl Row {
    /// Constructs a row directly.
    ///
    /// `#[non_exhaustive]` blocks struct-literal construction outside this
    /// crate, so callers that build a [`Row`] by hand — a CLI renderer's
    /// snapshot tests, say — go through this constructor instead.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account this row totals.
    /// * `path` - Colon-separated path from the account's root.
    /// * `depth` - Depth in the account tree, counting from a root at zero.
    /// * `own` - Sum of postings directly to this account.
    /// * `rolled_up` - `own` plus every descendant's `own`.
    ///
    /// # Returns
    ///
    /// The constructed [`Row`].
    #[inline]
    #[must_use]
    pub fn new(
        account_id: AccountId,
        path: String,
        depth: usize,
        own: Amount,
        rolled_up: Amount,
    ) -> Self {
        Self {
            account_id,
            path,
            depth,
            own,
            rolled_up,
        }
    }
}

/// A category report: per-account totals plus what could not be counted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Rows in pre-order, so a caller renders the tree by indenting on `depth`.
    pub rows: Vec<Row>,
    /// Legs matched by the query but not summed into any row.
    ///
    /// This counts legs excluded for a commodity mismatch (**excluded, never
    /// converted** — conversion is deferred to the FX work); elided legs
    /// whose transaction had a single-elided residual that carried no entry
    /// for the requested commodity; and elided legs whose residual could not
    /// be computed at all (e.g. an overflow while summing sibling legs). It
    /// does **not** count legs belonging to a transaction with more than one
    /// elided leg — those are counted only in [`Self::ambiguous_transactions`].
    /// A non-zero count must be surfaced to the user.
    pub excluded_postings: usize,
    /// Transactions carrying more than one elided leg, whose residual cannot
    /// be attributed to any single leg.
    ///
    /// Every elided leg belonging to such a transaction is dropped without
    /// incrementing [`Self::excluded_postings`]; this counter is the sole
    /// signal that money was left out for that reason.
    pub ambiguous_transactions: usize,
}

// MARK: Aggregation

/// Totals postings by account over the window and filters described by `query`.
///
/// Membership comes from [`crate::transaction::Service::search`], so the date,
/// account-subtree and tag semantics — including the posting-scoped tag rule —
/// are exactly those of a transaction search. Only legs the query actually
/// matched are summed, so a tag filter never leaks untagged legs into a total.
///
/// # Arguments
///
/// * `transactions` - Supplies query matching.
/// * `accounts` - Supplies the account tree for path and rollup construction.
/// * `query` - The window and filters to report over.
/// * `commodity` - Commodity code; legs in any other commodity are excluded.
///
/// # Returns
///
/// A [`Report`] whose rows are in pre-order.
///
/// # Errors
///
/// Returns [`crate::BcError`] on database or data-parse failure, or if a total
/// overflows [`Decimal`]'s range.
#[inline]
pub async fn category_totals(
    transactions: &crate::transaction::Service,
    accounts: &crate::account::Service,
    query: &TransactionQuery,
    commodity: &str,
) -> BcResult<Report> {
    let matched = transactions.search(query).await?;

    let mut own: HashMap<AccountId, Decimal> = HashMap::new();
    let mut excluded_postings = 0_usize;
    let mut ambiguous_transactions = 0_usize;

    for m in &matched {
        let tx = &m.transaction;
        let residual =
            crate::residual::residual_of(tx.postings().iter().map(bc_models::Posting::amount));
        if matches!(residual, Ok(crate::residual::Residual::Ambiguous)) {
            ambiguous_transactions = ambiguous_transactions.saturating_add(1);
        }

        for posting in tx.postings() {
            if !m.matched_postings.contains(posting.id()) {
                continue;
            }
            let value = if let Some(amount) = posting.amount() {
                if amount.commodity().as_str() != commodity {
                    excluded_postings = excluded_postings.saturating_add(1);
                    continue;
                }
                amount.value()
            } else {
                match residual.as_ref() {
                    Ok(crate::residual::Residual::Attributable(balances)) => {
                        let Some(value) = balances.get(commodity) else {
                            excluded_postings = excluded_postings.saturating_add(1);
                            continue;
                        };
                        value
                    }
                    // Already reflected once per transaction in
                    // `ambiguous_transactions` above; counting the leg here too
                    // would double-count a single dropped leg across both
                    // counters.
                    Ok(crate::residual::Residual::Ambiguous) => continue,
                    Ok(crate::residual::Residual::NotElided) | Err(_) => {
                        excluded_postings = excluded_postings.saturating_add(1);
                        continue;
                    }
                }
            };

            let entry = own.entry(posting.account_id().clone()).or_default();
            *entry = entry
                .checked_add(value)
                .ok_or_else(|| crate::BcError::BadData("category total overflow".into()))?;
        }
    }

    let rows = build_rows(accounts, &own, commodity).await?;

    Ok(Report {
        rows,
        excluded_postings,
        ambiguous_transactions,
    })
}

/// Maximum ancestor chain length walked when rolling totals up the tree.
///
/// A malformed `parent_id` cycle would otherwise loop forever. Real charts of
/// accounts are nowhere near this deep, so exceeding it means the account graph
/// is corrupt rather than merely deep.
const MAX_ACCOUNT_DEPTH: usize = 64;

/// Calls `visit` on `start` and then each of its ancestors, root-most last.
///
/// # Arguments
///
/// * `start` - The account to walk up from, visited first.
/// * `by_id` - Every account in the tree, keyed by id.
/// * `visit` - Called once per account in the chain.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if the chain exceeds
/// [`MAX_ACCOUNT_DEPTH`], which a `parent_id` cycle would do forever, or
/// whatever `visit` returns.
fn walk_ancestors<'a>(
    start: &'a AccountId,
    by_id: &HashMap<&'a AccountId, &'a bc_models::Account>,
    mut visit: impl FnMut(&'a AccountId) -> BcResult<()>,
) -> BcResult<()> {
    let mut cursor = Some(start);
    for _ in 0..MAX_ACCOUNT_DEPTH {
        let Some(id) = cursor else { return Ok(()) };
        visit(id)?;
        cursor = by_id.get(id).and_then(|a| a.parent_id());
    }
    Err(crate::BcError::BadData(format!(
        "account ancestry deeper than {MAX_ACCOUNT_DEPTH}; the parent chain is probably a cycle"
    )))
}

/// Builds pre-order report rows with paths, depths and rolled-up totals.
async fn build_rows(
    accounts: &crate::account::Service,
    own: &HashMap<AccountId, Decimal>,
    commodity: &str,
) -> BcResult<Vec<Row>> {
    let all = accounts.list_all().await?;

    let by_id: HashMap<&AccountId, &bc_models::Account> = all.iter().map(|a| (a.id(), a)).collect();

    let mut children: HashMap<Option<&AccountId>, Vec<&bc_models::Account>> = HashMap::new();
    for account in &all {
        children
            .entry(account.parent_id())
            .or_default()
            .push(account);
    }
    #[expect(
        clippy::iter_over_hash_type,
        reason = "each value's sibling list is sorted independently, so hashmap iteration order does not affect the result"
    )]
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| a.name().cmp(b.name()));
    }

    let mut rolled: HashMap<AccountId, Decimal> = HashMap::new();
    for account in &all {
        let Some(total) = own.get(account.id()).copied() else {
            continue;
        };
        walk_ancestors(account.id(), &by_id, |id| {
            let entry = rolled.entry(id.clone()).or_default();
            *entry = entry
                .checked_add(total)
                .ok_or_else(|| crate::BcError::BadData("rolled-up total overflow".into()))?;
            Ok(())
        })?;
    }

    // An account earns a row by having a non-zero rolled-up total, and so does
    // every one of its ancestors — including any whose own descendants cancel
    // out — so that a rendered subtree is never severed from its root.
    let mut visible: HashSet<AccountId> = HashSet::new();
    for account in &all {
        if rolled.get(account.id()).copied().unwrap_or(Decimal::ZERO) == Decimal::ZERO {
            continue;
        }
        walk_ancestors(account.id(), &by_id, |id| {
            visible.insert(id.clone());
            Ok(())
        })?;
    }

    let mut rows = Vec::new();
    let roots = children.get(&None).cloned().unwrap_or_default();
    for root in roots {
        emit(
            root, "", 0, &children, own, &rolled, &visible, commodity, &mut rows,
        );
    }
    Ok(rows)
}

/// Appends `account`'s row, then its subtree, in pre-order.
///
/// `visible` is closed under ancestry, so an account outside it can have no
/// visible descendant and its whole subtree is skipped.
#[expect(
    clippy::too_many_arguments,
    reason = "a recursive tree walk threading its accumulators; grouping them would obscure the recursion"
)]
fn emit(
    account: &bc_models::Account,
    parent_path: &str,
    depth: usize,
    children: &HashMap<Option<&AccountId>, Vec<&bc_models::Account>>,
    own: &HashMap<AccountId, Decimal>,
    rolled: &HashMap<AccountId, Decimal>,
    visible: &HashSet<AccountId>,
    commodity: &str,
    rows: &mut Vec<Row>,
) {
    if !visible.contains(account.id()) {
        return;
    }

    let path = if parent_path.is_empty() {
        account.name().to_owned()
    } else {
        format!("{parent_path}:{}", account.name())
    };

    rows.push(Row {
        account_id: account.id().clone(),
        path: path.clone(),
        depth,
        own: Amount::new(
            own.get(account.id()).copied().unwrap_or(Decimal::ZERO),
            commodity,
        ),
        rolled_up: Amount::new(
            rolled.get(account.id()).copied().unwrap_or(Decimal::ZERO),
            commodity,
        ),
    });

    let child_depth = depth.saturating_add(1);
    if let Some(kids) = children.get(&Some(account.id())) {
        for kid in kids {
            emit(
                kid,
                &path,
                child_depth,
                children,
                own,
                rolled,
                visible,
                commodity,
                rows,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::category_totals;
    use crate::search::TransactionQuery;

    /// Builds a two-leg transaction in `commodity`, debiting `debit` and
    /// crediting `credit`.
    fn tx(
        debit: &AccountId,
        credit: &AccountId,
        d: Date,
        value: rust_decimal::Decimal,
        commodity: &str,
    ) -> Transaction {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "negating a test fixture's Decimal magnitude to build the offsetting leg"
        )]
        let opposite = -value;
        Transaction::builder()
            .id(TransactionId::new())
            .date(d)
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(debit.clone())
                    .amount(Amount::new(value, CommodityCode::new(commodity)))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(credit.clone())
                    .amount(Amount::new(opposite, CommodityCode::new(commodity)))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sums_matched_legs_per_account(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &interest, date(2025, 8, 1), dec!(100), "AUD"))
            .await
            .expect("t1");
        txns.create(tx(&bank, &interest, date(2025, 9, 1), dec!(50), "AUD"))
            .await
            .expect("t2");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let interest_row = report
            .rows
            .iter()
            .find(|r| r.account_id == interest)
            .expect("interest row");
        assert_eq!(interest_row.own.value(), dec!(-150));

        let bank_row = report
            .rows
            .iter()
            .find(|r| r.account_id == bank)
            .expect("bank row");
        assert_eq!(bank_row.own.value(), dec!(150));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn other_commodities_are_excluded_and_counted(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &interest, date(2025, 8, 1), dec!(100), "AUD"))
            .await
            .expect("aud");
        txns.create(tx(&bank, &interest, date(2025, 8, 2), dec!(70), "USD"))
            .await
            .expect("usd");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let interest_row = report
            .rows
            .iter()
            .find(|r| r.account_id == interest)
            .expect("interest row");
        assert_eq!(interest_row.own.value(), dec!(-100));
        assert_eq!(report.excluded_postings, 2, "both USD legs are excluded");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn accounts_with_no_activity_are_omitted(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");
        let unused = accts
            .create()
            .name("Unused")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("unused");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &interest, date(2025, 8, 1), dec!(100), "AUD"))
            .await
            .expect("t1");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        assert!(
            !report.rows.iter().any(|r| r.account_id == unused),
            "an account with no activity must not appear"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_window_yields_no_rows(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &interest, date(2025, 8, 1), dec!(100), "AUD"))
            .await
            .expect("t1");

        let query = TransactionQuery {
            date_from: Some(date(2030, 1, 1)),
            date_until: Some(date(2031, 1, 1)),
            ..TransactionQuery::default()
        };
        let report = category_totals(&txns, &accts, &query, "AUD")
            .await
            .expect("report");

        assert!(report.rows.is_empty());
        assert_eq!(report.excluded_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn totals_roll_up_the_account_tree_in_pre_order(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let income = accts
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&income)
            .call()
            .await
            .expect("interest");
        let one = accts
            .create()
            .name("Bank-One")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&interest)
            .call()
            .await
            .expect("one");
        let two = accts
            .create()
            .name("Bank-Two")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&interest)
            .call()
            .await
            .expect("two");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &one, date(2025, 8, 1), dec!(30), "AUD"))
            .await
            .expect("t1");
        txns.create(tx(&bank, &two, date(2025, 9, 1), dec!(12), "AUD"))
            .await
            .expect("t2");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let income_rows: Vec<_> = report
            .rows
            .iter()
            .filter(|r| r.path.starts_with("Income"))
            .collect();

        let shape: Vec<(&str, usize, Decimal, Decimal)> = income_rows
            .iter()
            .map(|r| (r.path.as_str(), r.depth, r.own.value(), r.rolled_up.value()))
            .collect();

        assert_eq!(
            shape,
            vec![
                ("Income", 0, dec!(0), dec!(-42)),
                ("Income:Interest", 1, dec!(0), dec!(-42)),
                ("Income:Interest:Bank-One", 2, dec!(-30), dec!(-30)),
                ("Income:Interest:Bank-Two", 2, dec!(-12), dec!(-12)),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_parent_with_no_own_activity_is_still_emitted(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let income = accts
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");
        let rent = accts
            .create()
            .name("Rent")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&income)
            .call()
            .await
            .expect("rent");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &rent, date(2025, 8, 1), dec!(500), "AUD"))
            .await
            .expect("t1");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let parent = report
            .rows
            .iter()
            .find(|r| r.path == "Income")
            .expect("parent row must be emitted to keep the tree connected");
        assert_eq!(parent.own.value(), dec!(0));
        assert_eq!(parent.rolled_up.value(), dec!(-500));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_parent_whose_children_cancel_is_still_emitted(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let income = accts
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");
        let rent = accts
            .create()
            .name("Rent")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&income)
            .call()
            .await
            .expect("rent");
        let refunds = accts
            .create()
            .name("Refunds")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&income)
            .call()
            .await
            .expect("refunds");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &rent, date(2025, 8, 1), dec!(500), "AUD"))
            .await
            .expect("t1");
        txns.create(tx(&refunds, &bank, date(2025, 8, 2), dec!(500), "AUD"))
            .await
            .expect("t2");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let shape: Vec<(&str, usize, Decimal)> = report
            .rows
            .iter()
            .map(|r| (r.path.as_str(), r.depth, r.rolled_up.value()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("Income", 0, dec!(0)),
                ("Income:Refunds", 1, dec!(500)),
                ("Income:Rent", 1, dec!(-500)),
            ],
            "a parent whose children cancel to zero still anchors them at depth 0, \
             while a childless account that nets to zero stays pruned"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_tag_filter_narrows_the_totals(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let expenses = accts
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("expenses");

        let tags = crate::tag::Service::new(pool.clone());
        let deductible = tags
            .create_path(&"deductible".parse().expect("path"))
            .await
            .expect("tag");

        let txns = crate::transaction::Service::new(pool.clone());

        let tagged = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 8, 1))
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(expenses.clone())
                    .amount(Amount::new(dec!(200), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bank.clone())
                    .amount(Amount::new(dec!(-200), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .tag_ids(vec![deductible.clone()])
            .created_at(Timestamp::now())
            .build();
        txns.create(tagged).await.expect("tagged");

        txns.create(tx(&expenses, &bank, date(2025, 8, 2), dec!(75), "AUD"))
            .await
            .expect("untagged");

        let query = TransactionQuery {
            tags: vec![deductible],
            ..TransactionQuery::default()
        };
        let report = category_totals(&txns, &accts, &query, "AUD")
            .await
            .expect("report");

        let row = report
            .rows
            .iter()
            .find(|r| r.path == "Expenses")
            .expect("expenses row");
        assert_eq!(
            row.own.value(),
            dec!(200),
            "the untagged transaction must not contribute"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_middle_ancestor_whose_children_cancel_is_emitted(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let income = accts
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");
        let middle = accts
            .create()
            .name("Middle")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&income)
            .call()
            .await
            .expect("middle");
        let plus = accts
            .create()
            .name("Plus")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&middle)
            .call()
            .await
            .expect("plus");
        let minus = accts
            .create()
            .name("Minus")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .parent_id(&middle)
            .call()
            .await
            .expect("minus");

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(tx(&bank, &income, date(2025, 8, 1), dec!(100), "AUD"))
            .await
            .expect("income activity");
        txns.create(tx(&bank, &plus, date(2025, 8, 2), dec!(30), "AUD"))
            .await
            .expect("plus activity");
        txns.create(tx(&minus, &bank, date(2025, 8, 3), dec!(30), "AUD"))
            .await
            .expect("minus activity");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let income_rows: Vec<_> = report
            .rows
            .iter()
            .filter(|r| r.path.starts_with("Income"))
            .collect();

        let shape: Vec<(&str, usize, Decimal, Decimal)> = income_rows
            .iter()
            .map(|r| (r.path.as_str(), r.depth, r.own.value(), r.rolled_up.value()))
            .collect();

        assert_eq!(
            shape,
            vec![
                ("Income", 0, dec!(-100), dec!(-100)),
                ("Income:Middle", 1, dec!(0), dec!(0)),
                ("Income:Middle:Minus", 2, dec!(30), dec!(30)),
                ("Income:Middle:Plus", 2, dec!(-30), dec!(-30)),
            ],
            "Middle rolls up to zero but still anchors Plus and Minus, which \
             would otherwise be rendered as children of Income"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_elided_leg_resolves_via_the_residual(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");

        let elided = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 8, 1))
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bank.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(interest.clone())
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();

        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(elided).await.expect("elided");

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        let interest_row = report
            .rows
            .iter()
            .find(|r| r.account_id == interest)
            .expect("interest row");
        assert_eq!(
            interest_row.own.value(),
            dec!(-100),
            "the elided leg must resolve to the residual, not error or vanish"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_elided_legs_are_ambiguous_and_contribute_nothing(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank-A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let interest = accts
            .create()
            .name("Interest")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("interest");
        let fees = accts
            .create()
            .name("Fees")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("fees");

        // `transaction::Service::create` rejects two-or-more-elided-leg
        // transactions outright, so the only way to exercise
        // `Residual::Ambiguous` is to insert the rows directly.
        let tx_id = TransactionId::new();
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, '2025-08-01', 'AMBIGUOUS', 'reconciled', '2025-08-01T00:00:00Z')",
        )
        .bind(tx_id.to_string())
        .execute(&pool)
        .await
        .expect("insert transaction");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, '100.00', 'AUD', 0)",
        )
        .bind(PostingId::new().to_string())
        .bind(tx_id.to_string())
        .bind(bank.to_string())
        .execute(&pool)
        .await
        .expect("insert p_bank");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, NULL, NULL, 1)",
        )
        .bind(PostingId::new().to_string())
        .bind(tx_id.to_string())
        .bind(interest.to_string())
        .execute(&pool)
        .await
        .expect("insert p_interest (elided)");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, NULL, NULL, 2)",
        )
        .bind(PostingId::new().to_string())
        .bind(tx_id.to_string())
        .bind(fees.to_string())
        .execute(&pool)
        .await
        .expect("insert p_fees (elided)");

        let txns = crate::transaction::Service::new(pool.clone());

        let report = category_totals(&txns, &accts, &TransactionQuery::default(), "AUD")
            .await
            .expect("report");

        assert_eq!(
            report.ambiguous_transactions, 1,
            "a transaction with two elided legs must be counted as ambiguous"
        );
        assert_eq!(
            report.excluded_postings, 0,
            "the ambiguous legs are reflected via ambiguous_transactions, not excluded_postings"
        );
        assert!(
            !report.rows.iter().any(|r| r.account_id == interest),
            "an elided leg from an ambiguous transaction must not appear in any row"
        );
        assert!(
            !report.rows.iter().any(|r| r.account_id == fees),
            "an elided leg from an ambiguous transaction must not appear in any row"
        );

        let bank_row = report
            .rows
            .iter()
            .find(|r| r.account_id == bank)
            .expect("bank row");
        assert_eq!(
            bank_row.own.value(),
            dec!(100),
            "the one concrete leg still contributes its own amount"
        );
    }
}
