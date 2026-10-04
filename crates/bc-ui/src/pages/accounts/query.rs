//! Pure query helpers for the accounts register: building the effective search
//! filter (date range resolved) for the register backend, the stats and the
//! sparkline. Kept target-agnostic so it is native-testable.

use bc_ipc::Filter;
use bc_ipc::Period;
use bc_query::print;
use bc_query::shape;
use jiff::civil::Date;

use crate::components::period_nav::DisplayWindow;
use crate::components::period_nav::window_containing;
use crate::filter_ctx::query_expr;

/// Whether the query has a `date` term anywhere, which takes over from the
/// display window.
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn query_sets_dates(filter: &Filter) -> bool {
    query_expr(filter).is_some_and(|expr| shape::mentions_builtin(&expr, "date"))
}

/// The bounds the query's top-level `date` terms set.
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn query_dates(filter: &Filter) -> (Option<Date>, Option<Date>) {
    query_expr(filter).map_or((None, None), |expr| {
        let window = shape::date_window(&expr);
        (window.from, window.until)
    })
}

/// Builds the filter sent alongside `RegisterRequest`: the user's query, plus
/// `window`'s bounds when the query has no `date` term anywhere.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The register's display window.
#[must_use]
pub fn effective_filter(user: &Filter, window: &DisplayWindow) -> Filter {
    let mut eff = user.clone();
    if !query_sets_dates(user) {
        let (from, until) = window.bounds();
        eff.date_from = from;
        eff.date_until = until;
    }
    eff
}

/// The stats window: the query's top-level `date` bounds when it sets any;
/// all time when its only `date` terms are nested; else the display window.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The page's display window.
#[must_use]
pub fn stats_window(user: &Filter, window: &DisplayWindow) -> (Option<Date>, Option<Date>) {
    let dates = query_dates(user);
    if dates != (None, None) {
        return dates;
    }
    if query_sets_dates(user) {
        return (None, None);
    }
    window.bounds()
}

/// The query without its top-level `date` terms, which the stats and
/// sparkline apply through their own window: the membership filter. `None`
/// when nothing else remains.
///
/// # Arguments
///
/// * `user` - The active global filter.
#[must_use]
pub fn membership_filter(user: &Filter) -> Option<Filter> {
    let expr = query_expr(user)?;
    let kept = shape::strip(&expr, |c| shape::is_builtin(c, "date")).kept?;
    Some(Filter::new(print(&kept), None, None))
}

