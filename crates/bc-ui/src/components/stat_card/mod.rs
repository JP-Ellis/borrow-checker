//! KPI stat card — eyebrow label, large value, optional sub-line.

use leptos::prelude::*;
use leptos::web_sys;
use stylance::import_style;

import_style!(style, "stat_card.module.scss");

/// Colour tone for the value in a [`StatCard`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatTone {
    /// `--bc-good` (green) — income, positive deltas.
    Good,
    /// `--bc-bad` (red) — expenses, negative deltas.
    Bad,
    /// `--bc-warn` (amber) — attention needed.
    Warn,
    /// `--bc-ink` (default ink) — neutral.
    #[default]
    Neutral,
}

impl StatTone {
    /// Returns the CSS class for this tone.
    #[must_use]
    #[inline]
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Good => style::good,
            Self::Bad => style::bad,
            Self::Warn => style::warn,
            Self::Neutral => style::neutral,
        }
    }
}

/// Responsive container for [`StatCard`] tiles.
///
/// Items reflow as the container narrows. Unlike a CSS `auto-fill` grid,
/// items on the last row **expand to fill the full row width** rather than
/// leaving blank column slots. A widow-avoidance algorithm ensures no single
/// card sits alone on a row when reducing columns would give an even split.
///
/// The `count` prop must match the number of [`StatCard`] children; it is
/// used by the column algorithm and cannot be inferred from opaque children.
#[component]
pub fn StatCards(
    /// Number of [`StatCard`] children (must match actual child count).
    count: usize,
    /// The [`StatCard`] children to display.
    children: Children,
) -> impl IntoView {
    let node_ref = NodeRef::<leptos::html::Div>::new();
    let cols = RwSignal::new(count.max(1));

    // `get()` (not `get_untracked()`) makes this effect subscribe to the node
    // ref and re-run once the element is actually mounted in the DOM.
    Effect::new(move |_| {
        let Some(el) = node_ref.get() else {
            return;
        };
        let w = el.offset_width();
        if w <= 0_i32 {
            return;
        }
        cols.set(optimal_cols(count, max_cols_for_width(w)));
    });

    // `resize` fires on viewport change; `mouseup` catches CSS resize-handle
    // drags (used by the QA isolation container) and preset-button clicks.
    let recompute = move |_: web_sys::Event| {
        let Some(el) = node_ref.get_untracked() else {
            return;
        };
        let w = el.offset_width();
        if w <= 0_i32 {
            return;
        }
        cols.set(optimal_cols(count, max_cols_for_width(w)));
    };
    window_event_listener_untyped("resize", recompute);
    window_event_listener_untyped("mouseup", recompute);

    view! {
        <div
            node_ref=node_ref
            class=style::cards
            style=move || format!("--stat-cols:{}", cols.get())
        >
            {children()}
        </div>
    }
}

/// Computes the maximum number of same-width card columns that fit in `width_px`.
///
/// Uses the minimum card width (160 px) and an approximation of the gap token
/// (12 px for `--bc-space-4`) to determine how many columns can be accommodated.
#[expect(
    clippy::as_conversions,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "width_px comes from offset_width() which is non-negative in practice; \
              integer division is intentional floor-division for column count"
)]
fn max_cols_for_width(width_px: i32) -> usize {
    const MIN_CARD_PX: i32 = 160;
    const GAP_PX: i32 = 12;
    let fits = ((width_px + GAP_PX) / (MIN_CARD_PX + GAP_PX)) as usize;
    fits.max(1)
}

/// Returns the optimal column count for `n` items given a maximum of `max` columns.
///
/// Reduces the column count when a remainder of exactly 1 would leave a single
/// orphaned card on the last row (a "widow"). Stops reducing at 1 column.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division_remainder_used,
    reason = "modulo and subtraction on small bounded usize values; no overflow possible"
)]
fn optimal_cols(n: usize, max: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut c = n.min(max);
    while c > 1 && n % c == 1 {
        c -= 1;
    }
    c
}

/// A small KPI tile: eyebrow label, large value, optional sub-line.
///
/// # Arguments
///
/// * `label` - Short eyebrow label, e.g. `"income (30d)"`.
/// * `value` - Primary display value, e.g. `"+$9,100"`.
/// * `sub` - Optional secondary line, e.g. `"avg · commbank-au"`.
/// * `tone` - Colour tone for the value. Defaults to [`StatTone::Neutral`].
#[component]
pub fn StatCard(
    /// Short eyebrow label.
    label: String,
    /// Primary display value.
    value: String,
    /// Optional secondary line below the value.
    #[prop(optional, into)]
    sub: Option<String>,
    /// Value colour tone.
    #[prop(default = StatTone::Neutral)]
    tone: StatTone,
) -> impl IntoView {
    let value_class = format!("{} {}", style::value, tone.css_class());
    view! {
        <div class=style::card>
            <span class=style::label>{label}</span>
            <span class=value_class>{value}</span>
            {sub.map(|s| view! { <span class=style::sub>{s}</span> })}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
