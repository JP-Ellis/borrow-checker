//! Category reporting: postings aggregated by account over a period and rolled
//! up the account tree.

use std::collections::HashMap;

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
    /// Number of this row's ancestors that also appear in the report.
    pub depth: usize,
    /// Sum of postings directly to this account.
    pub own: Amount,
    /// `own` plus every descendant's `own`.
    pub rolled_up: Amount,
}

/// A category report: per-account totals plus what could not be counted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Rows in pre-order, so a caller renders the tree by indenting on `depth`.
    pub rows: Vec<Row>,
    /// Legs skipped because their commodity is not the requested one.
    ///
    /// These are **excluded, never converted** — conversion is deferred to the
    /// FX work. A non-zero count must be surfaced to the user.
    pub excluded_postings: usize,
    /// Transactions carrying more than one elided leg, whose residual cannot be
    /// attributed to any single leg.
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
                let Ok(crate::residual::Residual::Attributable(ref balances)) = residual else {
                    continue;
                };
                let Some(value) = balances.get(commodity) else {
                    continue;
                };
                value
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

/// Turns per-account totals into report rows.
///
/// Task 5 replaces this with a tree walk; for now each account with a non-zero
/// total becomes a flat, depth-zero row named by the account itself.
async fn build_rows(
    accounts: &crate::account::Service,
    own: &HashMap<AccountId, Decimal>,
    commodity: &str,
) -> BcResult<Vec<Row>> {
    let all = accounts.list_all().await?;
    let mut rows: Vec<Row> = all
        .iter()
        .filter_map(|account| {
            let total = own.get(account.id()).copied()?;
            if total == Decimal::ZERO {
                return None;
            }
            Some(Row {
                account_id: account.id().clone(),
                path: account.name().to_owned(),
                depth: 0,
                own: Amount::new(total, commodity),
                rolled_up: Amount::new(total, commodity),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
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
            .payee("Payee".to_owned())
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
            .payee("Payee".to_owned())
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
}
