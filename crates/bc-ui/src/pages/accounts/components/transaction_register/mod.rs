//! Transaction register — column headers, keyboard-navigable row list.

use bc_ipc::AccountRef;
use leptos::prelude::*;
use leptos::web_sys;
use stylance::import_style;
use wasm_bindgen::JsCast as _;

use crate::components::balance_cell::BalanceCell;
use crate::components::period_nav::DisplayWindow;
use crate::components::transaction_row::RowPerspective;
use crate::components::transaction_row::TransactionRow;
use crate::pages::accounts::register_pages::BalanceMode;
use crate::pages::accounts::register_pages::LoadedRegister;
use crate::pages::accounts::register_pages::axes_for;

import_style!(style, "register.module.scss");

/// The full transaction register: column headers and row list.
///
/// Handles keyboard navigation (`j`/`k` to move, `Enter` to expand, `Esc` to
/// collapse) via a `keydown` listener on the register container. `j` past the
/// last loaded row asks the page for the next one.
///
/// # Arguments
///
/// * `register` - Everything loaded so far (rows, total, paging state).
/// * `on_load_more` - Asks the page for the next page of rows.
/// * `balance_mode` - What the balance column shows (page-owned, persisted).
/// * `viewing_account_id` - The account whose page is currently shown.
/// * `on_change` - Optional callback invoked after any mutation (e.g. reverse)
///   so the parent can refresh its transaction list.
/// * `accounts` - All selectable accounts for the per-row recategorise picker.
/// * `window` - Page-level display window (shared with the dashboard).
/// * `busy` - `true` while `transactions` still shows a previous window.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos props must take String for #[prop(into)] support"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn TransactionRegister(
    /// Everything loaded so far.
    register: Signal<LoadedRegister>,
    /// Asks the page for the next page of rows.
    on_load_more: Callback<()>,
    /// What the balance column shows (page-owned, persisted).
    balance_mode: RwSignal<BalanceMode>,
    /// Account ID being viewed (determines headline amounts).
    #[prop(into)]
    viewing_account_id: String,
    /// Called after any mutation so the parent can bump its data version.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
    /// All selectable accounts for the per-row recategorise picker.
    #[prop(optional)]
    accounts: Vec<AccountRef>,
    /// Page-level display window (shared with the dashboard).
    window: RwSignal<DisplayWindow>,
    /// `true` while `transactions` still shows a previous window; rendered as
    /// `aria-busy` so assistive tech and the e2e suite can tell stale rows
    /// from settled ones.
    #[prop(optional, into)]
    busy: Signal<bool>,
) -> impl IntoView {
    let accounts = StoredValue::new(accounts);

    let selected_idx = RwSignal::new(Option::<usize>::None);
    let expanded_idx = RwSignal::new(Option::<usize>::None);

    let filter_store = crate::filter_ctx::use_filter_store();
    let period_locked = Signal::derive(move || {
        filter_store
            .filter
            .with(|f| f.date_from.is_some() || f.date_until.is_some())
    });
    let filter_active = Signal::derive(move || {
        filter_store
            .filter
            .with(crate::pages::accounts::query::filter_has_non_date_dim)
    });

    let row_count = Signal::derive(move || register.with(|r| r.rows.len()));
    let axes = Signal::derive(move || register.with(|r| axes_for(&r.rows, balance_mode.get())));

    let on_keydown = move |e: web_sys::KeyboardEvent| {
        if let Some(t) = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let tag = t.tag_name();
            if tag == "INPUT" || tag == "TEXTAREA" || t.is_content_editable() {
                return;
            }
        }

        let row_count = row_count.get_untracked();
        if row_count == 0 {
            return;
        }

        match e.key().as_str() {
            "j" | "ArrowDown" => {
                if selected_idx.get_untracked() == Some(row_count.saturating_sub(1)) {
                    on_load_more.run(());
                }
                selected_idx.update(|s| {
                    *s =
                        Some(s.map_or(0, |i| i.saturating_add(1).min(row_count.saturating_sub(1))));
                });
                e.prevent_default();
            }
            "k" | "ArrowUp" => {
                selected_idx.update(|s| {
                    *s = Some(s.map_or(0, |i| i.saturating_sub(1)));
                });
                e.prevent_default();
            }
            "Enter" => {
                if let Some(idx) = selected_idx.get() {
                    expanded_idx.update(|ex| {
                        *ex = if *ex == Some(idx) { None } else { Some(idx) };
                    });
                }
                e.prevent_default();
            }
            "Escape" => {
                expanded_idx.set(None);
                e.prevent_default();
            }
            _ => {}
        }
    };

    let vid = viewing_account_id.clone();
    let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));

    let toasts = crate::components::toast::use_toasts();
    let on_saved_cb = Callback::new(move |date: jiff::civil::Date| {
        crate::pages::accounts::period_notify::notify_if_out_of_period(toasts, window, date);
    });

    view! {
        <div
            class=style::register
            on:keydown=on_keydown
            tabindex="0"
            aria-label="transaction register"
            aria-busy=move || if busy.get() { "true" } else { "false" }
        >
            <div class=style::header>
                <crate::components::period_nav::WindowNav
                    window=window
                    compact=true
                    disabled=period_locked
                />
                <span class=style::reg_title>
                    "register" <span class=style::bracket>"["</span>
                    <span class=style::count>
                        {move || {
                            register
                                .with(|r| {
                                    if r.fully_loaded() {
                                        r.total.to_string()
                                    } else {
                                        format!("{} / {}", r.rows.len(), r.total)
                                    }
                                })
                        }}
                    </span> <span class=style::bracket>"]"</span>
                </span>
                <button
                    class=style::mode_btn
                    data-testid="balance-mode"
                    on:click=move |_| balance_mode.update(|m| *m = m.cycle())
                >
                    {move || balance_mode.get().label(filter_active.get())}
                </button>
            </div>

            <div class=style::col_headers>
                <span class=style::col_date>"date"</span>
                <span>"payee"</span>
                <span class=style::col_category>"category"</span>
                <span class=style::col_amt>"amount"</span>
                <span class=style::col_balance>"balance"</span>
                <span />
            </div>

            <For
                each=move || {
                    register
                        .with(|r| {
                            r.rows
                                .iter()
                                .enumerate()
                                .map(|(i, row)| (r.generation, i, row.clone()))
                                .collect::<Vec<_>>()
                        })
                }
                key=|(generation, _, row)| (*generation, row.transaction.id.clone())
                children=move |(_, i, row)| {
                    let vid = vid.clone();
                    let matched = row.matched_postings.clone();
                    let real = row.balance_after.clone();
                    let sum = row.filtered_sum_after.clone();
                    let balance = Signal::derive(move || {
                        let mode = balance_mode.get();
                        let amount = match mode {
                            BalanceMode::Real => real.clone(),
                            BalanceMode::FilteredSum => sum.clone(),
                            BalanceMode::Hidden => None,
                        }?;
                        let bounds = axes.with(|a| a.get(&amount.currency_code).copied())?;
                        Some(BalanceCell {
                            amount,
                            axis: bounds,
                        })
                    });
                    view! {
                        <TransactionRow
                            tx=row.transaction
                            matched_postings=matched
                            perspective=RowPerspective::Account {
                                account_id: vid,
                            }
                            selected=Signal::derive(move || selected_idx.get() == Some(i))
                            expanded=Signal::derive(move || expanded_idx.get() == Some(i))
                            on_toggle=Callback::new(move |()| {
                                expanded_idx
                                    .update(|ex| {
                                        *ex = if *ex == Some(i) { None } else { Some(i) };
                                    });
                                selected_idx.set(Some(i));
                            })
                            on_change=on_change_cb
                            on_saved=on_saved_cb
                            accounts=accounts.get_value()
                            balance=balance
                        />
                    }
                }
            />

            {move || {
                register
                    .with(|r| {
                        (!r.fully_loaded())
                            .then(|| {
                                let remaining = r
                                    .total
                                    .saturating_sub(
                                        u32::try_from(r.rows.len()).unwrap_or(u32::MAX),
                                    );
                                let loading = r.loading;
                                view! {
                                    <div class=style::sentinel data-testid="register-sentinel" />
                                    <button
                                        class=style::load_more
                                        data-testid="load-more"
                                        disabled=loading
                                        on:click=move |_| on_load_more.run(())
                                    >
                                        {if loading {
                                            "loading\u{2026}".to_owned()
                                        } else {
                                            format!("load more \u{00b7} {remaining} remaining")
                                        }}
                                    </button>
                                }
                            })
                    })
            }}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
