//! Pure filter-shaping helpers for the budget page (native-testable).

/// Returns a copy of `user` with the date and balance dimensions cleared.
///
/// Budgets are period-gridded and driven by `PeriodNav`, so date bounds never
/// reach the budget backend. Budget actuals assume double entry, which an
/// unbalanced transaction violates, so balance status is inert too.
///
/// # Arguments
///
/// * `user` - The active global filter.
#[must_use]
pub fn budget_effective_filter(user: &bc_ipc::Filter) -> bc_ipc::Filter {
    let mut eff = user.clone();
    eff.date_from = None;
    eff.date_until = None;
    eff.balance = None;
    eff
}

/// Returns whether `filter` sets any date bound or a balance status (used to
/// show the inert-dimension hint).
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn inert_filter_active(filter: &bc_ipc::Filter) -> bool {
    filter.date_from.is_some() || filter.date_until.is_some() || filter.balance.is_some()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn strips_date_keeps_other_dims() {
        let mut f = bc_ipc::Filter::default();
        f.date_from = Some(jiff::civil::date(2026, 6, 1));
        f.date_until = Some(jiff::civil::date(2026, 6, 30));
        f.text = Some("rent".to_owned());

        let eff = super::budget_effective_filter(&f);
        assert_eq!(eff.date_from, None);
        assert_eq!(eff.date_until, None);
        assert_eq!(eff.text.as_deref(), Some("rent"));
        assert!(super::inert_filter_active(&f));
        assert!(!super::inert_filter_active(&eff));
    }

    #[test]
    fn strips_balance_keeps_other_dims() {
        let mut f = bc_ipc::Filter::default();
        f.balance = Some(bc_ipc::BalanceStatus::Unbalanced);
        f.text = Some("rent".to_owned());

        let eff = super::budget_effective_filter(&f);
        assert_eq!(eff.balance, None);
        assert_eq!(eff.text.as_deref(), Some("rent"));
        assert!(super::inert_filter_active(&f));
        assert!(!super::inert_filter_active(&eff));
    }
}
