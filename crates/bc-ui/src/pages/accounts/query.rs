//! Pure query helpers for the accounts register: building the effective search
//! filter (date range resolved) for the register backend and the sparkline.
//! Kept target-agnostic so it is native-testable.

use bc_ipc::Filter;
use bc_ipc::Period;
use jiff::civil::Date;

use crate::components::period_nav::DisplayWindow;
use crate::components::period_nav::window_containing;

/// Builds the filter sent alongside `RegisterRequest`.
///
/// The global filter's `accounts` dimension is kept: the backend intersects it
/// with the viewed account (and its subtree, when roll-up is on) and scopes
/// the page server-side. The date range is resolved as: if the user filter
/// sets either date bound, keep it verbatim (the display window is
/// overridden); otherwise inject `window`'s bounds (`None` for both in all
/// time).
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The register's display window.
///
/// # Returns
///
/// The effective filter to search with.
#[must_use]
pub fn effective_filter(user: &Filter, window: &DisplayWindow) -> Filter {
    let mut eff = user.clone();
    if eff.date_from.is_none() && eff.date_until.is_none() {
        let (from, until) = window.bounds();
        eff.date_from = from;
        eff.date_until = until;
    }
    eff
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

/// `true` while a window-tagged resource does not yet answer for `current`.
///
/// A `LocalResource` keeps serving its previous value while a refetch is in
/// flight, so the page tags each result with the window it was fetched for
/// and compares that tag here. Nothing loaded is busy; an error is not, as
/// nothing further will arrive for it.
///
/// # Arguments
///
/// * `loaded` - The resource's current value, if any.
/// * `current` - The page's display window.
#[must_use]
pub fn awaiting_window<T, E>(
    loaded: Option<&Result<(DisplayWindow, T), E>>,
    current: &DisplayWindow,
) -> bool {
    match loaded {
        None => true,
        Some(Err(_)) => false,
        Some(Ok((fetched, _))) => fetched != current,
    }
}

/// Context length of the sparkline for a window; all time takes the monthly span.
fn nav_span_len(window: &DisplayWindow) -> jiff::Span {
    match window.period() {
        Some(Period::Fortnightly) => jiff::Span::new().weeks(8_i64),
        Some(Period::Quarterly | Period::FinancialQuarter { .. }) => {
            jiff::Span::new().months(6_i64)
        }
        Some(Period::CalendarYear | Period::FinancialYear { .. }) => {
            jiff::Span::new().months(12_i64)
        }
        Some(Period::Daily | Period::Weekly) => jiff::Span::new().days(14_i64),
        // Monthly, all time, and future #[non_exhaustive] variants.
        _ => jiff::Span::new().weeks(13_i64),
    }
}

/// Exclusive end of the nav span: the window end, or tomorrow for all time.
fn nav_end(window: &DisplayWindow, today: Date) -> Date {
    window
        .bounds()
        .1
        .unwrap_or_else(|| today.saturating_add(jiff::Span::new().days(1_i64)))
}

/// Resolves the overarching sparkline span `[start, end)` for the active filter.
///
/// Stage 1 of the span-driven sparkline bucketing. The source depends on whether
/// the filter carries a date bound:
///
/// * both bounds → the exact filter range;
/// * `after:` only → `[date_from, nav_end)`;
/// * `before:` only → a nav-length span ending at `date_until`;
/// * no date bound, all time → `[first_activity, nav_end)` (or the fallback
///   nav-length span when there is no activity yet);
/// * no date bound, a period window → the `PeriodNav` span ending at the
///   window end.
///
/// Nothing constrains `date_from <= date_until`, so an inverted filter range
/// yields an inverted (empty) span. Callers must treat `start >= end` as
/// "matches nothing"; [`sparkline_bucketing`] does exactly that.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The page's display window.
/// * `first_activity` - Earliest transaction date of the viewed account scope
///   (the account, plus its descendants under roll-up), if any.
/// * `today` - Today's date; only consulted for an unbounded all-time span.
///
/// # Returns
///
/// The `[start, end)` span to bucket.
#[must_use]
pub fn sparkline_span(
    user: &Filter,
    window: &DisplayWindow,
    first_activity: Option<Date>,
    today: Date,
) -> (Date, Date) {
    let end = nav_end(window, today);
    match (user.date_from, user.date_until, window, first_activity) {
        (Some(from), Some(until), _, _) => (from, until),
        (Some(from), None, _, _) => (from, end),
        (None, Some(until), _, _) => (until.saturating_sub(nav_span_len(window)), until),
        (None, None, DisplayWindow::AllTime, Some(first)) => (first, end),
        (None, None, _, _) => (end.saturating_sub(nav_span_len(window)), end),
    }
}

/// Number of `bucket`-wide buckets needed for the oldest calendar-snapped bucket
/// to reach back to (or before) `span_start`, with the newest bucket containing
/// `as_of`.
///
/// Mirrors `bc_core::balance::bucket_ranges`: buckets are snapped to calendar
/// boundaries and the newest one contains `as_of`, so the oldest may start
/// before `span_start` (an accepted partial oldest bucket). Walks bucket
/// boundaries backward from `as_of` until coverage reaches `span_start`, reusing
/// [`window_containing`] for the DTO [`Period`] calendar math.
///
/// # Arguments
///
/// * `bucket` - The bucket granularity chosen by stage 2.
/// * `span_start` - Inclusive start the oldest bucket must reach.
/// * `as_of` - Reference date the newest bucket contains.
///
/// # Returns
///
/// The bucket count (always `>= 1`).
fn coverage_count(bucket: &Period, span_start: Date, as_of: Date) -> u32 {
    let mut count: u32 = 1;
    let mut cur_start = window_containing(bucket, as_of);
    while cur_start > span_start {
        let prev_day = cur_start.saturating_sub(jiff::Span::new().days(1_i64));
        cur_start = window_containing(bucket, prev_day);
        count = count.saturating_add(1);
    }
    count
}

/// Resolves the sparkline `(bucket, count, span_end)` for the active filter.
///
/// Composes stage 1 ([`sparkline_span`]) and stage 2
/// ([`bc_ipc::sparkline_bucketing_for`]). For an **unaligned** span (either
/// filter date bound set, or the all-time window) the nominal count is bumped
/// via [`coverage_count`] so the oldest calendar-snapped bucket reaches the
/// resolved span start (`date_from`, the nav-length lookback from
/// `date_until`, or `first_activity` for all time); without the bump the
/// leading postings would fall outside every bucket and be dropped from the
/// date-clamped fetch, desyncing the sparkline from the balance tiles.
/// `PeriodNav` spans are calendar-aligned and keep the nominal stage-2 count
/// unchanged.
///
/// An inverted filter range (`date_from > date_until`) matches nothing, so the
/// count is `0` and callers must render an empty sparkline rather than fetching.
///
/// The corrected count flows to both the dashboard title and the client fetch,
/// keeping the rendered bar count and the title label in sync.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The page's display window.
/// * `first_activity` - Earliest transaction date of the viewed account scope
///   (the account, plus its descendants under roll-up), if any.
/// * `today` - Today's date; only consulted for an unbounded all-time span.
///
/// # Returns
///
/// The `(bucket, count, span_end)` triple to fetch and render; `count == 0`
/// means the span is empty and nothing should be fetched or drawn.
#[must_use]
pub fn sparkline_bucketing(
    user: &Filter,
    window: &DisplayWindow,
    first_activity: Option<Date>,
    today: Date,
) -> (Period, u32, Date) {
    let (span_start, span_end) = sparkline_span(user, window, first_activity, today);
    if span_start >= span_end {
        return (Period::Daily, 0, span_end);
    }
    let (bucket, nominal) = bc_ipc::sparkline_bucketing_for(span_start, span_end);
    let unaligned = user.date_from.is_some()
        || user.date_until.is_some()
        || matches!(window, DisplayWindow::AllTime);
    let count = if unaligned {
        let as_of = span_end.saturating_sub(jiff::Span::new().days(1_i64));
        nominal.max(coverage_count(&bucket, span_start, as_of))
    } else {
        nominal
    };
    (bucket, count, span_end)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Period;
    use jiff::Span;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::awaiting_window;
    use super::effective_filter;
    use super::sparkline_bucketing;
    use crate::components::period_nav::DisplayWindow;
    use crate::components::period_nav::period_end;
    use crate::components::period_nav::window_containing;

    /// Oldest calendar-snapped bucket start for `count` `bucket`-wide buckets whose
    /// newest contains `as_of` — mirrors `bc_core::balance::bucket_ranges`.
    fn oldest_bucket_start(bucket: &Period, count: u32, as_of: Date) -> Date {
        let mut start = window_containing(bucket, as_of);
        for _ in 1..count {
            let prev_day = start.saturating_sub(Span::new().days(1_i64));
            start = window_containing(bucket, prev_day);
        }
        start
    }

    /// Shorthand for a monthly [`DisplayWindow`] starting at `start`.
    fn monthly(start: Date) -> DisplayWindow {
        DisplayWindow::Period {
            period: Period::Monthly,
            start,
        }
    }

    #[test]
    fn keeps_accounts_and_injects_window_when_no_date_bound() {
        let mut user = bc_ipc::Filter::default();
        user.accounts = vec!["a1".to_owned()];
        user.text = Some("coles".to_owned());

        let eff = effective_filter(&user, &monthly(Date::constant(2026, 6, 1)));

        /* Account dimension is preserved so it intersects with the sidebar. */
        assert_eq!(eff.accounts, vec!["a1".to_owned()]);
        assert_eq!(eff.text.as_deref(), Some("coles"));
        assert_eq!(eff.date_from, Some(Date::constant(2026, 6, 1)));
        /* Monthly period_end is exclusive: first day of next month. */
        assert_eq!(eff.date_until, Some(Date::constant(2026, 7, 1)));
    }

    #[test]
    fn awaiting_window_until_the_loaded_tag_matches() {
        type Loaded = Result<(DisplayWindow, ()), ()>;
        let current = monthly(Date::constant(2026, 6, 1));
        let fresh: Loaded = Ok((current.clone(), ()));
        let stale: Loaded = Ok((DisplayWindow::AllTime, ()));
        let failed: Loaded = Err(());

        assert!(awaiting_window::<(), ()>(None, &current));
        assert!(awaiting_window(Some(&stale), &current));
        assert!(!awaiting_window(Some(&fresh), &current));
        /* An error will never be followed by a value for this window. */
        assert!(!awaiting_window(Some(&failed), &current));
    }

    #[test]
    fn all_time_injects_no_dates() {
        let mut user = bc_ipc::Filter::default();
        user.text = Some("coles".to_owned());
        let eff = effective_filter(&user, &DisplayWindow::AllTime);
        assert_eq!(eff.date_from, None);
        assert_eq!(eff.date_until, None);
        assert_eq!(eff.text.as_deref(), Some("coles"));
    }

    #[test]
    fn keeps_filter_dates_and_ignores_window() {
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(Date::constant(2026, 3, 10));

        let eff = effective_filter(&user, &monthly(Date::constant(2026, 6, 1)));

        /* Filter date present → window is NOT injected on either side. */
        assert_eq!(eff.date_from, Some(Date::constant(2026, 3, 10)));
        assert_eq!(eff.date_until, None);
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
        let window = DisplayWindow::Period {
            period: bc_ipc::Period::CalendarYear,
            start: date(2025, 1, 1),
        };
        let (start, end) = super::sparkline_span(&user, &window, None, date(2025, 1, 1));
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
        let (start, end) =
            super::sparkline_span(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));
        assert_eq!((start, end), (date(2025, 3, 1), date(2025, 4, 15)));
    }

    #[test]
    fn sparkline_span_after_only_ends_at_nav_end() {
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 2, 10));
        let (start, end) =
            super::sparkline_span(&user, &monthly(date(2025, 6, 1)), None, date(2025, 6, 1));
        assert_eq!(start, date(2025, 2, 10));
        assert_eq!(end, period_end(&bc_ipc::Period::Monthly, date(2025, 6, 1)));
    }

    #[test]
    fn sparkline_span_before_only_uses_nav_length_ending_at_until() {
        let mut user = bc_ipc::Filter::default();
        user.date_until = Some(date(2025, 6, 1));
        let window = DisplayWindow::Period {
            period: bc_ipc::Period::CalendarYear,
            start: date(2025, 1, 1),
        };
        let (start, end) = super::sparkline_span(&user, &window, None, date(2025, 1, 1));
        assert_eq!(end, date(2025, 6, 1));
        /* Year granularity → ~12-month lookback ending at `until`. */
        assert_eq!(start, date(2024, 6, 1));
    }

    #[test]
    fn all_time_span_is_first_activity_to_tomorrow() {
        let today = Date::constant(2026, 9, 20);
        let span = super::sparkline_span(
            &bc_ipc::Filter::default(),
            &DisplayWindow::AllTime,
            Some(Date::constant(2015, 3, 10)),
            today,
        );
        assert_eq!(
            span,
            (Date::constant(2015, 3, 10), Date::constant(2026, 9, 21))
        );
    }

    #[test]
    fn all_time_span_without_activity_falls_back_to_thirteen_weeks() {
        let today = Date::constant(2026, 9, 20);
        let span = super::sparkline_span(
            &bc_ipc::Filter::default(),
            &DisplayWindow::AllTime,
            None,
            today,
        );
        assert_eq!(
            span,
            (
                Date::constant(2026, 9, 21).saturating_sub(Span::new().weeks(13_i64)),
                Date::constant(2026, 9, 21)
            )
        );
    }

    #[test]
    fn all_time_bucketing_reaches_first_activity() {
        let today = Date::constant(2026, 9, 20);
        let first = Date::constant(2015, 3, 10);
        let (bucket, count, end) = sparkline_bucketing(
            &bc_ipc::Filter::default(),
            &DisplayWindow::AllTime,
            Some(first),
            today,
        );
        assert_eq!(bucket, Period::CalendarYear);
        assert_eq!(end, Date::constant(2026, 9, 21));
        assert!(oldest_bucket_start(&bucket, count, today) <= first);
    }

    #[test]
    fn explicit_filter_coverage_reaches_span_start() {
        /* Weekly case from the bug report: [2025-01-01, 2025-02-11) is 41 days →
         * stage 2 picks (Weekly, 6), but calendar snapping drops the oldest days.
         * The corrected count bumps to 7 so the oldest bucket reaches Jan 1. */
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 1, 1));
        user.date_until = Some(date(2025, 2, 11));

        let (bucket, count, span_end) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(bucket, bc_ipc::Period::Weekly);
        /* Nominal ceil(41/7) = 6 would undershoot; coverage bumps to 7. */
        assert_eq!(count, 7);
        let as_of = span_end.saturating_sub(Span::new().days(1_i64));
        assert!(oldest_bucket_start(&bucket, count, as_of) <= date(2025, 1, 1));
        /* Prove the nominal count really would have dropped the leading days. */
        assert!(oldest_bucket_start(&bucket, 6, as_of) > date(2025, 1, 1));
    }

    #[test]
    fn explicit_filter_coverage_reaches_span_start_monthly() {
        /* Monthly analogue: [2025-01-15, 2025-08-20) is 217 days → (Monthly, 7),
         * whose oldest bucket snaps to Feb 1 and drops the Jan 15–31 window. The
         * corrected count bumps to 8 so the oldest bucket reaches Jan 1 ≤ Jan 15. */
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 1, 15));
        user.date_until = Some(date(2025, 8, 20));

        let (bucket, count, span_end) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(bucket, bc_ipc::Period::Monthly);
        assert_eq!(count, 8);
        let as_of = span_end.saturating_sub(Span::new().days(1_i64));
        assert!(oldest_bucket_start(&bucket, count, as_of) <= date(2025, 1, 15));
        assert!(oldest_bucket_start(&bucket, 7, as_of) > date(2025, 1, 15));
    }

    #[test]
    fn nav_span_keeps_nominal_count() {
        /* No explicit date bound → nav path keeps the stage-2 nominal count
         * unchanged (calendar-aligned span already covers exactly). */
        let user = bc_ipc::Filter::default();
        let window = DisplayWindow::Period {
            period: bc_ipc::Period::CalendarYear,
            start: date(2025, 1, 1),
        };
        let (bucket, count, _) = sparkline_bucketing(&user, &window, None, date(2025, 1, 1));
        assert_eq!((bucket, count), (bc_ipc::Period::Monthly, 12));
    }

    /// The `PeriodNav` densities are emergent from two independent constants —
    /// [`super::nav_span_len`] and the `bc_ipc` threshold ladder — so they are
    /// pinned here for every [`Period`] variant.
    #[rstest]
    #[case(Period::Daily, Period::Daily, 14)]
    #[case(Period::Weekly, Period::Daily, 14)]
    #[case(Period::Fortnightly, Period::Weekly, 8)]
    #[case(Period::Monthly, Period::Weekly, 13)]
    #[case(Period::Quarterly, Period::Monthly, 6)]
    #[case(
        Period::FinancialQuarter { start_month: 7, start_day: 1 },
        Period::Monthly,
        6
    )]
    #[case(Period::CalendarYear, Period::Monthly, 12)]
    #[case(
        Period::FinancialYear { start_month: 7, start_day: 1 },
        Period::Monthly,
        12
    )]
    fn nav_densities_unchanged(
        #[case] period: Period,
        #[case] expected_bucket: Period,
        #[case] expected_count: u32,
    ) {
        let user = bc_ipc::Filter::default();
        let today = date(2025, 6, 15);
        let window = DisplayWindow::Period {
            start: window_containing(&period, today),
            period,
        };

        let (bucket, count, _) = sparkline_bucketing(&user, &window, None, today);

        assert_eq!((bucket, count), (expected_bucket, expected_count));
    }

    #[test]
    fn inverted_filter_range_yields_empty_bucketing() {
        /* `after:2025-06-01 before:2025-01-01` matches nothing, so the sparkline
         * must render explicitly empty rather than collapsing to a single bar. */
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 6, 1));
        user.date_until = Some(date(2025, 1, 1));

        let (bucket, count, span_end) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(count, 0);
        assert_eq!(bucket, Period::Daily);
        assert_eq!(span_end, date(2025, 1, 1));
    }

    #[test]
    fn equal_filter_bounds_yield_empty_bucketing() {
        /* A half-open span of zero length is equally empty. */
        let mut user = bc_ipc::Filter::default();
        user.date_from = Some(date(2025, 3, 1));
        user.date_until = Some(date(2025, 3, 1));

        let (_, count, _) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(count, 0);
    }
}
