//! The URL query string as the source of the palette query, the accounts
//! display window and a transaction to open:
//! `?q=<query>&period=<granularity>&start=<YYYY-MM-DD>&tx=<id>`.

use bc_ipc::Period;
use jiff::civil::Date;
use percent_encoding::AsciiSet;
use percent_encoding::NON_ALPHANUMERIC;
use percent_encoding::percent_decode_str;
use percent_encoding::utf8_percent_encode;

use crate::components::period_nav::DisplayWindow;
use crate::components::period_nav::period_from_str;
use crate::components::period_nav::period_to_str;
use crate::components::period_nav::window_containing;
use crate::period_defaults::PeriodDefaults;

// MARK: Codec

/// `localStorage` key holding the last location, replayed on a cold start.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm shell mirrors the location")
)]
pub const LAST_LOCATION_KEY: &str = "bc.last_location";

/// `sessionStorage` key marking a tab that has loaded the app. A reload keeps
/// the mark; a new tab or an app restart starts without it.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm shell marks the tab")
)]
pub const TAB_SEEN_KEY: &str = "bc.tab_seen";

/// Characters a transaction id is escaped on: all but the URL's unreserved set.
const TX_ESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Route prefix of the debug QA pages, never mirrored or replayed.
const QA_PREFIX: &str = "/__test";

/// Whether `path` is a QA route: `/__test` itself or a path under it.
fn is_qa_route(path: &str) -> bool {
    path.strip_prefix(QA_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// How a write lands in the browser history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm store writes history")
)]
pub enum History {
    /// A new entry; back undoes the write.
    Push,
    /// Overwrites the current entry.
    Replace,
}

/// The query, display window and transaction to open that a URL carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlState {
    /// Palette query text; empty for no filter.
    pub query: String,
    /// Accounts-page display window.
    pub window: DisplayWindow,
    /// A transaction the accounts register opens once its rows load, then
    /// drops from the URL.
    pub tx: Option<String>,
    /// Parameters a page reads and this state does not, such as the import
    /// page's `profile`, each `key=value` as the URL spelled it. They follow
    /// this state's own parameters, in their given order.
    pub page_params: Vec<String>,
}

impl Default for UrlState {
    fn default() -> Self {
        Self {
            query: String::new(),
            window: DisplayWindow::AllTime,
            tx: None,
            page_params: Vec::new(),
        }
    }
}

