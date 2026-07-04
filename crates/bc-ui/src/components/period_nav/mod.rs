//! Shared period navigation: window labels, stepping, and the period stepper component.
//!
//! Provides helpers for computing human-readable window labels and for
//! stepping the display window forward or backward by one period.
#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

use bc_ipc::Period;
use jiff::Span;
use jiff::civil::Date;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "period_nav.module.scss");

// MARK: Private helpers

/// Returns the start of the period that contains `date`.
///
/// For each variant the rule is:
/// - `Daily` → `date` itself
/// - `Weekly` → Monday of the ISO week containing `date`
/// - `Fortnightly` → Monday of the fortnight containing `date`, anchored to
///   2000-01-03 (a Monday)
/// - `Monthly` → 1st of the month
/// - `Quarterly` → 1st of the calendar quarter (Jan/Apr/Jul/Oct)
/// - `CalendarYear` → 1 January
/// - `FinancialYear` → most recent FY start on or before `date`
/// - `FinancialQuarter` → most recent FQ start on or before `date`
#[expect(
    clippy::arithmetic_side_effects,
    reason = "day offset arithmetic on bounded weekday values (0-6); overflow is not possible"
)]
fn period_start(period: &Period, date: Date) -> Date {
    match period {
        Period::Weekly => {
            // weekday().to_monday_one_offset() gives 1=Mon … 7=Sun
            let offset = i64::from(date.weekday().to_monday_one_offset()) - 1_i64;
            date.saturating_sub(Span::new().days(offset))
        }
        Period::Fortnightly => {
            // Anchor: 2000-01-03 (a known Monday)
            let anchor = Date::constant(2000, 1, 3);
            let diff = i64::from((date - anchor).get_days());
            let phase = diff.rem_euclid(14_i64);
            date.saturating_sub(Span::new().days(phase))
        }
        Period::Monthly => month_start(date),
        Period::Quarterly => quarter_start(date),
        Period::CalendarYear => year_start(date),
        Period::FinancialYear {
            start_month,
            start_day,
        } => fy_start(date, *start_month, *start_day),
        Period::FinancialQuarter {
            start_month,
            start_day,
        } => fq_start(date, *start_month, *start_day),
        // Daily and future variants: `date` is its own period start.
        Period::Daily | &_ => date,
    }
}

/// Returns the exclusive end of the period that begins at `start` (i.e. the
/// first day of the next period).
#[expect(
    clippy::expect_used,
    reason = "Date::new arguments are computed from valid calendar offsets; overflow is not possible"
)]
pub fn period_end(period: &Period, start: Date) -> Date {
    match period {
        Period::Weekly => start.saturating_add(Span::new().days(7_i64)),
        Period::Fortnightly => start.saturating_add(Span::new().days(14_i64)),
        Period::Monthly => {
            // Advance one month and land on the 1st.
            let next = start.saturating_add(Span::new().months(1_i64));
            Date::new(next.year(), next.month(), 1).expect("first day of month is always valid")
        }
        Period::Quarterly => {
            let next = start.saturating_add(Span::new().months(3_i64));
            Date::new(next.year(), next.month(), 1).expect("first day of quarter is always valid")
        }
        Period::CalendarYear => {
            let next_year = start.saturating_add(Span::new().years(1_i64));
            year_start(next_year)
        }
        Period::FinancialYear {
            start_month,
            start_day,
        } => {
            // Advance one year — re-snap to the FY boundary.
            let next_year = start.saturating_add(Span::new().years(1_i64));
            fy_start(next_year, *start_month, *start_day)
        }
        Period::FinancialQuarter {
            start_month,
            start_day,
        } => {
            // Advance three months — re-snap to the FQ boundary.
            let next = start.saturating_add(Span::new().months(3_i64));
            fq_start(next, *start_month, *start_day)
        }
        // Daily and future variants: advance by one day.
        Period::Daily | &_ => start.saturating_add(Span::new().days(1_i64)),
    }
}

/// Returns `Date` of the first day of the month containing `date`.
#[expect(
    clippy::expect_used,
    reason = "day=1 is always valid for any year/month from a real jiff::civil::Date"
)]
fn month_start(date: Date) -> Date {
    Date::new(date.year(), date.month(), 1).expect("day 1 of current month is always valid")
}

