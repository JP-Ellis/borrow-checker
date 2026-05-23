//! Transaction register — filter bar, column headers, keyboard-navigable row list.

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;
use leptos::web_sys;
use stylance::import_style;

use super::transaction_row::TransactionRow;

import_style!(style, "register.module.scss");

/// Monotonic counter for generating unique per-instance anchor names and popover IDs.
static REGISTER_INSTANCE: AtomicUsize = AtomicUsize::new(0);

/// Filter options for the register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Filter {
    /// Show all transactions.
    #[default]
    All,
    /// Only uncleared/pending.
    Pending,
    /// Only transactions without an envelope assignment.
    Uncategorised,
}

impl Filter {
    /// Returns the display label for this filter.
    #[must_use]
    #[inline]
    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Pending => "pending",
            Self::Uncategorised => "uncategorised",
        }
    }
}

/// The full transaction register: filter bar, column headers, and row list.
///
/// Handles keyboard navigation (`j`/`k` to move, `Enter` to expand, `Esc` to
/// collapse) via a `keydown` listener on the register container.
///
/// # Arguments
///
/// * `transactions` - Reactive signal of all transactions for this account.
/// * `viewing_account_id` - The account whose page is currently shown.
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
    transactions: Signal<Vec<Transaction>>,
    /// Account ID being viewed (determines headline amounts).
    #[prop(into)]
    viewing_account_id: String,
) -> impl IntoView {
    let instance = REGISTER_INSTANCE.fetch_add(1, Ordering::Relaxed);
    let anchor_name = format!("--bc-reg-filter-{instance}");
    let popover_id = format!("bc-register-filter-menu-{instance}");

    let active_filter = RwSignal::new(Filter::All);
    let selected_idx = RwSignal::new(Option::<usize>::None);
    let expanded_idx = RwSignal::new(Option::<usize>::None);

    let filtered = Memo::new(move |_| {
        transactions
            .get()
            .into_iter()
            .filter(|tx| match active_filter.get() {
                Filter::Pending => tx.status == TxStatus::Pending,
                // TODO: filter by missing envelope_id once the field is available on postings
                Filter::All | Filter::Uncategorised => true,
            })
            .collect::<Vec<_>>()
    });

    let on_keydown = move |e: web_sys::KeyboardEvent| {
        let row_count = filtered.get().len();
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

    view! {
        <div
            class=style::register
            on:keydown=on_keydown
            tabindex="0"
            aria-label="transaction register"
        >
            <div class=style::header>
                <span class=style::reg_title>
                    "register" <span class=style::bracket>"["</span>
                    <span class=style::count>{move || filtered.get().len().to_string()}</span>
                    <span class=style::bracket>"]"</span>
                </span>

                <div class=style::filter_area>
                    <button
                        class=style::filter_toggle
                        style=format!("anchor-name: {anchor_name}")
                        popovertarget=popover_id.clone()
                    >
                        {move || format!("{} ▾", active_filter.get().label())}
                    </button>
                </div>
                <div
                    popover=""
                    id=popover_id.clone()
                    class=style::filter_menu
                    style=format!("position-anchor: {anchor_name}")
                >
                    {[Filter::All, Filter::Pending, Filter::Uncategorised]
                        .into_iter()
                        .map(|f| {
                            let popover_id = popover_id.clone();
                            view! {
                                <button
                                    class=move || {
                                        if active_filter.get() == f {
                                            format!(
                                                "{} {}",
                                                style::filter_menu_btn,
                                                style::filter_active,
                                            )
                                        } else {
                                            style::filter_menu_btn.to_owned()
                                        }
                                    }
                                    popovertarget=popover_id
                                    popovertargetaction="hide"
                                    on:click=move |_| active_filter.set(f)
                                >
                                    {f.label()}
                                </button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>

                <div class=style::filters>
                    {[Filter::All, Filter::Pending, Filter::Uncategorised]
                        .into_iter()
                        .map(|f| {
                            view! {
                                <button
                                    class=move || {
                                        if active_filter.get() == f {
                                            format!("{} {}", style::filter_btn, style::filter_active)
                                        } else {
                                            style::filter_btn.to_owned()
                                        }
                                    }
                                    on:click=move |_| active_filter.set(f)
                                >
                                    {f.label()}
                                </button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </div>

            <div class=style::col_headers>
                <span class=style::col_date>"date"</span>
                <span>"payee"</span>
                <span class=style::col_envelope>"envelope"</span>
                <span class=style::col_amt>"amount"</span>
                <span />
            </div>

            {move || {
                filtered
                    .get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, tx)| {
                        let vid = vid.clone();
                        view! {
                            <TransactionRow
                                tx=tx
                                viewing_account_id=vid
                                selected=Signal::derive(move || selected_idx.get() == Some(i))
                                expanded=Signal::derive(move || expanded_idx.get() == Some(i))
                                on_toggle=Callback::new(move |()| {
                                    expanded_idx
                                        .update(|ex| {
                                            *ex = if *ex == Some(i) { None } else { Some(i) };
                                        });
                                    selected_idx.set(Some(i));
                                })
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
