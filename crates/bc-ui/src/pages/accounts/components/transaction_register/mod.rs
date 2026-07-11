//! Transaction register — column headers, keyboard-navigable row list.

use bc_ipc::AccountRef;
use bc_ipc::FilteredTransaction;
use leptos::prelude::*;
use leptos::web_sys;
use stylance::import_style;
use wasm_bindgen::JsCast as _;

use crate::components::transaction_row::RowPerspective;
use crate::components::transaction_row::TransactionRow;

import_style!(style, "register.module.scss");

/// The full transaction register: column headers and row list.
///
/// Handles keyboard navigation (`j`/`k` to move, `Enter` to expand, `Esc` to
/// collapse) via a `keydown` listener on the register container.
///
/// # Arguments
///
/// * `transactions` - Reactive signal of all transactions for this account.
/// * `viewing_account_id` - The account whose page is currently shown.
/// * `on_change` - Optional callback invoked after any mutation (e.g. reverse)
///   so the parent can refresh its transaction list.
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
    /// Reactive transaction list for this account.
    transactions: Signal<Vec<FilteredTransaction>>,
    /// Account ID being viewed (determines headline amounts).
    #[prop(into)]
    viewing_account_id: String,
    /// Called after any mutation so the parent can bump its data version.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
    /// All selectable accounts for the per-row recategorise picker.
    #[prop(optional)]
    accounts: Vec<AccountRef>,
    /// Page-level period granularity (shared with the dashboard).
    period: RwSignal<bc_ipc::Period>,
    /// Page-level display-window start.
    window_start: RwSignal<jiff::civil::Date>,
) -> impl IntoView {
    let accounts = StoredValue::new(accounts);

    let selected_idx = RwSignal::new(Option::<usize>::None);
    let expanded_idx = RwSignal::new(Option::<usize>::None);

    let filter_store = crate::filter_ctx::use_filter_store();
    let strictness = Signal::derive(move || filter_store.strictness.get());
    let period_locked = Signal::derive(move || {
        filter_store
            .filter
            .with(|f| f.date_from.is_some() || f.date_until.is_some())
    });

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

        let row_count = transactions.get().len();
        if row_count == 0 {
            return;
        }

        match e.key().as_str() {
            "j" | "ArrowDown" => {
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
        crate::pages::accounts::period_notify::notify_if_out_of_period(
            toasts,
            period.get_untracked(),
            window_start,
            date,
        );
    });

    view! {
        <div
            class=style::register
            on:keydown=on_keydown
            tabindex="0"
            aria-label="transaction register"
        >
            <div class=style::header>
                <crate::components::period_nav::PeriodNav
                    period=period
                    window_start=window_start
                    compact=true
                    disabled=period_locked
                />
                <span class=style::reg_title>
                    "register" <span class=style::bracket>"["</span>
                    <span class=style::count>{move || transactions.get().len().to_string()}</span>
                    <span class=style::bracket>"]"</span>
                </span>
            </div>

            <div class=style::col_headers>
                <span class=style::col_date>"date"</span>
                <span>"payee"</span>
                <span class=style::col_category>"category"</span>
                <span class=style::col_amt>"amount"</span>
                <span />
            </div>

            {move || {
                let strict = strictness.get();
                transactions
                    .get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, ft)| {
                        let vid = vid.clone();
                        let matched = ft.matched_postings.clone();
                        view! {
                            <TransactionRow
                                tx=ft.transaction
                                matched_postings=matched
                                strictness=strict
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
                            />
                        }
                    })
                    .collect::<Vec<_>>()
            }}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