impl UrlState {
    /// Parses a search string, with or without its leading `?`. Unknown
    /// parameters belong to the page and are kept verbatim; an unknown
    /// `period`, an unparsable `start` or only one of the pair reads as all
    /// time; `start` snaps to its period.
    ///
    /// # Arguments
    ///
    /// * `search` - The URL's search component.
    /// * `defaults` - Configured values that complete a period named in the URL.
    ///
    /// # Returns
    ///
    /// The state, and `true` when `search` was already canonical.
    #[must_use]
    pub fn from_search(search: &str, defaults: PeriodDefaults) -> (Self, bool) {
        let raw = search.trim_prefix('?');
        let mut query = None;
        let mut period_name = None;
        let mut start_text = None;
        let mut tx = None;
        let mut page_params = Vec::new();
        for pair in raw.split('&').filter(|p| !p.is_empty()) {
            let (key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode(raw_value);
            match decode(key).as_str() {
                "q" => query = Some(value),
                "period" => period_name = Some(value),
                "start" => start_text = Some(value),
                "tx" => tx = Some(value),
                _ => page_params.push(pair.to_owned()),
            }
        }
        let granularity = period_name
            .as_deref()
            .and_then(|name| period_from_str(name, defaults));
        let start_date = start_text
            .as_deref()
            .and_then(|text| text.parse::<Date>().ok());
        let window = match (granularity, start_date) {
            (Some(period), Some(start)) => DisplayWindow::Period {
                start: window_containing(&period, start),
                period,
            },
            _ => DisplayWindow::AllTime,
        };
        let state = Self {
            query: query.unwrap_or_default(),
            window,
            tx: tx.filter(|id| !id.is_empty()),
            page_params,
        };
        let given = if raw.is_empty() {
            String::new()
        } else {
            format!("?{raw}")
        };
        let canonical = state.to_search() == given;
        (state, canonical)
    }

    /// This state with no transaction to open. Every write but the link
    /// opener's goes through this, so a `tx` is consumed once.
    #[must_use]
    pub fn without_tx(self) -> Self {
        Self { tx: None, ..self }
    }

    /// The canonical search string: `""`, or `?` and the non-empty parameters
    /// in the order `q`, `period`, `start`, `tx`, then the page's own.
    #[must_use]
    pub fn to_search(&self) -> String {
        let mut parts = Vec::new();
        if !self.query.trim().is_empty() {
            parts.push(format!(
                "q={}",
                utf8_percent_encode(&self.query, NON_ALPHANUMERIC)
            ));
        }
        if let DisplayWindow::Period { period, start } = &self.window {
            parts.push(format!("period={}", period_to_str(period)));
            parts.push(format!("start={start}"));
        }
        if let Some(tx) = &self.tx {
            parts.push(format!("tx={}", utf8_percent_encode(tx, TX_ESCAPED)));
        }
        parts.extend(self.page_params.iter().cloned());
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    }
}

/// Decodes one query-string component; `+` reads as a space, as browsers do.
fn decode(component: &str) -> String {
    percent_decode_str(&component.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// The `period` window that holds `date`.
///
/// # Arguments
///
/// * `period` - Granularity to keep.
/// * `date` - A date the window must contain.
#[must_use]
pub fn latest_window(period: &Period, date: Date) -> DisplayWindow {
    DisplayWindow::Period {
        start: window_containing(period, date),
        period: period.clone(),
    }
}

/// The state that opens transaction `tx_id` on another page.
///
/// The query stays, so a filter that hides the row stays too. A period window
/// moves to the one holding `date`. All time becomes the month holding
/// `date`, so the row lands on the register's first page. The previous
/// page's own parameters are dropped.
///
/// # Arguments
///
/// * `current` - The state shown now.
/// * `tx_id` - The transaction to open.
/// * `date` - The transaction's date.
#[must_use]
pub fn transaction_link_state(current: &UrlState, tx_id: &str, date: Date) -> UrlState {
    UrlState {
        query: current.query.clone(),
        window: current.window.period().map_or_else(
            || latest_window(&Period::Monthly, date),
            |period| latest_window(period, date),
        ),
        tx: Some(tx_id.to_owned()),
        page_params: Vec::new(),
    }
}

/// The location string to mirror for a cold start; `None` on a QA route.
///
/// # Arguments
///
/// * `pathname` - The URL path.
/// * `search` - The search component, with or without its `?`.
#[must_use]
pub fn mirror_entry(pathname: &str, search: &str) -> Option<String> {
    if is_qa_route(pathname) {
        return None;
    }
    let bare = search.trim_prefix('?');
    Some(if bare.is_empty() {
        pathname.to_owned()
    } else {
        format!("{pathname}?{bare}")
    })
}

/// The saved location to replay, when a tab's first load lands at bare `/`.
///
/// # Arguments
///
/// * `pathname` - The path the document loaded at.
/// * `search` - The search the document loaded with.
/// * `saved` - The mirrored location, if any.
/// * `tab_seen` - Whether this tab loaded the app before, so this load is a
///   reload.
#[must_use]
pub fn should_replay(
    pathname: &str,
    search: &str,
    saved: Option<&str>,
    tab_seen: bool,
) -> Option<String> {
    let bare = search.trim_prefix('?');
    if tab_seen || pathname != "/" || !bare.is_empty() {
        return None;
    }
    let target = saved?;
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    // A browser treats `\` as `/`, so `/\host` would leave the app.
    let in_app = target.starts_with('/') && !target.starts_with("//") && !target.contains('\\');
    (in_app && target != "/" && !is_qa_route(path)).then(|| target.to_owned())
}

/// The restore toast's text; `None` when nothing worth announcing came back.
///
/// # Arguments
///
/// * `state` - The restored state.
#[must_use]
pub fn restore_message(state: &UrlState) -> Option<String> {
    let mut parts = Vec::new();
    if !state.query.trim().is_empty() {
        parts.push(state.query.clone());
    }
    if state.window != DisplayWindow::AllTime {
        parts.push(state.window.label());
    }
    (!parts.is_empty()).then(|| format!("Restored filter: {}", parts.join(" · ")))
}

/// The search component of a location string, without its `?`.
///
/// # Arguments
///
/// * `location` - A path with an optional query string.
#[must_use]
pub fn search_of(location: &str) -> &str {
    location.split_once('?').map_or("", |(_, search)| search)
}

// MARK: Tests

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Period;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::UrlState;
    use super::latest_window;
    use super::mirror_entry;
    use super::restore_message;
    use super::search_of;
    use super::should_replay;
    use super::transaction_link_state;
    use crate::components::period_nav::DisplayWindow;
    use crate::components::period_nav::period_from_str;
    use crate::components::period_nav::window_label;
    use crate::period_defaults::PeriodDefaults;

    fn monthly(y: i16, m: i8, d: i8) -> DisplayWindow {
        DisplayWindow::Period {
            period: Period::Monthly,
            start: Date::constant(y, m, d),
        }
    }

    fn state(query: &str, window: DisplayWindow) -> UrlState {
        UrlState {
            query: query.to_owned(),
            window,
            tx: None,
            page_params: Vec::new(),
        }
    }

    #[rstest]
    #[case::empty(state("", DisplayWindow::AllTime), "")]
    #[case::query_only(state("tag:work", DisplayWindow::AllTime), "?q=tag%3Awork")]
    #[case::window_only(state("", monthly(2025, 12, 1)), "?period=monthly&start=2025-12-01")]
    #[case::both(
        state("tag:work", monthly(2025, 12, 1)),
        "?q=tag%3Awork&period=monthly&start=2025-12-01"
    )]
    #[case::blank_query_omitted(state("   ", DisplayWindow::AllTime), "")]
    fn to_search_emits_canonical_form(#[case] input: UrlState, #[case] expected: &str) {
        assert_eq!(input.to_search(), expected);
    }