/// Returns `Date` of the first day of the calendar quarter containing `date`.
#[expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "quarter start month derived from month (1–12); day=1 is always valid"
)]
fn quarter_start(date: Date) -> Date {
    let q_start_month = (date.month() - 1_i8) / 3_i8 * 3_i8 + 1_i8;
    Date::new(date.year(), q_start_month, 1).expect("quarter start date is always valid")
}

/// Returns `Date` of January 1 of the year containing `date`.
#[expect(
    clippy::expect_used,
    reason = "January 1 is always valid for any year from a real jiff::civil::Date"
)]
fn year_start(date: Date) -> Date {
    Date::new(date.year(), 1, 1).expect("January 1 is always valid")
}

/// Returns the most recent financial-year start date on or before `date`.
#[expect(
    clippy::expect_used,
    reason = "start_month/start_day are from bc_ipc::Period (values 1-28); Date::new is always valid here"
)]
fn fy_start(date: Date, start_month: u8, start_day: u8) -> Date {
    let prev_year = date.saturating_sub(Span::new().years(1_i64)).year();
    let sm = i8::try_from(start_month).expect("start_month is 1-12, always fits i8");
    let sd = i8::try_from(start_day).expect("start_day is 1-28, always fits i8");
    let candidate =
        Date::new(date.year(), sm, sd).expect("FY start in current year is always valid");
    if candidate <= date {
        candidate
    } else {
        Date::new(prev_year, sm, sd).expect("FY start in previous year is always valid")
    }
}

/// Returns the most recent financial-quarter start date on or before `date`.
///
/// FQ boundaries occur at 0, 3, 6, and 9 months after the financial-year start.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "months_into_fy is in 0..11; dividing by 3 and multiplying produces 0/3/6/9; overflow impossible"
)]
fn fq_start(date: Date, start_month: u8, start_day: u8) -> Date {
    let fy = fy_start(date, start_month, start_day);
    let months_into_fy = months_between(fy, date);
    // Round down to the nearest quarter boundary (0, 3, 6, 9).
    let q_months = months_into_fy / 3_i32 * 3_i32;
    fy.saturating_add(Span::new().months(i64::from(q_months)))
}

/// Returns the number of whole months elapsed from `from` to `to` (non-negative).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "year/month differences on valid calendar dates; max range is bounded by real-world FY offsets"
)]
fn months_between(from: Date, to: Date) -> i32 {
    let year_diff = i32::from(to.year()) - i32::from(from.year());
    let month_diff = i32::from(to.month()) - i32::from(from.month());
    // If the day has not yet reached the anchor day, we haven't completed the month.
    let day_adj = if to.day() < from.day() { -1_i32 } else { 0_i32 };
    (year_diff * 12_i32 + month_diff + day_adj).max(0_i32)
    // NOTE: year_diff * 12 cannot realistically overflow i32 for any valid calendar dates
    // since we only compare dates within financial year boundaries.
}

// MARK: Public API

/// Returns the start date of the period window that contains `date`.
///
/// This is the inverse anchor of [`window_label`]: given any date, it returns
/// the first day of the enclosing window for `period`.
///
/// # Arguments
///
/// * `period` - The period granularity.
/// * `date` - Any date within the desired window.
///
/// # Returns
///
/// The first day of the window containing `date`.
///
/// # Example
///
/// ```ignore
/// use bc_ipc::Period;
/// use jiff::civil::Date;
/// let d = Date::constant(2026, 6, 15);
/// assert_eq!(window_containing(&Period::Monthly, d), Date::constant(2026, 6, 1));
/// ```
#[must_use]
#[inline]
pub fn window_containing(period: &Period, date: jiff::civil::Date) -> jiff::civil::Date {
    period_start(period, date)
}

/// Returns `true` when `date` falls outside the window that starts at
/// `window_start` for the given `period`.
///
/// A date is inside the window when its enclosing window start (per
/// [`window_containing`]) equals `window_start`; otherwise it is outside.
///
/// # Arguments
///
/// * `period` - The period granularity defining window boundaries.
/// * `window_start` - The first day of the currently-displayed window.
/// * `date` - The date to test.
///
/// # Returns
///
/// `true` if `date` is not within the window beginning at `window_start`.
#[must_use]
#[inline]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by out-of-period toast notifications")
)]
pub fn is_outside_window(
    period: &Period,
    window_start: jiff::civil::Date,
    date: jiff::civil::Date,
) -> bool {
    window_containing(period, date) != window_start
}

