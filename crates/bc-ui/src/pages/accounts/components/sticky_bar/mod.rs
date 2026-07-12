//! Compact sticky header — pins at the top of the main column after the
//! account dashboard scrolls out of view.

use bc_ipc::AccountNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "sticky_bar.module.scss");

/// Compact one-line account summary that sticks to the top of the main
/// column once the full dashboard has scrolled past.
///
/// Uses `max-height` to collapse to zero when not yet visible, so it
/// occupies no space at the top of the page.
///
/// # Arguments
///
/// * `node` - The currently selected account, or `None` if no account is selected.
/// * `stats` - Resolved account statistics; the sticky balance mirrors the dashboard
///   headline (filtered closing + muted real).
/// * `visible` - Whether the dashboard has scrolled past (shows the bar).
#[component]
pub fn StickyAccountBar(
    /// Currently selected account.
    node: Signal<Option<AccountNode>>,
    /// Resolved account statistics; the sticky balance mirrors the dashboard
    /// headline (filtered closing + muted real).
    stats: Signal<Option<bc_ipc::AccountStats>>,
    /// Whether the bar should be shown.
    visible: ReadSignal<bool>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let balance_view = move || {
        let cur = currencies.get();
        let fmt = |a: &bc_ipc::Amount| {
            let meta = crate::components::num::meta::display_meta_for(&a.currency_code, &cur);
            crate::components::num::format_amount(&a.value, &meta)
        };
        let (closing, real) = match stats.get() {
            Some(s) => (fmt(&s.closing_balance), s.real_closing.as_ref().map(&fmt)),
            None => ("\u{2014}".to_owned(), None),
        };
        view! {
            <span class=style::balance data-testid="sticky-balance">
                {closing}
            </span>
            {real
                .map(|r| {
                    view! {
                        <span class=style::balance_real data-testid="sticky-real-balance">
                            "real "
                            {r}
                        </span>
                    }
                })}
        }
    };
    view! {
        <div class=move || {
            if visible.get() {
                format!("{} {}", style::bar, style::bar_visible)
            } else {
                style::bar.to_owned()
            }
        }>
            {move || {
                node.get()
                    .map(|n| {
                        view! {
                            <span class=style::name>{n.name}</span>
                            <span class=style::sep>" / "</span>
                            {balance_view}
                            <span class=style::meta>" // imported recently"</span>
                            <span class=style::spacer />
                            <div class=style::actions>
                                <button class=style::action_btn>
                                    "reconcile " <kbd class=style::kbd>"r"</kbd>
                                </button>
                                <button class=style::action_btn>
                                    "import " <kbd class=style::kbd>"i"</kbd>
                                </button>
                                <button class=format!(
                                    "{} {}",
                                    style::action_btn,
                                    style::action_primary,
                                )>"+ tx " <kbd class=style::kbd>"↵"</kbd></button>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
