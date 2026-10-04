//! Account bar: one line pinned at the top of the accounts main column.

use bc_ipc::AccountNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "sticky_bar.module.scss");

/// One-line account summary pinned at the top of the main column: the full
/// account path, the headline balance and the add-transaction action.
///
/// It always occupies `--bc-account-bar-height` while an account is selected,
/// so nothing below it moves on scroll. With no account it renders nothing.
///
/// # Arguments
///
/// * `node` - The currently selected account, or `None` if no account is selected.
/// * `path` - The account's full path, e.g. `Assets :: Bank :: Savings`.
/// * `stats` - Resolved account statistics; the balance mirrors the dashboard
///   headline (filtered closing + muted real).
/// * `on_add_tx` - Opens the add-transaction form.
#[component]
pub fn StickyAccountBar(
    /// Currently selected account.
    node: Signal<Option<AccountNode>>,
    /// The account's full path.
    path: Signal<String>,
    /// Resolved account statistics; the balance mirrors the dashboard
    /// headline (filtered closing + muted real).
    stats: Signal<Option<bc_ipc::AccountStats>>,
    /// Opens the add-transaction form.
    #[prop(optional)]
    on_add_tx: Option<Callback<()>>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let balance_view = move || {
        let cur = currencies.get();
        let fmt = |a: &bc_ipc::Amount| {
            let meta = crate::components::num::meta::display_meta_for(&a.currency_code, &cur);
            crate::components::num::format_amount(&a.value, &meta)
        };
        let (closing, real) = match stats.get() {
            Some(s) => (fmt(&s.closing_balance), s.real_closing.as_ref().map(fmt)),
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
        <Show when=move || node.with(Option::is_some)>
            <div class=style::bar data-testid="account-bar">
                // RTL clips the path from the left, so the leaf stays in view;
                // the inner span keeps the text itself left-to-right.
                <span class=style::path title=move || path.get() data-testid="account-path">
                    <span class=style::path_text>{move || path.get()}</span>
                </span>
                <span class=style::sep>" / "</span>
                {balance_view}
                <span class=style::spacer />
                <div class=style::actions>
                    <button
                        class=format!("{} {}", style::action_btn, style::action_primary)
                        aria-label="+ tx: add transaction"
                        data-testid="add-tx"
                        on:click=move |_| {
                            if let Some(cb) = on_add_tx {
                                cb.run(());
                            }
                        }
                    >
                        "+ tx "
                        <kbd class=style::kbd>"↵"</kbd>
                    </button>
                </div>
            </div>
        </Show>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