/// Whether the filter narrows membership beyond its top-level dates.
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn filter_has_non_date_dim(filter: &Filter) -> bool {
    membership_filter(filter).is_some()
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
/// Stage 1 of the span-driven sparkline bucketing. The source depends on the
/// query's top-level `date` bounds:
///
/// * both bounds → the exact filter range;
/// * a lower bound only → `[from, nav_end)`;
/// * an upper bound only → a nav-length span ending at `until`;
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
/// * `first_activity` - The selected account's earliest activity date (and, with
///   roll-up, its descendants'), if any.
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
    let (date_from, date_until) = query_dates(user);
    // Nested-only `date` terms leave no top-level bounds: like `stats_window`,
    // that means all time rather than the display window.
    let nested_only = date_from.is_none() && date_until.is_none() && query_sets_dates(user);
    let effective = if nested_only {
        &DisplayWindow::AllTime
    } else {
        window
    };
    let end = nav_end(effective, today);
    match (date_from, date_until, effective, first_activity) {
        (Some(from), Some(until), _, _) => (from, until),
        (Some(from), None, _, _) => (from, end),
        (None, Some(until), _, _) => (until.saturating_sub(nav_span_len(effective)), until),
        (None, None, DisplayWindow::AllTime, Some(first)) => (first, end),
        (None, None, _, _) => (end.saturating_sub(nav_span_len(effective)), end),
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
/// top-level `date` bound set, or the all-time window) the nominal count is
/// bumped via [`coverage_count`] so the oldest calendar-snapped bucket reaches
/// the resolved span start (the lower bound, the nav-length lookback from the
/// upper bound, or `first_activity` for all time); without the bump the
/// leading postings would fall outside every bucket and be dropped from the
/// date-clamped fetch, desyncing the sparkline from the balance tiles.
/// `PeriodNav` spans are calendar-aligned and keep the nominal stage-2 count
/// unchanged.
///
/// An inverted filter range (lower bound after upper) matches nothing, so the
/// count is `0` and callers must render an empty sparkline rather than fetching.
///
/// The corrected count flows to both the dashboard title and the client fetch,
/// keeping the rendered bar count and the title label in sync.
///
/// # Arguments
///
/// * `user` - The active global filter.
/// * `window` - The page's display window.
/// * `first_activity` - The selected account's earliest activity date (and, with
///   roll-up, its descendants'), if any.
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
    let unaligned = query_dates(user) != (None, None)
        || query_sets_dates(user)
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
    use super::filter_has_non_date_dim;
    use super::membership_filter;
    use super::sparkline_bucketing;
    use super::sparkline_span;
    use super::stats_window;
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
        let user = bc_ipc::Filter::new("account:Assets:BankA coles", None, None);

        let eff = effective_filter(&user, &monthly(Date::constant(2026, 6, 1)));

        /* The account term is preserved; the server joins it per leg with the sidebar scope. */
        assert_eq!(eff.query, user.query);
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
        let user = bc_ipc::Filter::new("coles", None, None);
        let eff = effective_filter(&user, &DisplayWindow::AllTime);
        assert_eq!(eff.date_from, None);
        assert_eq!(eff.date_until, None);
        assert_eq!(eff.query, user.query);
    }

    #[test]
    fn keeps_filter_dates_and_ignores_window() {
        let user = bc_ipc::Filter::new("date:>=2026-03-10", None, None);

        let eff = effective_filter(&user, &monthly(Date::constant(2026, 6, 1)));

        /* A date term in the query → the window is NOT injected on either side. */
        assert_eq!(eff.date_from, None);
        assert_eq!(eff.date_until, None);
        assert_eq!(eff.query, user.query);
    }

    #[test]
    fn a_nested_date_term_suppresses_the_window() {
        let user = bc_ipc::Filter::new("coffee or date:2026", None, None);
        let window = DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(2026, 6, 1),
        };
        let eff = effective_filter(&user, &window);
        assert_eq!((eff.date_from, eff.date_until), (None, None));
        assert_eq!(stats_window(&user, &window), (None, None));
        assert_eq!(
            membership_filter(&user).map(|f| f.query),
            Some("coffee or date:2026".to_owned())
        );
    }

    #[test]
    fn a_nested_date_term_makes_the_sparkline_all_time() {
        let user = bc_ipc::Filter::new("coffee or date:2025", None, None);
        let window = DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(2026, 6, 1),
        };
        let first = Date::constant(2024, 2, 10);
        let today = Date::constant(2026, 6, 15);
        let (span_start, span_end) = sparkline_span(&user, &window, Some(first), today);
        assert_eq!((span_start, span_end), (first, Date::constant(2026, 6, 16)));
        let (bucket, count, end) = sparkline_bucketing(&user, &window, Some(first), today);
        let as_of = end.saturating_sub(Span::new().days(1_i64));
        assert!(oldest_bucket_start(&bucket, count, as_of) <= first);
    }

    #[test]
    fn top_level_date_terms_set_the_stats_window() {
        let user = bc_ipc::Filter::new("date:>=2026-03-10 coles", None, None);
        let window = DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(2026, 6, 1),
        };
        assert_eq!(
            stats_window(&user, &window),
            (Some(Date::constant(2026, 3, 10)), None)
        );
        assert_eq!(
            membership_filter(&user).map(|f| f.query),
            Some("coles".to_owned())
        );
        assert_eq!(
            membership_filter(&bc_ipc::Filter::new("date:2026", None, None)),
            None
        );
    }

    #[test]
    fn non_date_dim_detection() {
        let empty = bc_ipc::Filter::default();
        assert!(!filter_has_non_date_dim(&empty));

        let date_only = bc_ipc::Filter::new("date:>=2026-01-01", None, None);
        assert!(!filter_has_non_date_dim(&date_only));

        let tagged = bc_ipc::Filter::new("tag:recurring", None, None);
        assert!(filter_has_non_date_dim(&tagged));

        let texted = bc_ipc::Filter::new("coles", None, None);
        assert!(filter_has_non_date_dim(&texted));

        let unbalanced = bc_ipc::Filter::new("status:unbalanced", None, None);
        assert!(filter_has_non_date_dim(&unbalanced));
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
        let user = bc_ipc::Filter::new("date:>=2025-03-01 date:<2025-04-15", None, None);
        let (start, end) =
            super::sparkline_span(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));
        assert_eq!((start, end), (date(2025, 3, 1), date(2025, 4, 15)));
    }

    #[test]
    fn sparkline_span_after_only_ends_at_nav_end() {
        let user = bc_ipc::Filter::new("date:>=2025-02-10", None, None);
        let (start, end) =
            super::sparkline_span(&user, &monthly(date(2025, 6, 1)), None, date(2025, 6, 1));
        assert_eq!(start, date(2025, 2, 10));
        assert_eq!(end, period_end(&bc_ipc::Period::Monthly, date(2025, 6, 1)));
    }

    #[test]
    fn sparkline_span_before_only_uses_nav_length_ending_at_until() {
        let user = bc_ipc::Filter::new("date:<2025-06-01", None, None);
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
        let user = bc_ipc::Filter::new("date:>=2025-01-01 date:<2025-02-11", None, None);

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
        let user = bc_ipc::Filter::new("date:>=2025-01-15 date:<2025-08-20", None, None);

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
        /* `date:>=2025-06-01 date:<2025-01-01` matches nothing, so the sparkline
         * must render explicitly empty rather than collapsing to a single bar. */
        let user = bc_ipc::Filter::new("date:>=2025-06-01 date:<2025-01-01", None, None);

        let (bucket, count, span_end) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(count, 0);
        assert_eq!(bucket, Period::Daily);
        assert_eq!(span_end, date(2025, 1, 1));
    }

    #[test]
    fn equal_filter_bounds_yield_empty_bucketing() {
        /* A half-open span of zero length is equally empty. */
        let user = bc_ipc::Filter::new("date:>=2025-03-01 date:<2025-03-01", None, None);

        let (_, count, _) =
            sparkline_bucketing(&user, &monthly(date(2025, 1, 1)), None, date(2025, 1, 1));

        assert_eq!(count, 0);
    }
}
