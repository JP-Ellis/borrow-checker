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
/// * `visible` - Whether the dashboard has scrolled past (shows the bar).
#[component]
pub fn StickyAccountBar(
    /// Currently selected account.
    node: Signal<Option<AccountNode>>,
    /// Whether the bar should be shown.
    visible: ReadSignal<bool>,
) -> impl IntoView {
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
                        let currency = bc_ipc::currency_from_code(&n.balance.currency_code)
                            .unwrap_or(&bc_ipc::USD);
                        let balance = crate::components::num::format_amount(
                            n.balance.minor_units,
                            currency,
                        );
                        view! {
                            <span class=style::name>{n.name}</span>
                            <span class=style::sep>" / "</span>
                            <span class=style::balance>{balance}</span>
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