    #[rstest]
    #[case::ampersand("a & b")]
    #[case::hash("#tag")]
    #[case::plus("amount:>=1+2")]
    #[case::percent("100%")]
    #[case::quoted("@payee:\"Coffee & Co\"")]
    #[case::non_ascii("café 東京")]
    #[case::equals("meta:k=v")]
    fn query_round_trips(#[case] query: &str) {
        let original = state(query, DisplayWindow::AllTime);
        let (parsed, canonical) =
            UrlState::from_search(&original.to_search(), PeriodDefaults::FALLBACK);
        assert_eq!(parsed, original);
        assert!(canonical);
    }

    #[rstest]
    #[case::weekly("weekly")]
    #[case::fortnightly("fortnightly")]
    #[case::monthly("monthly")]
    #[case::quarterly("quarterly")]
    #[case::financial_quarter("financial_quarter")]
    #[case::financial_year("financial_year")]
    #[case::calendar_year("calendar_year")]
    fn every_period_round_trips(#[case] name: &str) {
        let defaults = PeriodDefaults {
            fortnightly_anchor: Some(Date::constant(2000, 1, 3)),
            ..PeriodDefaults::FALLBACK
        };
        let period = period_from_str(name, defaults).expect("known period");
        let window = latest_window(&period, Date::constant(2025, 12, 15));
        let original = state("", window);
        let (parsed, canonical) = UrlState::from_search(&original.to_search(), defaults);
        assert_eq!(parsed, original);
        assert!(canonical);
    }

