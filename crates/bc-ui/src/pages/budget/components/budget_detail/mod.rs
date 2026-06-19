//! Expandable detail panel for a single budget line.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::num::to_decimal_string;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::components::accrual_editor::AccrualEditor;
use crate::pages::budget::period_nav;

import_style!(style, "detail.module.scss");

// MARK: Helpers

/// Formats an [`Amount`] as a plain decimal string suitable for an `<input>` field.
///
/// Does not use `Intl.NumberFormat` — input fields must receive a bare decimal
/// without currency symbols or grouping separators. Budget targets are always
/// non-negative, so the unsigned absolute value is sufficient.
#[must_use]
#[inline]
fn format_target(amount: &Amount) -> String {
    to_decimal_string(amount.minor_units.unsigned_abs(), amount.scale)
}

/// Parses a target input string into minor units using the given scale.
///
/// Returns `None` when the string is empty or cannot be parsed as a valid decimal.
/// Negative inputs are not supported (budget targets are always non-negative).
#[must_use]
#[inline]
#[expect(
    clippy::string_slice,
    reason = "minor is produced by split_once('.') on a user-typed decimal string; indexing by byte count is safe for ASCII digit input"
)]
fn parse_target(s: &str, scale: u8) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let scale_factor = 10_i64.pow(u32::from(scale));
    match s.split_once('.') {
        None => {
            if s.starts_with('-') {
                return None;
            }
            s.parse::<i64>()
                .ok()
                .map(|n| n.saturating_mul(scale_factor))
        }
        Some((major, minor)) => {
            if major.starts_with('-') {
                return None;
            }
            let maj: i64 = major.parse().ok()?;
            let scale_usize = usize::from(scale);
            let minor_trimmed = &minor[..minor.len().min(scale_usize)];
            let min_str = format!("{minor_trimmed:0<scale_usize$}");
            let min: i64 = min_str.parse().ok()?;
            Some(maj.saturating_mul(scale_factor).saturating_add(min))
        }
    }
}

/// Sums the positive posting amounts for a transaction's display total.
///
/// Returns a zeroed [`Amount`] using the first posting's currency when there are no positive
/// postings.
#[must_use]
#[inline]
fn tx_display_amount(tx: &Transaction) -> Amount {
    let first = tx.postings.first();
    let currency = first.map_or("", |p| p.amount.currency_code.as_str());
    let scale = first.map_or(2, |p| p.amount.scale);

    let total: i64 = tx
        .postings
        .iter()
        .filter(|p| p.amount.minor_units > 0)
        .map(|p| p.amount.minor_units)
        .fold(0_i64, i64::saturating_add);

    Amount::new(total, currency, scale)
}

// MARK: PostingRow

/// Renders a single posting sub-row with optional accrual-spread editor.
#[component]
fn PostingRow(
    /// The posting to display.
    posting: Posting,
    /// Callback invoked after a successful accrual spread change.
    on_change: Callback<()>,
) -> impl IntoView {
    let spread_open = RwSignal::new(false);
    let has_spread = posting.spread_from.is_some();
    let spread_from = posting.spread_from.map(|d| d.to_string());
    let spread_until = posting.spread_until.map(|d| d.to_string());
    let posting_id = posting.id.clone();

    let spread_label = match (&spread_from, &spread_until) {
        (Some(f), Some(u)) => format!("{f} \u{2013} {u}"),
        (Some(f), None) => f.clone(),
        _ => String::new(),
    };

    let btn_label = move || {
        if spread_open.get() {
            "Hide spread"
        } else if has_spread {
            "Edit spread"
        } else {
            "Add spread"
        }
    };

    view! {
        <div>
            <div class=style::posting_row>
                <span>{posting.account.name.clone()}</span>
                <span class=style::posting_amount>{posting.amount.format_short()}</span>
            </div>
            {has_spread
                .then(|| {
                    view! {
                        <div class=style::spread_badge_row>
                            <span class=style::accrual_badge>"\u{27F3} accrued"</span>
                            <span class=style::spread_dates>{spread_label.clone()}</span>
                        </div>
                    }
                })}
            <div class=style::spread_badge_row>
                <button
                    class=style::spread_edit_btn
                    on:click=move |_| spread_open.update(|o| *o = !*o)
                >
                    {btn_label}
                </button>
            </div>
            <Show when=move || spread_open.get()>
                <AccrualEditor
                    posting_id=posting_id.clone()
                    has_spread=has_spread
                    spread_from=spread_from.clone().unwrap_or_default()
                    spread_until=spread_until.clone().unwrap_or_default()
                    on_change=on_change
                />
            </Show>
        </div>
    }
}