/// Returns a human-readable label for the period window that begins at `start`.
///
/// # Arguments
///
/// * `period` - The period granularity.
/// * `start` - The first day of the display window.
///
/// # Returns
///
/// A formatted label string, for example:
/// - `"June 2026"` for monthly
/// - `"w24 2026 (9–15 Jun)"` for weekly (ISO week, Mon–Sun range, month shown once at end)
/// - `"Q2 2026"` for quarterly
/// - `"2026"` for calendar year
/// - `"FY 2025–26"` for financial year
/// - `"FQ1 2026/27"` for financial quarter
/// - `"1–14 Jun 2026"` or `"29 Jun – 12 Jul 2026"` for fortnightly
///
/// # Example
///
/// ```ignore
/// use bc_ipc::Period;
/// use jiff::civil::Date;
/// let d = Date::constant(2026, 6, 1);
/// assert_eq!(window_label(&Period::Monthly, d), "June 2026");
/// ```
#[must_use]
#[inline]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::modulo_arithmetic,
    reason = "year rem 100 for two-digit display is safe for CE years; quarter arithmetic on months 1-12"
)]
pub fn window_label(period: &Period, start: jiff::civil::Date) -> String {
    match period {
        Period::Daily => {
            format!(
                "{} {} {}",
                start.day(),
                month_abbr(start.month()),
                start.year()
            )
        }
        Period::Weekly => {
            let end_inclusive = start.saturating_add(Span::new().days(6_i64));
            let iso_week = start.iso_week_date().week();
            let year = start.iso_week_date().year();
            // Show month once at the end; omit start month only when both days share it.
            let start_label =
                if start.month() == end_inclusive.month() && start.year() == end_inclusive.year() {
                    format!("{}", start.day())
                } else {
                    format!("{} {}", start.day(), month_abbr(start.month()))
                };
            let end_label = format!(
                "{} {}",
                end_inclusive.day(),
                month_abbr(end_inclusive.month())
            );
            format!("w{iso_week:02} {year} ({start_label}–{end_label})")
        }
        Period::Fortnightly => {
            let end_inclusive = start.saturating_add(Span::new().days(13_i64));
            fortnightly_label(start, end_inclusive)
        }
        Period::Monthly => {
            format!("{} {}", month_name(start.month()), start.year())
        }
        Period::Quarterly => {
            let q = (start.month() - 1_i8) / 3_i8 + 1_i8;
            format!("Q{q} {}", start.year())
        }
        Period::CalendarYear => {
            format!("{}", start.year())
        }
        Period::FinancialYear { .. } => {
            let end = period_end(period, start);
            let start_year = start.year();
            let end_year = end.year() % 100_i16;
            format!("FY {start_year}–{end_year:02}")
        }
        Period::FinancialQuarter {
            start_month,
            start_day,
        } => {
            let fy = fy_start(start, *start_month, *start_day);
            let q = months_between(fy, start) / 3 + 1;
            // FY end start = next FY start.
            let fy_next = fy_start(
                fy.saturating_add(Span::new().years(1_i64)),
                *start_month,
                *start_day,
            );
            let fy_start_year = fy.year();
            let fy_end_year = fy_next.year() % 100_i16;
            if fy_start_year == fy_next.year() {
                format!("FQ{q} {fy_start_year}")
            } else {
                format!("FQ{q} {fy_start_year}/{fy_end_year:02}")
            }
        }
        // Future variants: fall back to ISO date of window start.
        &_ => format!("{start}"),
    }
}

/// Steps the display window one period in the requested direction, returning
/// the new window start date.
///
/// # Arguments
///
/// * `period` - The period granularity.
/// * `current_start` - The first day of the current display window.
/// * `forward` - `true` to advance by one period; `false` to go back.
///
/// # Returns
///
/// The start date of the adjacent period.
///
/// # Example
///
/// ```ignore
/// use bc_ipc::Period;
/// use jiff::civil::Date;
/// let june = Date::constant(2026, 6, 1);
/// assert_eq!(step_window(&Period::Monthly, june, true), Date::constant(2026, 7, 1));
/// assert_eq!(step_window(&Period::Monthly, june, false), Date::constant(2026, 5, 1));
/// ```
#[must_use]
#[inline]
pub fn step_window(
    period: &Period,
    current_start: jiff::civil::Date,
    forward: bool,
) -> jiff::civil::Date {
    if forward {
        period_end(period, current_start)
    } else {
        let day_before = current_start.saturating_sub(Span::new().days(1_i64));
        period_start(period, day_before)
    }
}

