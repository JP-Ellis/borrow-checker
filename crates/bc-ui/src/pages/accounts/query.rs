//! Pure query helpers for the accounts register: building the effective search
//! filter (date range resolved) and narrowing results to the viewed account.
//! Kept target-agnostic so it is native-testable.

use bc_ipc::Filter;
use bc_ipc::Period;
use bc_ipc::Transaction;
use jiff::civil::Date;

use crate::components::period_nav::period_end;

/// Builds the filter actually sent to `search_transactions` for the register.
///
/// The global filter's `accounts` dimension is kept and intersected with the
/// sidebar account: the backend narrows to rows touching a filter account (and
/// attributes those legs as matched), and [`touches_account`] further narrows
/// to the viewed account client-side. The date range is resolved as: if the
/// user filter sets either date bound, keep it verbatim (the `PeriodNav` window
/// is overridden); otherwise inject the half-open window
/// `[window_start, period_end)`.
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

/// Returns `true` when the filter carries any non-date dimension
/// (`accounts` / `tags` / `text` / `amount` / `reconciliation`).
///
/// Date-only filters are excluded: dates already flow to the stats path through
/// the explicit window arguments, so a date-only filter takes the unfiltered
/// fast path.
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn filter_has_non_date_dim(filter: &Filter) -> bool {
    !filter.accounts.is_empty()
        || !filter.tags.is_empty()
        || filter.text.is_some()
        || filter.amount.is_some()
        || filter.reconciliation.is_some()
}

/// The overarching sparkline span length for a `PeriodNav` view at `period`.
///
/// The trend shows the current period plus a few of context, scaled by
/// granularity, so that the finer bucketing (see
/// [`bc_ipc::sparkline_bucketing_for`]) yields a readable density.
fn nav_span_len(period: &Period) -> jiff::Span {
    match period {
        Period::Fortnightly => jiff::Span::new().weeks(8_i64),
        Period::Monthly => jiff::Span::new().weeks(13_i64),
        Period::Quarterly | Period::FinancialQuarter { .. } => jiff::Span::new().months(6_i64),
        Period::CalendarYear | Period::FinancialYear { .. } => jiff::Span::new().months(12_i64),
        // Daily, Weekly, and future variants: a two-week window of context.
        Period::Daily | Period::Weekly | &_ => jiff::Span::new().days(14_i64),
    }
}

/// Resolves the overarching sparkline span `[start, end)` for the active filter.
///
/// Stage 1 of the span-driven sparkline bucketing. The source depends on whether
/// the filter carries a date bound:
///
/// * both bounds → the exact filter range;
/// * `after:` only → `[date_from, nav_end)`;
/// * `before:` only → a nav-length span ending at `date_until`;
/// * no date bound → the `PeriodNav` span ending at the page window end.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `period` - The page period granularity.
/// * `window_start` - The page display-window start.
///
/// # Returns
///
/// The `[start, end)` span to bucket.
#[must_use]
pub fn sparkline_span(user: &Filter, period: &Period, window_start: Date) -> (Date, Date) {
    let nav_end = period_end(period, window_start);
    match (user.date_from, user.date_until) {
        (Some(from), Some(until)) => (from, until),
        (Some(from), None) => (from, nav_end),
        (None, Some(until)) => (until.saturating_sub(nav_span_len(period)), until),
        (None, None) => (nav_end.saturating_sub(nav_span_len(period)), nav_end),
    }
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
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::effective_filter;
    use super::touches_account;

    #[test]
    fn keeps_accounts_and_injects_window_when_no_date_bound() {
        let mut user = bc_ipc::Filter::default();
        user.accounts = vec!["a1".to_owned()];
        user.text = Some("coles".to_owned());

        let eff = effective_filter(&user, &Period::Monthly, Date::constant(2026, 6, 1));

        /* Account dimension is preserved so it intersects with the sidebar. */
        assert_eq!(eff.accounts, vec!["a1".to_owned()]);
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

    #[test]
    fn non_date_dim_detection() {
        use super::filter_has_non_date_dim;

        let empty = bc_ipc::Filter::default();
        assert!(!filter_has_non_date_dim(&empty));

        let mut date_only = bc_ipc::Filter::default();
        date_only.date_from = Some(Date::constant(2026, 1, 1));
        assert!(!filter_has_non_date_dim(&date_only));

        let mut tagged = bc_ipc::Filter::default();
        tagged.tags = vec!["t1".to_owned()];
        assert!(filter_has_non_date_dim(&tagged));

        let mut texted = bc_ipc::Filter::default();
        texted.text = Some("coles".to_owned());
        assert!(filter_has_non_date_dim(&texted));
    }

    #[test]
    fn sparkline_span_nav_source_reproduces_year_density() {
        let user = bc_ipc::Filter::default();
        /* CalendarYear window starting 2025-01-01. */
        let (start, end) =
            super::sparkline_span(&user, &bc_ipc::Period::CalendarYear, date(2025, 1, 1));
        assert_eq!(end, date(2026, 1, 1));
        /* 12-month span → Monthly × 12 via stage 2. */
        assert_eq!(
            bc_ipc::sparkline_bucketing_for(start, end),
            (bc_ipc::Period::Monthly, 12)
        );
    }

    #[test]
    fn sparkline_span_both_bounds_is_exact_filter_range() {
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 3, 1));
        user.date_until = Some(date(2025, 4, 15));
        let (start, end) = super::sparkline_span(&user, &bc_ipc::Period::Monthly, date(2025, 1, 1));
        assert_eq!((start, end), (date(2025, 3, 1), date(2025, 4, 15)));
    }

    #[test]
    fn sparkline_span_after_only_ends_at_nav_end() {
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 2, 10));
        let (start, end) = super::sparkline_span(&user, &bc_ipc::Period::Monthly, date(2025, 6, 1));
        assert_eq!(start, date(2025, 2, 10));
        assert_eq!(
            end,
            super::period_end(&bc_ipc::Period::Monthly, date(2025, 6, 1))
        );
    }

    #[test]
    fn sparkline_span_before_only_uses_nav_length_ending_at_until() {
        let mut user = bc_ipc::Filter::default();
        user.date_until = Some(date(2025, 6, 1));
        let (start, end) =
            super::sparkline_span(&user, &bc_ipc::Period::CalendarYear, date(2025, 1, 1));
        assert_eq!(end, date(2025, 6, 1));
        /* Year granularity → ~12-month lookback ending at `until`. */
        assert_eq!(start, date(2024, 6, 1));
    }
}
