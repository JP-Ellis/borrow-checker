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

use crate::period_defaults::PeriodDefaults;
#[cfg(target_arch = "wasm32")]
use crate::period_defaults::use_period_defaults;
#[cfg(target_arch = "wasm32")]
use crate::url_state::History;

#[cfg(target_arch = "wasm32")]
import_style!(style, "period_nav.module.scss");

// MARK: Private helpers

/// Returns the start of the period that contains `date`, per
/// [`Period::period_start_on_or_before`].
fn period_start(period: &Period, date: Date) -> Date {
    period.period_start_on_or_before(date)
}

/// Returns the exclusive end of the period that begins at `start` (i.e. the
/// first day of the next period), per [`Period::advance`].
pub fn period_end(period: &Period, start: Date) -> Date {
    period.advance(start)
}

/// Returns the last day of the financial year that starts on `fy_start`.
fn fy_last_day(fy_start: Date, start_month: u8, start_day: u8) -> Date {
    let fy = Period::FinancialYear {
        start_month,
        start_day,
    };
    fy.advance(fy_start).saturating_sub(Span::new().days(1_i64))
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
        Period::Fortnightly { .. } => {
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
        Period::FinancialYear {
            start_month,
            start_day,
        } => {
            let last = fy_last_day(start, *start_month, *start_day);
            let start_year = start.year();
            if start_year == last.year() {
                format!("FY {start_year}")
            } else {
                let end_year = last.year() % 100_i16;
                format!("FY {start_year}–{end_year:02}")
            }
        }
        Period::FinancialQuarter {
            start_month,
            start_day,
        } => {
            let fy = Period::FinancialYear {
                start_month: *start_month,
                start_day: *start_day,
            }
            .period_start_on_or_before(start);
            let q = months_between(fy, start) / 3 + 1;
            let last = fy_last_day(fy, *start_month, *start_day);
            let fy_start_year = fy.year();
            if fy_start_year == last.year() {
                format!("FQ{q} {fy_start_year}")
            } else {
                let fy_end_year = last.year() % 100_i16;
                format!("FQ{q} {fy_start_year}/{fy_end_year:02}")
            }
        }
        // Custom and future variants: fall back to ISO date of window start.
        Period::Custom { .. } | &_ => format!("{start}"),
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

// MARK: DisplayWindow

/// The register's display window: every transaction, or one calendar period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayWindow {
    /// No date bound; the whole ledger.
    AllTime,
    /// The period of `period` granularity that begins at `start`.
    Period {
        /// Granularity.
        period: Period,
        /// First day of the window.
        start: Date,
    },
}

impl DisplayWindow {
    /// Half-open `[from, until)` date bounds; both `None` for all time.
    #[must_use]
    pub fn bounds(&self) -> (Option<Date>, Option<Date>) {
        match self {
            Self::AllTime => (None, None),
            Self::Period { period, start } => (Some(*start), Some(period_end(period, *start))),
        }
    }

    /// `true` when `date` falls inside the window (always, for all time).
    #[must_use]
    pub fn contains(&self, date: Date) -> bool {
        match self {
            Self::AllTime => true,
            Self::Period { period, start } => !is_outside_window(period, *start, date),
        }
    }

    /// The calendar year the window lies in, or `None` when it spans two years
    /// or is all time. A date inside that year can omit its year.
    #[must_use]
    pub fn year(&self) -> Option<i16> {
        match self {
            Self::AllTime => None,
            Self::Period { period, start } => {
                let last = period_end(period, *start).saturating_sub(Span::new().days(1_i64));
                (start.year() == last.year()).then_some(start.year())
            }
        }
    }

    /// Human-readable label: `"all time"` or the period's [`window_label`].
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::AllTime => "all time".to_owned(),
            Self::Period { period, start } => window_label(period, *start),
        }
    }

    /// The granularity, or `None` for all time.
    #[must_use]
    pub fn period(&self) -> Option<&Period> {
        match self {
            Self::AllTime => None,
            Self::Period { period, .. } => Some(period),
        }
    }

    /// The granularity to re-anchor on when `date` is not on screen: `None`
    /// for all time (everything is on screen) or when the period window
    /// already contains `date`.
    #[must_use]
    pub fn period_excluding(&self, date: Date) -> Option<&Period> {
        if self.contains(date) {
            return None;
        }
        self.period()
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

/// Parses a granularity from its `<select>` and URL value; `None` when unknown.
///
/// # Arguments
///
/// * `val` - The value string.
/// * `defaults` - Configured financial-year start and fortnightly anchor.
#[must_use]
pub fn period_from_str(val: &str, defaults: PeriodDefaults) -> Option<Period> {
    match val {
        "weekly" => Some(Period::Weekly),
        "fortnightly" => defaults
            .fortnightly_anchor
            .map(|anchor| Period::Fortnightly { anchor }),
        "monthly" => Some(Period::Monthly),
        "quarterly" => Some(Period::Quarterly),
        "financial_quarter" => Some(Period::FinancialQuarter {
            start_month: defaults.fy_start_month,
            start_day: defaults.fy_start_day,
        }),
        "financial_year" => Some(Period::FinancialYear {
            start_month: defaults.fy_start_month,
            start_day: defaults.fy_start_day,
        }),
        "calendar_year" => Some(Period::CalendarYear),
        _ => None,
    }
}

/// Parses a period granularity from a `<select>` value attribute.
#[cfg(target_arch = "wasm32")]
fn parse_period(val: &str, defaults: PeriodDefaults) -> Period {
    period_from_str(val, defaults).unwrap_or(Period::Monthly)
}

/// Converts a [`Period`] to its `<select>` and URL value string.
///
/// # Arguments
///
/// * `p` - The granularity.
#[must_use]
pub fn period_to_str(p: &Period) -> &'static str {
    match p {
        Period::Weekly => "weekly",
        Period::Fortnightly { .. } => "fortnightly",
        Period::Quarterly => "quarterly",
        Period::FinancialQuarter { .. } => "financial_quarter",
        Period::FinancialYear { .. } => "financial_year",
        Period::CalendarYear => "calendar_year",
        Period::Monthly | Period::Daily | _ => "monthly",
    }
}