// MARK: TxRow

/// Renders a single transaction row and its expandable postings detail.
#[component]
fn TxRow(
    /// The transaction to display.
    tx: Transaction,
    /// Callback invoked after a successful accrual spread change.
    on_change: Callback<()>,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let display_amount = tx_display_amount(&tx);
    let postings = StoredValue::new(tx.postings.clone());

    view! {
        <div>
            <div
                class=move || if expanded.get() { style::txn_row_expanded } else { style::txn_row }
                on:click=move |_| expanded.update(|e| *e = !*e)
            >
                <span class=style::txn_date>{tx.date.to_string()}</span>
                <span>{tx.payee.clone()}</span>
                <span class=style::txn_amt>{display_amount.format_short()}</span>
            </div>
            <Show when=move || expanded.get()>
                <div class=style::txn_detail>
                    <For
                        each=move || postings.get_value()
                        key=|p| p.id.clone()
                        children=move |p| {
                            view! { <PostingRow posting=p on_change=on_change /> }
                        }
                    />
                </div>
            </Show>
        </div>
    }
}

// MARK: BudgetDetail

/// Expanded detail panel showing settings, actions, and transactions for a budget.
///
/// Renders as a two-column panel: left column has editable settings and action
/// buttons; right column shows a scrollable list of matched transactions with
/// expandable postings and accrual-spread editors.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props must be owned values"
)]
#[expect(
    clippy::too_many_lines,
    reason = "large view! block combining settings, actions, and transaction list columns"
)]
pub fn BudgetDetail(
    /// The tree node whose detail is being displayed.
    node: BudgetTreeNode,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let data_version = ctx.data_version;
    let open_detail_id = ctx.open_detail_id;
    let period = ctx.display_period;
    let window_start = ctx.window_start;

    /* --- initial field values --- */
    let initial_name = StoredValue::new(
        node.name
            .clone()
            .unwrap_or_else(|| node.account_name.clone()),
    );

    let initial_target = StoredValue::new(
        node.effective_target
            .as_ref()
            .map_or_else(String::new, format_target),
    );

    let target_scale = node.effective_target.as_ref().map_or(2_u8, |a| a.scale);

    let target_currency: StoredValue<String> =
        StoredValue::new(node.effective_target.as_ref().map_or_else(
            || node.spent.currency_code.clone(),
            |a| a.currency_code.clone(),
        ));

    /* --- local reactive state --- */
    let name_input = RwSignal::new(initial_name.get_value());
    let target_input = RwSignal::new(initial_target.get_value());
    let dirty = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let save_error: RwSignal<Option<String>> = RwSignal::new(None);
    let confirm_archive = RwSignal::new(false);
    let archiving = RwSignal::new(false);

    /* --- save handler --- */
    let budget_id_for_save = StoredValue::new(node.id.clone());
    let save = move |_| {
        let budget_id = budget_id_for_save.get_value();
        let name_val = name_input.get_untracked();
        let target_str = target_input.get_untracked();
        let target_parsed = parse_target(&target_str, target_scale);
        let currency = target_currency.get_value();

        saving.set(true);
        save_error.set(None);

        leptos::task::spawn_local(async move {
            let result = bc_ipc::client::update_budget(
                &budget_id,
                Some(Some(name_val)),
                target_parsed,
                Some(currency.as_str()),
                None,
                None,
                None,
            )
            .await;
            saving.set(false);
            match result {
                Ok(()) => {
                    dirty.set(false);
                    data_version.update(|v| *v = v.saturating_add(1));
                }
                Err(e) => {
                    save_error.set(Some(e.to_string()));
                }
            }
        });
    };

    /* --- reset handler --- */
    let reset = move |_| {
        name_input.set(initial_name.get_value());
        target_input.set(initial_target.get_value());
        dirty.set(false);
        save_error.set(None);
    };

    /* --- archive handler --- */
    let budget_id_for_archive = StoredValue::new(node.id.clone());
    let do_archive = move |_| {
        let budget_id = budget_id_for_archive.get_value();
        archiving.set(true);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::archive_budget(&budget_id).await {
                Ok(()) => {
                    archiving.set(false);
                    open_detail_id.set(None);
                    data_version.update(|v| *v = v.saturating_add(1));
                }
                Err(e) => {
                    archiving.set(false);
                    save_error.set(Some(e.to_string()));
                }
            }
        });
    };

    /* --- transaction list --- */
    let budget_id_for_txns = StoredValue::new(node.id.clone());
    let txns: LocalResource<Result<Vec<Transaction>, bc_ipc::BcError>> =
        LocalResource::new(move || {
            let bid = budget_id_for_txns.get_value();
            data_version.get();
            let p = period.get();
            let ws = window_start.get();
            let end = period_nav::step_window(&p, ws, true);
            async move { bc_ipc::client::get_budget_transactions(&bid, ws, end).await }
        });

    let on_change: Callback<()> = Callback::new(move |()| {
        data_version.update(|v| *v = v.saturating_add(1));
    });

    /* --- static display strings --- */
    let period_label = node.native_period_label.clone();
    let rollover_label = node
        .rollover
        .as_ref()
        .map_or_else(String::new, std::string::ToString::to_string);
    let tag_filter_label = node.tag_filter.clone().unwrap_or_else(|| "none".to_owned());

    view! {
        <div class=style::panel aria-label="budget detail">

            <div class=style::left_col>
                <div class=style::section_header>"Settings"</div>

                <div class=style::field_row>
                    <span class=style::field_label>"Name"</span>
                    <input
                        type="text"
                        class=style::field_input
                        prop:value=move || name_input.get()
                        on:input=move |ev| {
                            name_input.set(event_target_value(&ev));
                            dirty.set(true);
                        }
                    />
                </div>

                <div class=style::field_row>
                    <span class=style::field_label>"Target"</span>
                    <input
                        type="number"
                        step="0.01"
                        class=style::field_input
                        prop:value=move || target_input.get()
                        on:input=move |ev| {
                            target_input.set(event_target_value(&ev));
                            dirty.set(true);
                        }
                    />
                </div>

                <div class=style::field_row>
                    <span class=style::field_label>"Period"</span>
                    <span class=style::field_static>{period_label}</span>
                </div>

                <div class=style::field_row>
                    <span class=style::field_label>"Rollover"</span>
                    <span class=style::field_static>{rollover_label}</span>
                </div>

                <div class=style::field_row>
                    <span class=style::field_label>"Tag filter"</span>
                    <span class=style::field_static>{tag_filter_label}</span>
                </div>

                {move || { save_error.get().map(|msg| view! { <p class=style::err>{msg}</p> }) }}

                <Show when=move || dirty.get()>
                    <div class=style::btn_row>
                        <button class=style::btn_save disabled=move || saving.get() on:click=save>
                            {move || if saving.get() { "Saving…" } else { "Save" }}
                        </button>
                        <button class=style::btn_reset on:click=reset>
                            "Reset"
                        </button>
                    </div>
                </Show>

                <div class=style::divider />

                <div class=style::section_header>"Actions"</div>

                <div class=style::actions>
                    <button class=style::action_btn>"↗ View in Accounts"</button>
                    <button class=style::action_btn>"＋ Add budget line"</button>

                    {move || {
                        if confirm_archive.get() {
                            view! {
                                <div class=style::confirm_row>
                                    <span class=style::confirm_text>"Archive this budget?"</span>
                                    <button
                                        class=style::btn_archive
                                        disabled=move || archiving.get()
                                        on:click=do_archive
                                    >
                                        {move || {
                                            if archiving.get() {
                                                "Archiving…"
                                            } else {
                                                "Yes, archive"
                                            }
                                        }}
                                    </button>
                                    <button
                                        class=style::action_btn
                                        on:click=move |_| confirm_archive.set(false)
                                    >
                                        "Cancel"
                                    </button>
                                </div>
                            }
                                .into_any()
                        } else {
                            view! {
                                <button
                                    class=style::action_btn_danger
                                    on:click=move |_| confirm_archive.set(true)
                                >
                                    "⊘ Archive budget"
                                </button>
                            }
                                .into_any()
                        }
                    }}
                </div>
            </div>

            <div class=style::right_col>
                <div class=style::section_header>"Transactions"</div>

                <Suspense fallback=move || {
                    view! { <div class=style::txn_loading>"Loading transactions…"</div> }
                }>
                    {move || {
                        txns.get()
                            .map(|result| match result {
                                Err(e) => {
                                    view! {
                                        <div class=style::txn_error>{format!("Error: {e}")}</div>
                                    }
                                        .into_any()
                                }
                                Ok(list) if list.is_empty() => {
                                    view! {
                                        <div class=style::txn_empty>
                                            "// no transactions in this period"
                                        </div>
                                    }
                                        .into_any()
                                }
                                Ok(list) => {
                                    view! {
                                        <div class=style::txn_list>
                                            <For
                                                each=move || list.clone()
                                                key=|tx| tx.id.clone()
                                                children=move |tx| {
                                                    view! { <TxRow tx=tx on_change=on_change /> }
                                                }
                                            />
                                        </div>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </Suspense>
            </div>
        </div>
    }
}
