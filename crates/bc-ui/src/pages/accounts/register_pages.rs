//! Paging state for the lazily loaded register, and the balance-column mode.
//! Leptos-free so the merge rules are native-testable.

use std::collections::HashMap;

use bc_ipc::Amount;
use bc_ipc::RegisterCursor;
use bc_ipc::RegisterPage;
use bc_ipc::RegisterRow;
use rust_decimal::Decimal;

/// Rows fetched per request.
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "consumed by the page in Task 18")
)]
pub const PAGE_SIZE: u32 = 100;

/// `localStorage` key holding the balance mode's [`BalanceMode::as_str`] value.
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "consumed by the page in Task 18")
)]
pub const BALANCE_MODE_KEY: &str = "accounts.balance_mode";

/// Everything the register has loaded so far.
///
/// A *reset* (selection, window, filter, roll-up or data change) replaces
/// `rows`, asking for at least as many as were on screen so an edit does not
/// collapse the list. An *extend* appends the next page. Every request carries
/// the `generation` current when it started; a response from an older
/// generation is dropped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadedRegister {
    /// Rows in display order.
    pub rows: Vec<RegisterRow>,
    /// Matching rows across every page.
    pub total: u32,
    /// Resume point for the next page; `None` once everything is loaded.
    pub next_cursor: Option<RegisterCursor>,
    /// A request is in flight.
    pub loading: bool,
    /// Bumped by every reset.
    pub generation: u32,
}

impl LoadedRegister {
    /// Starts a reset. Returns the new generation and the limit to request:
    /// `PAGE_SIZE`, or the loaded row count when that is larger.
    pub fn begin_reset(&mut self) -> (u32, u32) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        let loaded = u32::try_from(self.rows.len()).unwrap_or(u32::MAX);
        (self.generation, PAGE_SIZE.max(loaded))
    }

    /// Starts an extend, or `None` while a request is in flight or the end
    /// has been reached. Returns the current generation and the cursor.
    pub fn begin_extend(&mut self) -> Option<(u32, RegisterCursor)> {
        if self.loading {
            return None;
        }
        let cursor = self.next_cursor.clone()?;
        self.loading = true;
        Some((self.generation, cursor))
    }

    /// Applies a reset response; `false` (and no change) when `generation` is stale.
    pub fn apply_reset(&mut self, generation: u32, page: RegisterPage) -> bool {
        if generation != self.generation {
            return false;
        }
        self.rows = page.rows;
        self.total = page.total;
        self.next_cursor = page.next_cursor;
        self.loading = false;
        true
    }

    /// Applies an extend response; `false` (and no change) when `generation` is stale.
    pub fn apply_extend(&mut self, generation: u32, page: RegisterPage) -> bool {
        if generation != self.generation {
            return false;
        }
        self.rows.extend(page.rows);
        self.total = page.total;
        self.next_cursor = page.next_cursor;
        self.loading = false;
        true
    }

    /// Clears `loading` after a failed request of `generation`.
    pub fn fail(&mut self, generation: u32) {
        if generation == self.generation {
            self.loading = false;
        }
    }

    /// `true` once every matching row is loaded.
    ///
    /// A default, never-reset register also reports `true`: it has no
    /// `next_cursor` because it has never asked for one, not because it
    /// exhausted the register.
    #[must_use]
    pub fn fully_loaded(&self) -> bool {
        self.next_cursor.is_none()
    }
}

/// What the balance column shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BalanceMode {
    /// The scope's real balance after each row.
    #[default]
    Real,
    /// The running sum of the matching rows, oldest match first.
    FilteredSum,
    /// No balance.
    Hidden,
}

impl BalanceMode {
    /// The next mode in the toggle's cycle.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Real => Self::FilteredSum,
            Self::FilteredSum => Self::Hidden,
            Self::Hidden => Self::Real,
        }
    }

    /// Storage value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::FilteredSum => "sum",
            Self::Hidden => "off",
        }
    }

    /// Parses a storage value; anything unknown is [`Self::Real`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "sum" => Self::FilteredSum,
            "off" => Self::Hidden,
            _ => Self::Real,
        }
    }

    /// Toggle label. The sum reads `filtered sum` only when a non-date filter
    /// is active; without one it differs from the real balance only by
    /// starting at zero.
    #[must_use]
    pub fn label(self, filter_active: bool) -> &'static str {
        match (self, filter_active) {
            (Self::Real, _) => "balance: real",
            (Self::FilteredSum, true) => "balance: filtered sum",
            (Self::FilteredSum, false) => "balance: sum",
            (Self::Hidden, _) => "balance: off",
        }
    }
}