// MARK: Formatting helpers

/// Returns the 3-letter month abbreviation (title case) for a 1-based month number.
fn month_abbr(month: i8) -> &'static str {
    match month {
        1_i8 => "Jan",
        2_i8 => "Feb",
        3_i8 => "Mar",
        4_i8 => "Apr",
        5_i8 => "May",
        6_i8 => "Jun",
        7_i8 => "Jul",
        8_i8 => "Aug",
        9_i8 => "Sep",
        10_i8 => "Oct",
        11_i8 => "Nov",
        12_i8 => "Dec",
        _ => "???",
    }
}

/// Returns the full English month name for a 1-based month number.
fn month_name(month: i8) -> &'static str {
    match month {
        1_i8 => "January",
        2_i8 => "February",
        3_i8 => "March",
        4_i8 => "April",
        5_i8 => "May",
        6_i8 => "June",
        7_i8 => "July",
        8_i8 => "August",
        9_i8 => "September",
        10_i8 => "October",
        11_i8 => "November",
        12_i8 => "December",
        _ => "???",
    }
}

/// Builds a fortnightly label of the form `"D–D Mon YYYY"` (same-month) or
/// `"D Mon – D Mon YYYY"` (month boundary within same year) or
/// `"D Mon YYYY – D Mon YYYY"` (year boundary).
fn fortnightly_label(start: Date, end_inclusive: Date) -> String {
    if start.month() == end_inclusive.month() && start.year() == end_inclusive.year() {
        format!(
            "{}–{} {} {}",
            start.day(),
            end_inclusive.day(),
            month_abbr(start.month()),
            start.year()
        )
    } else if start.year() == end_inclusive.year() {
        format!(
            "{} {} – {} {} {}",
            start.day(),
            month_abbr(start.month()),
            end_inclusive.day(),
            month_abbr(end_inclusive.month()),
            start.year()
        )
    } else {
        format!(
            "{} {} {} – {} {} {}",
            start.day(),
            month_abbr(start.month()),
            start.year(),
            end_inclusive.day(),
            month_abbr(end_inclusive.month()),
            end_inclusive.year()
        )
    }
}

// MARK: Component

/// Parses a period granularity from a `<select>` value attribute.
#[cfg(target_arch = "wasm32")]
fn parse_period(val: &str) -> Period {
    match val {
        "weekly" => Period::Weekly,
        "fortnightly" => Period::Fortnightly,
        "quarterly" => Period::Quarterly,
        "financial_quarter" => Period::FinancialQuarter {
            start_month: 7,
            start_day: 1,
        },
        "financial_year" => Period::FinancialYear {
            start_month: 7,
            start_day: 1,
        },
        "calendar_year" => Period::CalendarYear,
        _ => Period::Monthly,
    }
}

/// Converts a [`Period`] back to its `<select>` option value string.
#[cfg(target_arch = "wasm32")]
fn period_to_str(p: &Period) -> &'static str {
    match p {
        Period::Weekly => "weekly",
        Period::Fortnightly => "fortnightly",
        Period::Quarterly => "quarterly",
        Period::FinancialQuarter { .. } => "financial_quarter",
        Period::FinancialYear { .. } => "financial_year",
        Period::CalendarYear => "calendar_year",
        Period::Monthly | Period::Daily | _ => "monthly",
    }
}

