//! Pure query helpers for the accounts register: building the effective search
//! filter (account scope stripped, date range resolved) and narrowing results
//! to the viewed account. Kept target-agnostic so it is native-testable.

use bc_ipc::Filter;
use bc_ipc::Period;
use bc_ipc::Transaction;
use jiff::civil::Date;

use crate::components::period_nav::period_end;

/// Builds the filter actually sent to `search_transactions` for the register.
///
/// The global filter's `accounts` dimension is dropped — on an account page the
/// sidebar selection is the account authority, so account scoping happens via
/// [`touches_account`], not as a leg-greying predicate. The date range is
/// resolved as: if the user filter sets either date bound, keep it verbatim
/// (the `PeriodNav` window is overridden); otherwise inject the half-open
/// window `[window_start, period_end)`.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `period` - The register's period granularity.
/// * `window_start` - The register's display-window start.
///
/// # Returns
///
/// The effective filter to search with.
#[must_use]
pub fn effective_filter(user: &Filter, period: &Period, window_start: Date) -> Filter {
    let mut eff = user.clone();
    eff.accounts = Vec::new();
    if eff.date_from.is_none() && eff.date_until.is_none() {
        eff.date_from = Some(window_start);
        eff.date_until = Some(period_end(period, window_start));
    }
    eff
}

/// Returns `true` when `tx` has at least one posting on `account_id`.
///
/// # Arguments
///
/// * `tx` - The transaction to test.
/// * `account_id` - The viewed account id.
#[must_use]
pub fn touches_account(tx: &Transaction, account_id: &str) -> bool {
    tx.postings.iter().any(|p| p.account.id == account_id)
}

#[cfg(test)]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::Period;
    use bc_ipc::Posting;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::effective_filter;
    use super::touches_account;

    #[test]
    fn strips_accounts_and_injects_window_when_no_date_bound() {
        let mut user = bc_ipc::Filter::default();
        user.accounts = vec!["a1".to_owned()];
        user.text = Some("coles".to_owned());

        let eff = effective_filter(&user, &Period::Monthly, Date::constant(2026, 6, 1));

        assert!(eff.accounts.is_empty());
        assert_eq!(eff.text.as_deref(), Some("coles"));
        assert_eq!(eff.date_from, Some(Date::constant(2026, 6, 1)));
        /* Monthly period_end is exclusive: first day of next month. */
        assert_eq!(eff.date_until, Some(Date::constant(2026, 7, 1)));
    }

    #[test]
    fn keeps_filter_dates_and_ignores_window() {
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(Date::constant(2026, 3, 10));

        let eff = effective_filter(&user, &Period::Monthly, Date::constant(2026, 6, 1));

        /* Filter date present → window is NOT injected on either side. */
        assert_eq!(eff.date_from, Some(Date::constant(2026, 3, 10)));
        assert_eq!(eff.date_until, None);
    }

    /// Builds a two-posting transaction, one leg on `account_id`, one on
    /// `"other-account"`.
    fn transaction_with_posting_on(account_id: &str) -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 6, 1),
            "Payee",
            "",
            None::<&str>,
            vec![],
            Reconciliation::Reconciled,
            vec![],
            vec![
                Posting::new(
                    "posting-1",
                    AccountRef::new(account_id, "Account One"),
                    Some(Amount::new(Decimal::new(-1_000, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                Posting::new(
                    "posting-2",
                    AccountRef::new("other-account", "Account Two"),
                    Some(Amount::new(Decimal::new(1_000, 2), "AUD")),
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
    fn touches_account_true_when_posting_matches() {
        let tx = transaction_with_posting_on("cb-smart-access");

        assert!(touches_account(&tx, "cb-smart-access"));
    }

    #[test]
    fn touches_account_false_when_no_posting_matches() {
        let tx = transaction_with_posting_on("cb-smart-access");

        assert!(!touches_account(&tx, "groceries"));
    }
}