    #[rstest]
    #[case::unknown_period("?period=hourly&start=2025-12-01", state("", DisplayWindow::AllTime))]
    #[case::period_without_start("?period=monthly", state("", DisplayWindow::AllTime))]
    #[case::start_without_period("?start=2025-12-01", state("", DisplayWindow::AllTime))]
    #[case::bad_date("?period=monthly&start=2025-13-01", state("", DisplayWindow::AllTime))]
    #[case::mid_period_start("?period=monthly&start=2025-12-15", state("", monthly(2025, 12, 1)))]
    #[case::unescaped_colon("?q=tag:work", state("tag:work", DisplayWindow::AllTime))]
    #[case::plus_is_space(
        "?q=tag%3Awork+booking",
        state("tag:work booking", DisplayWindow::AllTime)
    )]
    #[case::page_param_first(
        "?profile=mock-bank&q=x",
        UrlState { page_params: vec!["profile=mock-bank".to_owned()], ..state("x", DisplayWindow::AllTime) }
    )]
    #[case::reordered("?start=2025-12-01&period=monthly", state("", monthly(2025, 12, 1)))]
    fn non_canonical_input_parses_and_reports(#[case] search: &str, #[case] expected: UrlState) {
        let (parsed, canonical) = UrlState::from_search(search, PeriodDefaults::FALLBACK);
        assert_eq!(parsed, expected);
        assert!(!canonical);
    }

    #[rstest]
    #[case::alone("?profile=mock-bank")]
    #[case::after_the_filter("?q=tag%3Awork&batch=import_batch_1&discard=1")]
    #[case::encoded("?backup=%2Fbackups%2Fledger.sqlite&section=backup")]
    fn page_params_survive_canonicalisation(#[case] search: &str) {
        let (parsed, canonical) = UrlState::from_search(search, PeriodDefaults::FALLBACK);
        assert!(canonical, "{search} is already canonical");
        assert_eq!(parsed.to_search(), search);
    }

    #[test]
    fn a_filter_change_keeps_the_page_params() {
        let (mut parsed, _) = UrlState::from_search("?profile=mock-bank", PeriodDefaults::FALLBACK);
        parsed.query = "tag:work".to_owned();
        assert_eq!(parsed.to_search(), "?q=tag%3Awork&profile=mock-bank");
    }

    #[rstest]
    #[case::empty("", "")]
    #[case::bare_question_mark("?", "")]
    #[case::no_prefix("q=x", "x")]
    fn prefix_is_optional(#[case] search: &str, #[case] expected: &str) {
        let (parsed, _) = UrlState::from_search(search, PeriodDefaults::FALLBACK);
        assert_eq!(parsed.query, expected);
    }

    #[test]
    fn tx_round_trips() {
        let search = "?period=monthly&start=2026-03-01&tx=abc";
        let (parsed, canonical) = UrlState::from_search(search, PeriodDefaults::FALLBACK);
        assert_eq!(parsed.tx.as_deref(), Some("abc"));
        assert_eq!(parsed.window, monthly(2026, 3, 1));
        assert!(canonical);
        assert_eq!(parsed.to_search(), search);
    }

    #[test]
    fn tx_alone_is_kept() {
        let (parsed, canonical) = UrlState::from_search("?tx=abc", PeriodDefaults::FALLBACK);
        assert_eq!(parsed.tx.as_deref(), Some("abc"));
        assert_eq!(parsed.window, DisplayWindow::AllTime);
        assert!(canonical);
    }

    #[test]
    fn tx_follows_the_window_and_precedes_page_params() {
        let (parsed, canonical) = UrlState::from_search(
            "?tx=abc&profile=mock-bank&start=2026-03-01&period=monthly",
            PeriodDefaults::FALLBACK,
        );
        assert!(!canonical);
        assert_eq!(
            parsed.to_search(),
            "?period=monthly&start=2026-03-01&tx=abc&profile=mock-bank"
        );
    }

    #[rstest]
    #[case::period_moves_to_the_date(monthly(2026, 1, 1), monthly(2026, 3, 1))]
    #[case::all_time_becomes_the_month(DisplayWindow::AllTime, monthly(2026, 3, 1))]
    #[case::quarter_moves_to_the_date(
        latest_window(&Period::Quarterly, Date::constant(2025, 1, 1)),
        latest_window(&Period::Quarterly, Date::constant(2026, 3, 15))
    )]
    fn transaction_link_covers_the_date(
        #[case] window: DisplayWindow,
        #[case] expected: DisplayWindow,
    ) {
        let current = UrlState {
            page_params: vec!["profile=mock-bank".to_owned()],
            ..state("tag:work", window)
        };
        let next = transaction_link_state(&current, "abc", Date::constant(2026, 3, 15));
        assert_eq!(
            next,
            UrlState {
                tx: Some("abc".to_owned()),
                ..state("tag:work", expected)
            }
        );
    }

    #[test]
    fn without_tx_keeps_everything_else() {
        let (parsed, _) = UrlState::from_search(
            "?q=x&period=monthly&start=2026-03-01&tx=abc&profile=mock-bank",
            PeriodDefaults::FALLBACK,
        );
        assert_eq!(
            parsed.without_tx().to_search(),
            "?q=x&period=monthly&start=2026-03-01&profile=mock-bank"
        );
    }

    #[test]
    fn latest_window_snaps_to_period_start() {
        assert_eq!(
            latest_window(&Period::Monthly, Date::constant(2025, 12, 15)),
            monthly(2025, 12, 1)
        );
    }

    #[rstest]
    #[case::root("/", "", Some("/"))]
    #[case::with_search("/accounts/7", "q=x", Some("/accounts/7?q=x"))]
    #[case::with_prefixed_search("/accounts/7", "?q=x", Some("/accounts/7?q=x"))]
    #[case::qa_route("/__test/chips", "q=x", None)]
    #[case::qa_root("/__test", "", None)]
    #[case::qa_subpath("/__test/x", "", None)]
    #[case::qa_lookalike("/__testing", "", Some("/__testing"))]
    fn mirror_entry_cases(
        #[case] path: &str,
        #[case] search: &str,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(mirror_entry(path, search).as_deref(), expected);
    }

    #[rstest]
    #[case::cold_start("/", "", Some("/accounts/7?q=x"), false, Some("/accounts/7?q=x"))]
    #[case::reload_at_root("/", "", Some("/accounts/7?q=x"), true, None)]
    #[case::nothing_saved("/", "", None, false, None)]
    #[case::saved_root("/", "", Some("/"), false, None)]
    #[case::deep_link("/accounts/7", "", Some("/budget"), false, None)]
    #[case::root_with_search("/", "q=x", Some("/budget"), false, None)]
    #[case::protocol_relative("/", "", Some("//example.com/"), false, None)]
    #[case::not_a_path("/", "", Some("https://example.com/"), false, None)]
    #[case::qa_route("/", "", Some("/__test/chips"), false, None)]
    #[case::qa_root("/", "", Some("/__test"), false, None)]
    #[case::qa_root_with_search("/", "", Some("/__test?q=x"), false, None)]
    #[case::qa_subpath("/", "", Some("/__test/x"), false, None)]
    #[case::qa_lookalike("/", "", Some("/__testing"), false, Some("/__testing"))]
    #[case::backslash_host("/", "", Some("/\\evil.example"), false, None)]
    fn should_replay_cases(
        #[case] path: &str,
        #[case] search: &str,
        #[case] saved: Option<&str>,
        #[case] tab_seen: bool,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            should_replay(path, search, saved, tab_seen).as_deref(),
            expected
        );
    }

    #[test]
    fn restore_message_names_present_parts() {
        let dec = Date::constant(2025, 12, 1);
        let label = window_label(&Period::Monthly, dec);
        assert_eq!(restore_message(&state("", DisplayWindow::AllTime)), None);
        assert_eq!(
            restore_message(&state("tag:work", DisplayWindow::AllTime)).as_deref(),
            Some("Restored filter: tag:work")
        );
        assert_eq!(
            restore_message(&state("", monthly(2025, 12, 1))),
            Some(format!("Restored filter: {label}"))
        );
        assert_eq!(
            restore_message(&state("tag:work", monthly(2025, 12, 1))),
            Some(format!("Restored filter: tag:work · {label}"))
        );
    }

    #[test]
    fn fy_url_is_canonical_under_a_january_fy() {
        let jan = PeriodDefaults {
            fy_start_month: 1,
            fy_start_day: 1,
            fortnightly_anchor: None,
        };
        let (state, canonical) =
            UrlState::from_search("?period=financial_year&start=2026-01-01", jan);
        assert!(canonical);
        assert_eq!(state.to_search(), "?period=financial_year&start=2026-01-01");
    }

    #[test]
    fn fortnightly_url_without_anchor_reads_as_all_time() {
        let (state, canonical) = UrlState::from_search(
            "?period=fortnightly&start=2026-03-02",
            PeriodDefaults::FALLBACK,
        );
        assert!(!canonical);
        assert_eq!(state.window, DisplayWindow::AllTime);
        assert_eq!(state.to_search(), "");
    }

    #[test]
    fn fortnightly_url_with_an_anchor_is_canonical() {
        let anchor = Date::constant(2026, 3, 2);
        let defaults = PeriodDefaults {
            fortnightly_anchor: Some(anchor),
            ..PeriodDefaults::FALLBACK
        };
        // 2026-03-30 is two fortnights after the anchor.
        let search = "?period=fortnightly&start=2026-03-30";
        let (state, canonical) = UrlState::from_search(search, defaults);
        assert!(canonical);
        assert_eq!(
            state.window,
            DisplayWindow::Period {
                period: Period::Fortnightly { anchor },
                start: Date::constant(2026, 3, 30),
            }
        );
        assert_eq!(state.to_search(), search);
    }

    #[rstest]
    #[case::with_search("/accounts/7?q=x", "q=x")]
    #[case::without_search("/accounts/7", "")]
    fn search_of_cases(#[case] location: &str, #[case] expected: &str) {
        assert_eq!(search_of(location), expected);
    }
}