/// Shared period stepper: `◀ label ▶` plus a granularity `<select>`.
///
/// The control writes both signals: `◀`/`▶` step `window_start`, and changing
/// the granularity re-snaps `window_start` to the window containing the current
/// start. It renders no page-specific chrome.
///
/// # Arguments
///
/// * `period` - Selected granularity. Owned by the page; this control writes it.
/// * `window_start` - Start of the display window. Owned by the page; written here.
/// * `compact` - When `true`, trims the label width for tight contexts.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn PeriodNav(
    /// Selected granularity (page-owned; written by this control).
    period: RwSignal<Period>,
    /// Display-window start (page-owned; written by this control).
    window_start: RwSignal<Date>,
    /// Trims chrome for tight contexts (e.g. a compact sticky bar).
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let label = move || window_label(&period.get(), window_start.get());
    let row_class = if compact {
        format!("{} {}", style::nav_row, style::compact)
    } else {
        style::nav_row.to_owned()
    };

    view! {
        <div class=row_class>
            <button
                class=style::nav_btn
                aria-label="previous period"
                on:click=move |_| {
                    window_start.update(|ws| *ws = step_window(&period.get(), *ws, false));
                }
            >
                "\u{25C0}"
            </button>
            <span class=style::nav_label>{label}</span>
            <button
                class=style::nav_btn
                aria-label="next period"
                on:click=move |_| {
                    window_start.update(|ws| *ws = step_window(&period.get(), *ws, true));
                }
            >
                "\u{25B6}"
            </button>
            <select
                class=style::period_select
                prop:value=move || period_to_str(&period.get())
                on:change=move |ev| {
                    let new_period = parse_period(&event_target_value(&ev));
                    window_start.update(|ws| *ws = window_containing(&new_period, *ws));
                    period.set(new_period);
                }
            >
                <option value="weekly">"Weekly"</option>
                <option value="fortnightly">"Fortnightly"</option>
                <option value="monthly">"Monthly"</option>
                <option value="quarterly">"Quarterly"</option>
                <option value="financial_quarter">"Financial Quarter"</option>
                <option value="financial_year">"Financial Year"</option>
                <option value="calendar_year">"Calendar Year"</option>
            </select>
        </div>
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;

// MARK: Tests

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    // MARK: period_start

    #[rstest]
    #[case(Date::constant(2026, 6, 15), Date::constant(2026, 6, 15))]
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 6, 1))]
    fn period_start_daily(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::Daily, input), expected);
    }

    #[rstest]
    // 2026-06-15 is a Monday — already the start of its week.
    #[case(Date::constant(2026, 6, 15), Date::constant(2026, 6, 15))]
    // Wednesday — go back to Monday.
    #[case(Date::constant(2026, 6, 17), Date::constant(2026, 6, 15))]
    // Sunday — go back to Monday.
    #[case(Date::constant(2026, 6, 21), Date::constant(2026, 6, 15))]
    fn period_start_weekly(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::Weekly, input), expected);
    }

    #[rstest]
    // Anchor 2000-01-03 is a known Monday.
    #[case(Date::constant(2000, 1, 3), Date::constant(2000, 1, 3))]
    // +7 days from anchor — still same fortnight.
    #[case(Date::constant(2000, 1, 10), Date::constant(2000, 1, 3))]
    // +14 days — start of next fortnight.
    #[case(Date::constant(2000, 1, 17), Date::constant(2000, 1, 17))]
    // +13 days — last day of first fortnight.
    #[case(Date::constant(2000, 1, 16), Date::constant(2000, 1, 3))]
    fn period_start_fortnightly(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::Fortnightly, input), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 6, 15), Date::constant(2026, 6, 1))]
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 6, 1))]
    #[case(Date::constant(2026, 6, 30), Date::constant(2026, 6, 1))]
    fn period_start_monthly(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::Monthly, input), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 1, 1), Date::constant(2026, 1, 1))]
    #[case(Date::constant(2026, 3, 31), Date::constant(2026, 1, 1))]
    #[case(Date::constant(2026, 4, 1), Date::constant(2026, 4, 1))]
    #[case(Date::constant(2026, 6, 15), Date::constant(2026, 4, 1))]
    #[case(Date::constant(2026, 7, 1), Date::constant(2026, 7, 1))]
    #[case(Date::constant(2026, 10, 1), Date::constant(2026, 10, 1))]
    #[case(Date::constant(2026, 12, 31), Date::constant(2026, 10, 1))]
    fn period_start_quarterly(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::Quarterly, input), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 1, 1), Date::constant(2026, 1, 1))]
    #[case(Date::constant(2026, 6, 15), Date::constant(2026, 1, 1))]
    #[case(Date::constant(2026, 12, 31), Date::constant(2026, 1, 1))]
    fn period_start_calendar_year(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(period_start(&Period::CalendarYear, input), expected);
    }

    #[rstest]
    // Australian FY: starts 1 July.
    #[case(Date::constant(2026, 6, 30), Date::constant(2025, 7, 1))]
    #[case(Date::constant(2026, 7, 1), Date::constant(2026, 7, 1))]
    #[case(Date::constant(2027, 6, 30), Date::constant(2026, 7, 1))]
    fn period_start_financial_year(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(
            period_start(
                &Period::FinancialYear {
                    start_month: 7,
                    start_day: 1,
                },
                input
            ),
            expected
        );
    }

    #[rstest]
    // Australian FY: FQ1 = Jul–Sep, FQ2 = Oct–Dec, FQ3 = Jan–Mar, FQ4 = Apr–Jun.
    #[case(Date::constant(2026, 7, 1), Date::constant(2026, 7, 1))] // start of FQ1
    #[case(Date::constant(2026, 9, 30), Date::constant(2026, 7, 1))] // end of FQ1
    #[case(Date::constant(2026, 10, 1), Date::constant(2026, 10, 1))] // start of FQ2
    #[case(Date::constant(2026, 12, 31), Date::constant(2026, 10, 1))]
    #[case(Date::constant(2027, 1, 1), Date::constant(2027, 1, 1))] // FQ3
    #[case(Date::constant(2027, 4, 1), Date::constant(2027, 4, 1))] // FQ4
    fn period_start_financial_quarter(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(
            period_start(
                &Period::FinancialQuarter {
                    start_month: 7,
                    start_day: 1,
                },
                input
            ),
            expected
        );
    }

    // MARK: window_label

    #[rstest]
    #[case(Period::Monthly, Date::constant(2026, 6, 1), "June 2026")]
    #[case(Period::Monthly, Date::constant(2026, 1, 1), "January 2026")]
    #[case(Period::Monthly, Date::constant(2026, 12, 1), "December 2026")]
    fn window_label_monthly(#[case] period: Period, #[case] start: Date, #[case] expected: &str) {
        assert_eq!(window_label(&period, start), expected);
    }

    #[rstest]
    // w24 2026: 2026-06-08 Mon to 2026-06-14 Sun (same month — month shown at end).
    #[case(Date::constant(2026, 6, 8), "w24 2026 (8–14 Jun)")]
    // w26 2026: 2026-06-22 Mon to 2026-06-28 Sun (same month).
    #[case(Date::constant(2026, 6, 22), "w26 2026 (22–28 Jun)")]
    // w27 2026: 2026-06-29 Mon to 2026-07-05 Sun (crosses month boundary).
    #[case(Date::constant(2026, 6, 29), "w27 2026 (29 Jun–5 Jul)")]
    fn window_label_weekly(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(window_label(&Period::Weekly, start), expected);
    }

    #[rstest]
    #[case(Period::Quarterly, Date::constant(2026, 1, 1), "Q1 2026")]
    #[case(Period::Quarterly, Date::constant(2026, 4, 1), "Q2 2026")]
    #[case(Period::Quarterly, Date::constant(2026, 7, 1), "Q3 2026")]
    #[case(Period::Quarterly, Date::constant(2026, 10, 1), "Q4 2026")]
    fn window_label_quarterly(#[case] period: Period, #[case] start: Date, #[case] expected: &str) {
        assert_eq!(window_label(&period, start), expected);
    }

    #[test]
    fn window_label_calendar_year() {
        assert_eq!(
            window_label(&Period::CalendarYear, Date::constant(2026, 1, 1)),
            "2026"
        );
    }

    #[rstest]
    // Australian FY 2025-26: starts 2025-07-01, ends 2026-06-30.
    #[case(Date::constant(2025, 7, 1), "FY 2025–26")]
    #[case(Date::constant(2026, 7, 1), "FY 2026–27")]
    fn window_label_financial_year(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(
            window_label(
                &Period::FinancialYear {
                    start_month: 7,
                    start_day: 1,
                },
                start
            ),
            expected
        );
    }

    #[rstest]
    /* January FY (e.g. US calendar FY): start_month=1. */
    #[case(Date::constant(2026, 1, 1), "FY 2026–27")]
    #[case(Date::constant(2025, 1, 1), "FY 2025–26")]
    fn window_label_financial_year_january(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(
            window_label(
                &Period::FinancialYear {
                    start_month: 1,
                    start_day: 1,
                },
                start,
            ),
            expected
        );
    }

    #[rstest]
    /* Labels use "FY YYYY–YY" format regardless of FY start month — no month range shown. */
    /* Jul start (Australian): 2025-07-01 → FY 2025–26 */
    #[case(7, Date::constant(2025, 7, 1), "FY 2025–26")]
    /* Jan start (calendar FY): 2026-01-01 → FY 2026–27 */
    #[case(1, Date::constant(2026, 1, 1), "FY 2026–27")]
    /* Apr start: 2026-04-01 → FY 2026–27 */
    #[case(4, Date::constant(2026, 4, 1), "FY 2026–27")]
    fn window_label_financial_year_variants(
        #[case] start_month: u8,
        #[case] start: Date,
        #[case] expected: &str,
    ) {
        assert_eq!(
            window_label(
                &Period::FinancialYear {
                    start_month,
                    start_day: 1,
                },
                start,
            ),
            expected
        );
    }

    #[rstest]
    // Aus FY starting Jul 1: FQ1=Jul–Sep, FQ2=Oct–Dec, FQ3=Jan–Mar, FQ4=Apr–Jun.
    #[case(Date::constant(2026, 7, 1), "FQ1 2026/27")]
    #[case(Date::constant(2026, 10, 1), "FQ2 2026/27")]
    #[case(Date::constant(2027, 1, 1), "FQ3 2026/27")]
    #[case(Date::constant(2027, 4, 1), "FQ4 2026/27")]
    fn window_label_financial_quarter(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(
            window_label(
                &Period::FinancialQuarter {
                    start_month: 7,
                    start_day: 1,
                },
                start
            ),
            expected
        );
    }

    #[rstest]
    // 2026-06-01 to 2026-06-14 — same month (anchor-aligned fortnight).
    #[case(Date::constant(2026, 6, 1), "1–14 Jun 2026")]
    // 2026-06-29 to 2026-07-12 — crosses month boundary (anchor-aligned fortnight).
    #[case(Date::constant(2026, 6, 29), "29 Jun – 12 Jul 2026")]
    fn window_label_fortnightly(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(window_label(&Period::Fortnightly, start), expected);
    }

    // MARK: step_window

    #[rstest]
    #[case(Date::constant(2026, 6, 1), true, Date::constant(2026, 7, 1))]
    #[case(Date::constant(2026, 6, 1), false, Date::constant(2026, 5, 1))]
    #[case(Date::constant(2026, 1, 1), false, Date::constant(2025, 12, 1))]
    #[case(Date::constant(2026, 12, 1), true, Date::constant(2027, 1, 1))]
    fn step_window_monthly(#[case] start: Date, #[case] forward: bool, #[case] expected: Date) {
        assert_eq!(step_window(&Period::Monthly, start, forward), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 6, 15), true, Date::constant(2026, 6, 22))]
    #[case(Date::constant(2026, 6, 15), false, Date::constant(2026, 6, 8))]
    fn step_window_weekly(#[case] start: Date, #[case] forward: bool, #[case] expected: Date) {
        assert_eq!(step_window(&Period::Weekly, start, forward), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 1, 1), true, Date::constant(2026, 4, 1))]
    #[case(Date::constant(2026, 4, 1), false, Date::constant(2026, 1, 1))]
    fn step_window_quarterly(#[case] start: Date, #[case] forward: bool, #[case] expected: Date) {
        assert_eq!(step_window(&Period::Quarterly, start, forward), expected);
    }

    #[rstest]
    #[case(Date::constant(2026, 1, 1), true, Date::constant(2027, 1, 1))]
    #[case(Date::constant(2026, 1, 1), false, Date::constant(2025, 1, 1))]
    fn step_window_calendar_year(
        #[case] start: Date,
        #[case] forward: bool,
        #[case] expected: Date,
    ) {
        assert_eq!(step_window(&Period::CalendarYear, start, forward), expected);
    }

    #[rstest]
    // Australian FY.
    #[case(Date::constant(2025, 7, 1), true, Date::constant(2026, 7, 1))]
    #[case(Date::constant(2026, 7, 1), false, Date::constant(2025, 7, 1))]
    fn step_window_financial_year(
        #[case] start: Date,
        #[case] forward: bool,
        #[case] expected: Date,
    ) {
        assert_eq!(
            step_window(
                &Period::FinancialYear {
                    start_month: 7,
                    start_day: 1,
                },
                start,
                forward
            ),
            expected
        );
    }

    #[rstest]
    /* January FY (e.g. US calendar FY): FY2026 = Jan 2026–Dec 2026. */
    #[case(Date::constant(2026, 1, 1), true, Date::constant(2027, 1, 1))]
    #[case(Date::constant(2027, 1, 1), false, Date::constant(2026, 1, 1))]
    fn step_window_financial_year_january(
        #[case] start: Date,
        #[case] forward: bool,
        #[case] expected: Date,
    ) {
        assert_eq!(
            step_window(
                &Period::FinancialYear {
                    start_month: 1,
                    start_day: 1,
                },
                start,
                forward
            ),
            expected
        );
    }

    #[rstest]
    /* January FQ: Q1 Jan, Q2 Apr, Q3 Jul, Q4 Oct. */
    #[case(Date::constant(2026, 1, 1), true, Date::constant(2026, 4, 1))]
    #[case(Date::constant(2026, 4, 1), true, Date::constant(2026, 7, 1))]
    #[case(Date::constant(2026, 7, 1), true, Date::constant(2026, 10, 1))]
    #[case(Date::constant(2026, 10, 1), true, Date::constant(2027, 1, 1))]
    /* Cross-FY-boundary backward: FQ1 Jan 2026 → FQ4 Oct 2025. */
    #[case(Date::constant(2026, 1, 1), false, Date::constant(2025, 10, 1))]
    fn step_window_financial_quarter_january(
        #[case] start: Date,
        #[case] forward: bool,
        #[case] expected: Date,
    ) {
        assert_eq!(
            step_window(
                &Period::FinancialQuarter {
                    start_month: 1,
                    start_day: 1,
                },
                start,
                forward
            ),
            expected
        );
    }

    #[rstest]
    #[case(Date::constant(2026, 7, 1), true, Date::constant(2026, 10, 1))]
    #[case(Date::constant(2026, 10, 1), true, Date::constant(2027, 1, 1))]
    #[case(Date::constant(2027, 1, 1), false, Date::constant(2026, 10, 1))]
    /* Cross-FY-boundary backward: FQ1 Jul 2026 → FQ4 Apr 2026 (same year). */
    #[case(Date::constant(2026, 7, 1), false, Date::constant(2026, 4, 1))]
    fn step_window_financial_quarter(
        #[case] start: Date,
        #[case] forward: bool,
        #[case] expected: Date,
    ) {
        assert_eq!(
            step_window(
                &Period::FinancialQuarter {
                    start_month: 7,
                    start_day: 1,
                },
                start,
                forward
            ),
            expected
        );
    }

    #[rstest]
    // 2026-06-01 is an anchor-aligned fortnight Monday.
    #[case(Date::constant(2026, 6, 1), true, Date::constant(2026, 6, 15))]
    #[case(Date::constant(2026, 6, 1), false, Date::constant(2026, 5, 18))]
    fn step_window_fortnightly(#[case] start: Date, #[case] forward: bool, #[case] expected: Date) {
        assert_eq!(step_window(&Period::Fortnightly, start, forward), expected);
    }

    // MARK: window_containing

    #[rstest]
    #[case(
        Period::Monthly,
        Date::constant(2026, 6, 15),
        Date::constant(2026, 6, 1)
    )]
    #[case(
        Period::Quarterly,
        Date::constant(2026, 6, 15),
        Date::constant(2026, 4, 1)
    )]
    #[case(
        Period::CalendarYear,
        Date::constant(2026, 6, 15),
        Date::constant(2026, 1, 1)
    )]
    #[case(
        Period::Weekly,
        Date::constant(2026, 6, 17),
        Date::constant(2026, 6, 15)
    )]
    fn window_containing_snaps_to_window_start(
        #[case] period: Period,
        #[case] input: Date,
        #[case] expected: Date,
    ) {
        assert_eq!(window_containing(&period, input), expected);
    }

    // MARK: is_outside_window

    #[rstest]
    // Monthly window starting 2026-06-01.
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 6, 15), false)] /* inside */
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 6, 1), false)] /* first day inside */
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 6, 30), false)] /* last day inside */
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 5, 31), true)] /* day before */
    #[case(Date::constant(2026, 6, 1), Date::constant(2026, 7, 1), true)] /* day after (exclusive end) */
    fn is_outside_window_monthly(
        #[case] window_start: Date,
        #[case] date: Date,
        #[case] expected: bool,
    ) {
        assert_eq!(
            is_outside_window(&Period::Monthly, window_start, date),
            expected
        );
    }

    #[rstest]
    // Calendar-year window starting 2025-01-01.
    #[case(Date::constant(2025, 1, 1), Date::constant(2025, 12, 31), false)] /* inside */
    #[case(Date::constant(2025, 1, 1), Date::constant(2026, 1, 1), true)] /* next year */
    #[case(Date::constant(2025, 1, 1), Date::constant(2024, 12, 31), true)] /* prev year */
    fn is_outside_window_calendar_year(
        #[case] window_start: Date,
        #[case] date: Date,
        #[case] expected: bool,
    ) {
        assert_eq!(
            is_outside_window(&Period::CalendarYear, window_start, date),
            expected
        );
    }
}