/// `(value, text)` pairs of every granularity, in display order.
const PERIOD_OPTIONS: [(&str, &str); 7] = [
    ("weekly", "Weekly"),
    ("fortnightly", "Fortnightly"),
    ("monthly", "Monthly"),
    ("quarterly", "Quarterly"),
    ("financial_quarter", "Financial Quarter"),
    ("financial_year", "Financial Year"),
    ("calendar_year", "Calendar Year"),
];

/// `<select>` value of the all-time entry offered by [`WindowNav`].
#[cfg(target_arch = "wasm32")]
const ALL_TIME_VALUE: &str = "all_time";

/// `(value, text)` pairs of the granularity `<select>`, in display order.
///
/// Fortnightly is offered only when an anchor is configured, since
/// [`period_from_str`] cannot build it otherwise.
///
/// # Arguments
///
/// * `defaults` - Configured financial-year start and fortnightly anchor.
///
/// # Returns
///
/// The rows the `<select>` renders.
#[must_use]
pub fn period_options(defaults: PeriodDefaults) -> Vec<(&'static str, &'static str)> {
    PERIOD_OPTIONS
        .into_iter()
        .filter(|(value, _)| *value != "fortnightly" || defaults.fortnightly_anchor.is_some())
        .collect()
}

/// Stateless `◀ label ▶` plus a granularity `<select>`. Owns no signals; the
/// wrapper decides what each control does.
///
/// # Arguments
///
/// * `label` - Window label text.
/// * `show_steps` - Render the `◀`/`▶` buttons.
/// * `selected` - Value of the active `<option>`.
/// * `options` - `(value, text)` pairs for the `<select>`.
/// * `on_prev` / `on_next` - Step callbacks.
/// * `on_latest` - Jumps to the latest period; `None` renders no button.
/// * `latest_disabled` - Disables the jump button alone.
/// * `on_select` - Receives the chosen `<option>` value.
/// * `compact` - Trims chrome for tight contexts.
/// * `disabled` - Dims the control and blocks interaction.
#[cfg(target_arch = "wasm32")]
#[component]
fn NavChrome(
    /// Window label text.
    label: Signal<String>,
    /// Render the `◀`/`▶` buttons.
    show_steps: Signal<bool>,
    /// Value of the active `<option>`.
    selected: Signal<&'static str>,
    /// `(value, text)` pairs for the `<select>`.
    options: Vec<(&'static str, &'static str)>,
    /// Called when the `◀` button is clicked.
    on_prev: Callback<()>,
    /// Called when the `▶` button is clicked.
    on_next: Callback<()>,
    /// Jumps to the latest period; `None` renders no button.
    #[prop(optional_no_strip)]
    on_latest: Option<Callback<()>>,
    /// Disables the jump button alone.
    #[prop(optional, into)]
    latest_disabled: Signal<bool>,
    /// Receives the chosen `<option>` value.
    on_select: Callback<String>,
    /// Trims chrome for tight contexts.
    compact: bool,
    /// Dims the control and blocks interaction.
    disabled: Signal<bool>,
) -> impl IntoView {
    let base_row_class = if compact {
        format!("{} {}", style::nav_row, style::compact)
    } else {
        style::nav_row.to_owned()
    };
    let row_class = move || {
        if disabled.get() {
            format!("{base_row_class} {}", style::disabled)
        } else {
            base_row_class.clone()
        }
    };
    // Without steps the window is all time, which the select already names.
    let window_label = move || {
        show_steps
            .get()
            .then(|| view! { <span class=style::nav_label>{label.get()}</span> })
    };
    view! {
        <div class=row_class>
            {move || {
                show_steps
                    .get()
                    .then(|| {
                        view! {
                            <button
                                class=style::nav_btn
                                aria-label="previous period"
                                disabled=move || disabled.get()
                                on:click=move |_| on_prev.run(())
                            >
                                "\u{25C0}"
                            </button>
                        }
                    })
            }} {window_label}
            {move || {
                show_steps
                    .get()
                    .then(|| {
                        view! {
                            <button
                                class=style::nav_btn
                                aria-label="next period"
                                disabled=move || disabled.get()
                                on:click=move |_| on_next.run(())
                            >
                                "\u{25B6}"
                            </button>
                        }
                    })
            }}
            {move || {
                (show_steps.get() && on_latest.is_some())
                    .then(|| {
                        view! {
                            <button
                                class=style::nav_btn
                                aria-label="latest period"
                                title="jump to the latest matching transaction"
                                disabled=move || disabled.get() || latest_disabled.get()
                                on:click=move |_| {
                                    if let Some(jump) = on_latest {
                                        jump.run(());
                                    }
                                }
                            >
                                "\u{21E5}"
                            </button>
                        }
                    })
            }}
            <select
                class=style::period_select
                aria-label="period"
                prop:value=move || selected.get()
                disabled=move || disabled.get()
                on:change=move |ev| on_select.run(event_target_value(&ev))
            >
                {options
                    .into_iter()
                    .map(|(value, text)| view! { <option value=value>{text}</option> })
                    .collect::<Vec<_>>()}
            </select>
        </div>
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
/// * `disabled` - When `true`, dims the control and blocks interaction.
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
    /// When `true`, dims the control and blocks interaction (e.g. the global
    /// filter's date bounds have taken over the range).
    #[prop(optional, into)]
    disabled: Signal<bool>,
) -> impl IntoView {
    let defaults = use_period_defaults();
    view! {
        <NavChrome
            label=Signal::derive(move || window_label(&period.get(), window_start.get()))
            show_steps=Signal::derive(|| true)
            selected=Signal::derive(move || period_to_str(&period.get()))
            options=period_options(defaults)
            on_prev=Callback::new(move |()| {
                window_start.update(|ws| *ws = step_window(&period.get(), *ws, false));
            })
            on_next=Callback::new(move |()| {
                window_start.update(|ws| *ws = step_window(&period.get(), *ws, true));
            })
            on_select=Callback::new(move |value: String| {
                let new_period = parse_period(&value, defaults);
                window_start.update(|ws| *ws = window_containing(&new_period, *ws));
                period.set(new_period);
            })
            compact=compact
            disabled=disabled
        />
    }
}

/// Window stepper for the accounts page: [`NavChrome`] plus an `All time`
/// entry and an optional jump to the latest period. In all time the step and
/// jump buttons are not rendered; choosing a granularity lands on the period
/// containing today. Steps report [`History::Replace`]; a granularity choice
/// reports [`History::Push`].
///
/// # Arguments
///
/// * `window` - The display window shown.
/// * `on_change` - Receives the next window and how it enters history.
/// * `on_latest` - Jumps to the latest period; `None` hides the button.
/// * `latest_busy` - Disables the jump while it is fetching.
/// * `compact` - Trims chrome for tight contexts.
/// * `disabled` - Dims the control and blocks interaction.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn WindowNav(
    /// The display window shown.
    #[prop(into)]
    window: Signal<DisplayWindow>,
    /// Receives the next window and how it enters history.
    on_change: Callback<(DisplayWindow, History)>,
    /// Jumps to the latest period; `None` hides the button.
    #[prop(optional_no_strip)]
    on_latest: Option<Callback<()>>,
    /// Disables the jump while it is fetching.
    #[prop(optional, into)]
    latest_busy: Signal<bool>,
    /// Trims chrome for tight contexts.
    #[prop(optional)]
    compact: bool,
    /// Dims the control and blocks interaction.
    #[prop(optional, into)]
    disabled: Signal<bool>,
) -> impl IntoView {
    let defaults = use_period_defaults();
    let mut options = vec![(ALL_TIME_VALUE, "All time")];
    options.extend(period_options(defaults));
    let step = move |forward: bool| {
        if let DisplayWindow::Period { period, start } = window.get_untracked() {
            let start = step_window(&period, start, forward);
            on_change.run((DisplayWindow::Period { period, start }, History::Replace));
        }
    };
    view! {
        <NavChrome
            label=Signal::derive(move || window.with(DisplayWindow::label))
            show_steps=Signal::derive(move || window.with(|w| w.period().is_some()))
            selected=Signal::derive(move || {
                window.with(|w| w.period().map_or(ALL_TIME_VALUE, period_to_str))
            })
            options=options
            on_prev=Callback::new(move |()| step(false))
            on_next=Callback::new(move |()| step(true))
            on_select=Callback::new(move |value: String| {
                if value == ALL_TIME_VALUE {
                    on_change.run((DisplayWindow::AllTime, History::Push));
                    return;
                }
                let period = parse_period(&value, defaults);
                let anchor = window
                    .with_untracked(|w| match w {
                        DisplayWindow::Period { start, .. } => *start,
                        DisplayWindow::AllTime => jiff::ZonedDateTime::now().date(),
                    });
                on_change
                    .run((
                        DisplayWindow::Period {
                            start: window_containing(&period, anchor),
                            period,
                        },
                        History::Push,
                    ));
            })
            on_latest=on_latest
            latest_disabled=latest_busy
            compact=compact
            disabled=disabled
        />
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;

// MARK: Tests

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    // MARK: period_options

    #[test]
    fn period_options_omit_fortnightly_without_an_anchor() {
        let options = period_options(PeriodDefaults::FALLBACK);
        assert_eq!(options.len(), 6);
        assert!(options.iter().all(|(value, _)| *value != "fortnightly"));
    }

    #[test]
    fn period_options_offer_fortnightly_with_an_anchor() {
        let defaults = PeriodDefaults {
            fortnightly_anchor: Some(Date::constant(2026, 3, 2)),
            ..PeriodDefaults::FALLBACK
        };
        let options = period_options(defaults);
        assert_eq!(options.len(), 7);
        assert_eq!(options.get(1), Some(&("fortnightly", "Fortnightly")));
    }

    // MARK: period_from_str

    #[test]
    fn fortnightly_uses_the_configured_anchor() {
        let anchor = Date::constant(2026, 3, 2);
        let defaults = PeriodDefaults {
            fortnightly_anchor: Some(anchor),
            ..PeriodDefaults::FALLBACK
        };
        assert_eq!(
            period_from_str("fortnightly", defaults),
            Some(Period::Fortnightly { anchor })
        );
    }

    #[test]
    fn fortnightly_without_an_anchor_is_unknown() {
        assert_eq!(
            period_from_str("fortnightly", PeriodDefaults::FALLBACK),
            None
        );
    }

    // MARK: DisplayWindow::year

    #[rstest]
    #[case::all_time(DisplayWindow::AllTime, None)]
    #[case::month(
        DisplayWindow::Period { period: Period::Monthly, start: Date::constant(2025, 12, 1) },
        Some(2025)
    )]
    #[case::calendar_year(
        DisplayWindow::Period { period: Period::CalendarYear, start: Date::constant(2025, 1, 1) },
        Some(2025)
    )]
    #[case::week_across_new_year(
        DisplayWindow::Period { period: Period::Weekly, start: Date::constant(2025, 12, 29) },
        None
    )]
    #[case::financial_year(
        DisplayWindow::Period {
            period: Period::FinancialYear { start_month: 7, start_day: 1 },
            start: Date::constant(2025, 7, 1),
        },
        None
    )]
    fn display_window_year_is_set_only_within_one_calendar_year(
        #[case] window: DisplayWindow,
        #[case] expected: Option<i16>,
    ) {
        assert_eq!(window.year(), expected);
    }

    // MARK: period_start

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
    // The anchor itself starts a fortnight.
    #[case(Date::constant(2026, 3, 2), Date::constant(2026, 3, 2))]
    // +7 days from anchor — still the same fortnight.
    #[case(Date::constant(2026, 3, 9), Date::constant(2026, 3, 2))]
    // +14 days — start of the next fortnight.
    #[case(Date::constant(2026, 3, 16), Date::constant(2026, 3, 16))]
    // +13 days — last day of the first fortnight.
    #[case(Date::constant(2026, 3, 15), Date::constant(2026, 3, 2))]
    // Before the anchor — phase carries backwards.
    #[case(Date::constant(2026, 3, 1), Date::constant(2026, 2, 16))]
    fn period_start_fortnightly(#[case] input: Date, #[case] expected: Date) {
        assert_eq!(
            period_start(
                &Period::Fortnightly {
                    anchor: Date::constant(2026, 3, 2)
                },
                input
            ),
            expected
        );
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
    #[case(Date::constant(2026, 1, 1), "FY 2026")]
    #[case(Date::constant(2025, 1, 1), "FY 2025")]
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
    /* Jan start (calendar FY): 2026-01-01 → FY 2026 (never touches 2027) */
    #[case(1, Date::constant(2026, 1, 1), "FY 2026")]
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
    // 2026-06-08 to 2026-06-21 — same month (7 fortnights after the anchor).
    #[case(Date::constant(2026, 6, 8), "8–21 Jun 2026")]
    // 2026-06-22 to 2026-07-05 — crosses a month boundary.
    #[case(Date::constant(2026, 6, 22), "22 Jun – 5 Jul 2026")]
    fn window_label_fortnightly(#[case] start: Date, #[case] expected: &str) {
        assert_eq!(
            window_label(
                &Period::Fortnightly {
                    anchor: Date::constant(2026, 3, 2)
                },
                start
            ),
            expected
        );
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
    // 2026-06-08 is 7 fortnights after the anchor 2026-03-02.
    #[case(Date::constant(2026, 6, 8), true, Date::constant(2026, 6, 22))]
    #[case(Date::constant(2026, 6, 8), false, Date::constant(2026, 5, 25))]
    fn step_window_fortnightly(#[case] start: Date, #[case] forward: bool, #[case] expected: Date) {
        assert_eq!(
            step_window(
                &Period::Fortnightly {
                    anchor: Date::constant(2026, 3, 2)
                },
                start,
                forward
            ),
            expected
        );
    }

    #[test]
    fn fortnight_window_follows_the_period_anchor() {
        let p = Period::Fortnightly {
            anchor: Date::constant(2026, 3, 2),
        };
        assert_eq!(
            window_containing(&p, Date::constant(2026, 3, 20)),
            Date::constant(2026, 3, 16)
        );
    }

    #[rstest]
    #[case::jan_fy(
        1,
        1,
        Date::constant(2026, 5, 10),
        Date::constant(2026, 1, 1),
        "FY 2026"
    )]
    #[case::jul_fy(
        7,
        1,
        Date::constant(2026, 5, 10),
        Date::constant(2025, 7, 1),
        "FY 2025–26"
    )]
    fn fy_window_follows_the_period_start(
        #[case] start_month: u8,
        #[case] start_day: u8,
        #[case] on: Date,
        #[case] start: Date,
        #[case] label: &str,
    ) {
        let p = Period::FinancialYear {
            start_month,
            start_day,
        };
        assert_eq!(window_containing(&p, on), start);
        assert_eq!(window_label(&p, start), label);
    }

    #[test]
    fn fq_label_with_january_fy_start() {
        let p = Period::FinancialQuarter {
            start_month: 1,
            start_day: 1,
        };
        let start = window_containing(&p, Date::constant(2026, 5, 10));
        assert_eq!(start, Date::constant(2026, 4, 1));
        insta::assert_snapshot!(window_label(&p, start), @"FQ2 2026");
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

    // MARK: DisplayWindow

    #[test]
    fn all_time_has_no_bounds_and_contains_everything() {
        let w = DisplayWindow::AllTime;
        assert_eq!(w.bounds(), (None, None));
        assert!(w.contains(Date::constant(1990, 1, 1)));
        assert_eq!(w.label(), "all time");
        assert_eq!(w.period(), None);
    }

    #[test]
    fn period_window_bounds_are_half_open() {
        let w = DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(2026, 6, 1),
        };
        assert_eq!(
            w.bounds(),
            (
                Some(Date::constant(2026, 6, 1)),
                Some(Date::constant(2026, 7, 1))
            )
        );
        assert!(w.contains(Date::constant(2026, 6, 30)));
        assert!(!w.contains(Date::constant(2026, 7, 1)));
        assert_eq!(w.label(), "June 2026");
        assert_eq!(w.period(), Some(&Period::Monthly));
    }

    #[test]
    fn all_time_never_excludes_a_date() {
        let w = DisplayWindow::AllTime;
        assert_eq!(w.period_excluding(Date::constant(1990, 1, 1)), None);
        assert_eq!(w.period_excluding(Date::constant(2099, 12, 31)), None);
    }

    #[test]
    fn period_window_excludes_dates_outside_its_half_open_range() {
        let w = DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(2026, 6, 1),
        };
        assert_eq!(w.period_excluding(Date::constant(2026, 6, 1)), None);
        assert_eq!(w.period_excluding(Date::constant(2026, 6, 30)), None);
        assert_eq!(
            w.period_excluding(Date::constant(2026, 5, 31)),
            Some(&Period::Monthly)
        );
        assert_eq!(
            w.period_excluding(Date::constant(2026, 7, 1)),
            Some(&Period::Monthly)
        );
    }
}