/// The row's value for `mode`; `None` in [`BalanceMode::Hidden`] or for a
/// mixed-commodity row.
#[must_use]
pub fn balance_value(row: &RegisterRow, mode: BalanceMode) -> Option<&Amount> {
    match mode {
        BalanceMode::Real => row.balance_after.as_ref(),
        BalanceMode::FilteredSum => row.filtered_sum_after.as_ref(),
        BalanceMode::Hidden => None,
    }
}

/// Per-commodity bar axis over the loaded rows: `(min(0, lowest), max(0, highest))`.
#[must_use]
pub fn axes_for(rows: &[RegisterRow], mode: BalanceMode) -> HashMap<String, (Decimal, Decimal)> {
    let mut axes: HashMap<String, (Decimal, Decimal)> = HashMap::new();
    for amount in rows.iter().filter_map(|r| balance_value(r, mode)) {
        let bounds = axes
            .entry(amount.currency_code.clone())
            .or_insert((Decimal::ZERO, Decimal::ZERO));
        bounds.0 = bounds.0.min(amount.value);
        bounds.1 = bounds.1.max(amount.value);
    }
    axes
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Amount;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;

    fn row(id: &str, real: Option<(i64, &str)>, sum: Option<(i64, &str)>) -> RegisterRow {
        let tx = Transaction::new(
            id,
            Date::constant(2026, 6, 1),
            "",
            vec![],
            Reconciliation::Reconciled,
            vec![],
            vec![],
            vec![],
        );
        let amt = |v: Option<(i64, &str)>| v.map(|(n, c)| Amount::new(Decimal::new(n, 2), c));
        RegisterRow::new(tx, vec![], amt(real), amt(sum))
    }

    fn page(ids: &[&str], total: u32, more: bool) -> RegisterPage {
        let rows = ids
            .iter()
            .map(|id| row(id, Some((100, "AUD")), None))
            .collect();
        let cursor = more.then(|| {
            RegisterCursor::new(
                Date::constant(2026, 6, 1),
                (*ids.last().expect("ids")).to_owned(),
            )
        });
        RegisterPage::new(rows, total, cursor)
    }

    #[test]
    #[expect(
        clippy::shadow_unrelated,
        reason = "second limit is an unrelated later reset; reusing the name keeps the test readable"
    )]
    fn reset_keeps_loaded_count_as_limit() {
        let mut r = LoadedRegister::default();
        let (g0, limit) = r.begin_reset();
        assert_eq!(limit, PAGE_SIZE);
        assert!(r.apply_reset(g0, page(&["a", "b"], 5, true)));
        let (g1, _) = r.begin_extend().expect("more");
        assert!(r.apply_extend(g1, page(&["c", "d"], 5, true)));
        assert_eq!(r.rows.len(), 4);
        // An edit refreshes what is on screen: the limit covers the loaded rows.
        let (_, limit) = r.begin_reset();
        assert_eq!(limit, PAGE_SIZE.max(4));
    }

    #[test]
    #[expect(
        clippy::shadow_unrelated,
        reason = "second limit is an unrelated later reset; reusing the name keeps the test readable"
    )]
    fn reset_limit_grows_past_page_size() {
        let mut r = LoadedRegister::default();
        let ids: Vec<String> = (0_u32..150_u32).map(|i| format!("r{i}")).collect();
        let rows: Vec<RegisterRow> = ids
            .iter()
            .map(|id| row(id, Some((100, "AUD")), None))
            .collect();
        let loaded = rows.len();

        let (g0, limit) = r.begin_reset();
        assert_eq!(limit, PAGE_SIZE, "nothing loaded yet");
        assert!(r.apply_reset(
            g0,
            RegisterPage::new(rows, u32::try_from(loaded).expect("fits u32"), None)
        ));

        // More than a page is on screen: the limit must cover it, past PAGE_SIZE.
        let (_, limit) = r.begin_reset();
        assert_eq!(limit, u32::try_from(loaded).expect("fits u32"));
        assert!(limit > PAGE_SIZE);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn apply_extend_stale_generation_is_dropped() {
        let mut r = LoadedRegister::default();
        let (g0, _) = r.begin_reset();
        assert!(r.apply_reset(g0, page(&["a"], 5, true)));
        let (g_extend, _cursor) = r.begin_extend().expect("more");

        // A reset starts (e.g. the filter changes) before the extend's
        // response arrives, bumping the generation past the extend's.
        let (g1, _) = r.begin_reset();
        assert_ne!(g_extend, g1);

        assert!(!r.apply_extend(g_extend, page(&["b"], 5, true)));

        // The stale extend changed nothing beyond what the second
        // begin_reset already did.
        assert_eq!(r.generation, g1);
        assert!(r.loading);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].transaction.id, "a");
        assert_eq!(r.total, 5);
        assert!(r.next_cursor.is_some());
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn stale_generation_is_dropped() {
        let mut r = LoadedRegister::default();
        let (g0, _) = r.begin_reset();
        let (g1, _) = r.begin_reset();
        assert!(!r.apply_reset(g0, page(&["old"], 1, false)));
        assert!(r.apply_reset(g1, page(&["new"], 1, false)));
        assert_eq!(r.rows[0].transaction.id, "new");
        assert!(r.fully_loaded());
        assert_eq!(r.begin_extend(), None);
    }

    #[test]
    fn extend_is_blocked_while_loading_and_fail_unblocks() {
        let mut r = LoadedRegister::default();
        let (g0, _) = r.begin_reset();
        r.apply_reset(g0, page(&["a"], 3, true));
        let (g1, _) = r.begin_extend().expect("first extend");
        assert_eq!(r.begin_extend(), None, "in flight");
        r.fail(g1);
        assert!(r.begin_extend().is_some());
    }

    #[rstest]
    #[case(BalanceMode::Real, BalanceMode::FilteredSum)]
    #[case(BalanceMode::FilteredSum, BalanceMode::Hidden)]
    #[case(BalanceMode::Hidden, BalanceMode::Real)]
    fn mode_cycles(#[case] from: BalanceMode, #[case] to: BalanceMode) {
        assert_eq!(from.cycle(), to);
        assert_eq!(BalanceMode::parse(to.as_str()), to);
    }

    #[test]
    fn mode_key_is_the_storage_key() {
        assert_eq!(BALANCE_MODE_KEY, "accounts.balance_mode");
    }

    #[test]
    fn mode_label_names_the_filter() {
        assert_eq!(
            BalanceMode::FilteredSum.label(true),
            "balance: filtered sum"
        );
        assert_eq!(BalanceMode::FilteredSum.label(false), "balance: sum");
        assert_eq!(BalanceMode::parse("garbage"), BalanceMode::Real);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn axes_per_commodity_include_zero() {
        let rows = vec![
            row("a", Some((-12_000, "AUD")), Some((500, "AUD"))),
            row("b", Some((8_000, "AUD")), Some((900, "AUD"))),
            row("c", Some((1_000, "XYZ")), None),
            row("d", None, None),
        ];
        let real = axes_for(&rows, BalanceMode::Real);
        assert_eq!(
            real["AUD"],
            (Decimal::new(-12_000, 2), Decimal::new(8_000, 2))
        );
        assert_eq!(real["XYZ"], (Decimal::ZERO, Decimal::new(1_000, 2)));
        let sum = axes_for(&rows, BalanceMode::FilteredSum);
        assert_eq!(sum["AUD"], (Decimal::ZERO, Decimal::new(900, 2)));
        assert!(!sum.contains_key("XYZ"));
        assert!(axes_for(&rows, BalanceMode::Hidden).is_empty());
        assert_eq!(balance_value(&rows[3], BalanceMode::Real), None);
    }
}
