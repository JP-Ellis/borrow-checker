//! Expandable detail panel for a single budget line.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetRevisionView;
use bc_ipc::BudgetTreeNode;
use bc_ipc::Transaction;
use jiff::Span;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::period_nav;
use crate::components::transaction_row::RowPerspective;
use crate::components::transaction_row::TransactionRow;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::components::revision_form::RevisionForm;

import_style!(style, "detail.module.scss");

// MARK: BudgetDetail

/// Expanded detail panel showing the revision timeline, actions, and transactions for a budget.
///
/// Renders as a two-column panel: left column has the revision list with an inline
/// add/amend form and action buttons; right column shows a scrollable list of matched
/// transactions with expandable postings and accrual-spread editors.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "large view! block combining revision timeline, actions, and transaction list columns"
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
    let currencies = crate::currency_ctx::use_currency_store();

    /* --- revision timeline state --- */
    let budget_id_for_revs = StoredValue::new(node.id.clone());
    let revisions: LocalResource<Result<Vec<BudgetRevisionView>, bc_ipc::BcError>> =
        LocalResource::new(move || {
            let bid = budget_id_for_revs.get_value();
            data_version.get();
            let p = period.get();
            let ws = window_start.get();
            let end = period_nav::step_window(&p, ws, true);
            async move { bc_ipc::client::list_budget_revisions(&bid, ws, end).await }
        });

    // None = no editor open; Some(None) = add form; Some(Some(rev)) = amend that revision.
    let editor: RwSignal<Option<Option<BudgetRevisionView>>> = RwSignal::new(None);

    let budget_id_for_form = StoredValue::new(node.id.clone());
    let on_saved = Callback::new(move |()| {
        editor.set(None);
        data_version.update(|v| *v = v.saturating_add(1));
    });
    let on_cancel = Callback::new(move |()| editor.set(None));

    let budget_id_for_remove = StoredValue::new(node.id.clone());
    let remove_revision = move |rev_id: String| {
        let bid = budget_id_for_remove.get_value();
        leptos::task::spawn_local(async move {
            if bc_ipc::client::remove_budget_revision(&bid, &rev_id)
                .await
                .is_ok()
            {
                data_version.update(|v| *v = v.saturating_add(1));
            }
        });
    };

    /* --- archive state --- */
    let confirm_archive = RwSignal::new(false);
    let archiving = RwSignal::new(false);
    let archive_error: RwSignal<Option<String>> = RwSignal::new(None);

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
                    archive_error.set(Some(e.to_string()));
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

    view! {
        <div class=style::panel aria-label="budget detail">

            <div class=style::left_col>
                <div class=style::section_header>"Revisions"</div>

                <Suspense fallback=move || {
                    view! { <div class=style::txn_loading>"Loading revisions\u{2026}"</div> }
                }>
                    {move || {
                        revisions
                            .get()
                            .map(|result| match result {
                                Err(e) => {
                                    view! {
                                        <div class=style::txn_error>{format!("Error: {e}")}</div>
                                    }
                                        .into_any()
                                }
                                Ok(list) => {
                                    let only_one = list.len() == 1;
                                    view! {
                                        <div class=style::rev_list>
                                            <For
                                                each=move || list.clone()
                                                key=|r| (r.id.clone(), r.effective_from)
                                                children=move |r| {
                                                    let rev_for_edit = r.clone();
                                                    let rev_id = StoredValue::new(r.id.clone());
                                                    let active = r.window_overlap.is_some();
                                                    let full = r
                                                        .window_overlap
                                                        .as_ref()
                                                        .is_some_and(|o| o.covers_full_window);
                                                    let reign = r
                                                        .reign_end
                                                        .map_or_else(
                                                            || { format!("from {}", r.effective_from) },
                                                            |e| {
                                                                format!("from {} \u{00b7} until {e}", r.effective_from)
                                                            },
                                                        );
                                                    let target_for_summary = r.target.clone();
                                                    let period_label_for_summary = r.period_label.clone();
                                                    let rollover_for_summary = r.rollover;
                                                    let summary = move || {
                                                        let target_str = target_for_summary
                                                            .as_ref()
                                                            .map_or_else(
                                                                || "tracking".to_owned(),
                                                                |t| {
                                                                    let (sym, after) = crate::currency_ctx::short_symbol(
                                                                        &t.currency_code,
                                                                        &currencies.get(),
                                                                    );
                                                                    t.format_short(sym.as_deref(), after)
                                                                },
                                                            );
                                                        format!(
                                                            "{target_str} \u{00b7} {period_label_for_summary} \u{00b7} {rollover_for_summary}",
                                                        )
                                                    };
                                                    let badge = if !active {
                                                        ("not in window", style::badge_off)
                                                    } else if full {
                                                        ("governing", style::badge_full)
                                                    } else {
                                                        ("partial", style::badge_part)
                                                    };
                                                    view! {
                                                        <div
                                                            data-testid="revision-row"
                                                            class=move || {
                                                                if active { style::rev_active } else { style::rev_row }
                                                            }
                                                            on:click=move |_| {
                                                                editor.set(Some(Some(rev_for_edit.clone())));
                                                            }
                                                        >
                                                            <div class=style::rev_head>
                                                                <span class=style::rev_dates>{reign}</span>
                                                                <span class=badge.1>{badge.0}</span>
                                                                <Show when=move || !only_one>
                                                                    <button
                                                                        class=style::rev_remove
                                                                        on:click=move |ev| {
                                                                            ev.stop_propagation();
                                                                            remove_revision(rev_id.get_value());
                                                                        }
                                                                    >
                                                                        "\u{00d7}"
                                                                    </button>
                                                                </Show>
                                                            </div>
                                                            <div class=style::rev_cfg>{summary}</div>
                                                        </div>
                                                    }
                                                }
                                            />
                                        </div>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </Suspense>

                {move || match editor.get() {
                    None => {
                        view! {
                            <button
                                class=style::add_rev_btn
                                on:click=move |_| editor.set(Some(None))
                            >
                                "＋ add revision"
                            </button>
                        }
                            .into_any()
                    }
                    Some(None) => {
                        view! {
                            <RevisionForm
                                budget_id=budget_id_for_form.get_value()
                                allow_snap=true
                                on_saved=on_saved
                                on_cancel=on_cancel
                            />
                        }
                            .into_any()
                    }
                    Some(Some(rev)) => {
                        view! {
                            <RevisionForm
                                budget_id=budget_id_for_form.get_value()
                                revision=rev
                                allow_snap=true
                                on_saved=on_saved
                                on_cancel=on_cancel
                            />
                        }
                            .into_any()
                    }
                }}

                <div class=style::divider />

                <div class=style::section_header>"Actions"</div>

                <div class=style::actions>
                    <button class=style::action_btn>"↗ View in Accounts"</button>

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
                                                "Archiving\u{2026}"
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
                                    "\u{2298} Archive budget"
                                </button>
                            }
                                .into_any()
                        }
                    }}

                    {move || {
                        archive_error.get().map(|msg| view! { <p class=style::err>{msg}</p> })
                    }}
                </div>
            </div>

            <div class=style::right_col>
                <div class=style::section_header>"Transactions"</div>

                <Suspense fallback=move || {
                    view! { <div class=style::txn_loading>"Loading transactions\u{2026}"</div> }
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
                                    let acct = StoredValue::new(node.account_id.clone());
                                    let filter = StoredValue::new(node.tag_filter.clone());
                                    let p = period.get();
                                    let ws = window_start.get();
                                    let next_start = period_nav::step_window(&p, ws, true);
                                    let we = next_start.saturating_sub(Span::new().days(1_i64));
                                    view! {
                                        <div class=style::txn_list>
                                            <For
                                                each=move || list.clone()
                                                key=|tx| tx.id.clone()
                                                children=move |tx| {
                                                    view! {
                                                        <TransactionRow
                                                            tx=tx
                                                            perspective=RowPerspective::Budget {
                                                                account_id: acct.get_value(),
                                                                tag_filter: filter.get_value(),
                                                                window_start: ws,
                                                                window_end: we,
                                                            }
                                                            on_change=on_change
                                                        />
                                                    }
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
