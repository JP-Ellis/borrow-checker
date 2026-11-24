//! Transaction register — column headers, keyboard-navigable row list.

use core::time::Duration;

use bc_ipc::AccountRef;
use bc_ipc::RegisterRow;
use bc_ipc::WriteWarning;
use leptos::prelude::*;
use leptos::web_sys;
use stylance::import_style;
use wasm_bindgen::JsCast as _;

use crate::components::balance_cell::BalanceCell;
use crate::components::period_nav::DisplayWindow;
use crate::components::toast::ToastAction;
use crate::components::transaction_row::DetailSource;
use crate::components::transaction_row::RowPerspective;
use crate::components::transaction_row::Saved;
use crate::components::transaction_row::TransactionDetail;
use crate::components::transaction_row::TransactionRow;
use crate::components::transaction_row::editable::EditableTransaction;
use crate::pages::accounts::register_pages::BalanceMode;
use crate::pages::accounts::register_pages::LinkOutcome;
use crate::pages::accounts::register_pages::LoadTrigger;
use crate::pages::accounts::register_pages::LoadedRegister;
use crate::pages::accounts::register_pages::Step;
use crate::pages::accounts::register_pages::axes_for;
use crate::pages::accounts::register_pages::balance_value;
use crate::pages::accounts::register_pages::created_row_shows;
use crate::pages::accounts::register_pages::link_outcome;
use crate::pages::accounts::register_pages::next_selection;
use crate::pages::accounts::register_pages::retain_present;
use crate::pages::accounts::register_scroll;
use crate::pending_warnings::WarnedToast;
use crate::pending_warnings::toast_created;
use crate::pending_warnings::use_pending_warnings;
use crate::transaction_link::use_transaction_link;
use crate::url_state::History;

import_style!(style, "register.module.scss");

/// Selects the new-transaction draft's wrapper.
pub const DRAFT_SELECTOR: &str = r#"[data-testid="register-draft"]"#;

/// Which row has the keyboard cursor and which is expanded, by transaction id.
///
/// Kept out of [`LoadedRegister`] so moving the cursor does not re-run the row
/// list. The page mounts one register per account, so a switch starts empty.
#[derive(Clone, Copy)]
struct RowFocus {
    /// The keyboard-selected transaction.
    selected: RwSignal<Option<String>>,
    /// The expanded transaction.
    expanded: RwSignal<Option<String>>,
}

impl RowFocus {
    /// Nothing selected or expanded.
    fn new() -> Self {
        Self {
            selected: RwSignal::new(None),
            expanded: RwSignal::new(None),
        }
    }

    /// Drops either id whose transaction is not among `rows`.
    fn retain_present(self, rows: &[RegisterRow]) {
        for signal in [self.selected, self.expanded] {
            let kept = retain_present(signal.get_untracked(), rows);
            if kept != signal.get_untracked() {
                signal.set(kept);
            }
        }
    }
}

/// A transaction the register opens once its rows settle.
#[derive(Clone)]
struct Linked {
    /// The transaction to open.
    id: String,
    /// Warnings a found row also announces in a transient toast; empty for
    /// none.
    announce: Vec<WriteWarning>,
    /// The action on the sticky toast that carries a missing row's warnings.
    view: Option<ToastAction>,
    /// Whether `tx` in the URL named it, so settling drops `tx`.
    from_url: bool,
}

/// A memo over one field of the row at `index`, whose transaction is `id`.
///
/// It notifies only when the field's value changes, so a refetch that
/// leaves the row alone re-renders nothing. It reads a row only when that
/// row's transaction is `id`; once `index` runs past the loaded rows, or
/// briefly lands on another row before a re-sort updates it, the memo keeps
/// its last value.
fn row_field<T>(
    register: Signal<LoadedRegister>,
    index: ReadSignal<usize>,
    id: String,
    initial: T,
    field: fn(&RegisterRow) -> &T,
) -> Memo<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    Memo::new_owning(move |held: Option<T>| {
        let first = held.is_none();
        let held = held.unwrap_or_else(|| initial.clone());
        register.with(|r| {
            let value = r
                .rows
                .get(index.get())
                .filter(|row| row.transaction.id == id)
                .map(field);
            match value {
                Some(value) if *value != held => (value.clone(), true),
                _ => (held, first),
            }
        })
    })
}

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
/// * `focal_account_ids` - Accounts whose postings form each row's amount: the
///   viewed account, plus its descendants when the page rolls them up.
/// * `on_change` - Optional callback invoked with the mutated row's
///   transaction id after any mutation (e.g. reverse), so the parent can
///   refresh its transaction list.
/// * `accounts` - All selectable accounts for the per-row recategorise picker.
/// * `busy` - `true` while `register` still shows a previous request.
/// * `on_latest` - Moves the window to the period of the latest matching
///   transaction.
/// * `latest_busy` - `true` while that jump is fetching.
/// * `draft` - The new-transaction draft shown above the first row, if any.
///   The register clears it on discard and after the save.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn TransactionRegister(
    /// Everything loaded so far.
    register: Signal<LoadedRegister>,
    /// Asks the page for the next page of rows; `Explicit` retries after a failure.
    on_load_more: Callback<LoadTrigger>,
    /// What the balance column shows (page-owned, persisted).
    balance_mode: RwSignal<BalanceMode>,
    /// Accounts whose postings form each row's amount and are left out of
    /// its category. A change recomputes every row's header in place.
    #[prop(into)]
    focal_account_ids: Signal<Vec<String>>,
    /// Called with the mutated row's transaction id after any mutation.
    #[prop(optional)]
    on_change: Option<Callback<String>>,
    /// All selectable accounts for the per-row recategorise picker. A mounted
    /// row's pickers follow changes to it.
    #[prop(optional, into)]
    accounts: Signal<Vec<AccountRef>>,
    /// `true` while `register` still shows a previous request; rendered as
    /// `aria-busy` so assistive tech and the e2e suite can tell stale rows
    /// from settled ones.
    #[prop(optional, into)]
    busy: Signal<bool>,
    /// Moves the window to the period of the latest matching transaction.
    on_latest: Callback<()>,
    /// `true` while the jump to the latest period is fetching.
    #[prop(into)]
    latest_busy: Signal<bool>,
    /// The new-transaction draft shown above the first row, if any.
    draft: RwSignal<Option<EditableTransaction>>,
) -> impl IntoView {
    // A reset can land rows without the selected or expanded transaction;
    // drop the stale id so it cannot resurface if the row comes back. Rows
    // still on screen during a load are the previous request's, so the check
    // waits for the refetch to land.
    let focus = RowFocus::new();
    Effect::new(move |_| {
        register.with(|r| {
            if !r.loading {
                focus.retain_present(&r.rows);
            }
        });
    });

    // After a switch the rows are empty while the first page loads. The
    // placeholder waits 150 ms so a fast response goes straight to rows.
    let show_placeholder = RwSignal::new(false);
    let waiting = Memo::new(move |_| register.with(|r| r.loading && r.rows.is_empty()));
    Effect::new(move |prev: Option<Option<TimeoutHandle>>| {
        if let Some(Some(handle)) = prev {
            handle.clear();
        }
        if waiting.get() {
            set_timeout_with_handle(
                move || {
                    show_placeholder.try_set(true);
                },
                core::time::Duration::from_millis(150),
            )
            .ok()
        } else {
            show_placeholder.set(false);
            None
        }
    });

    let filter_store = crate::filter_ctx::use_filter_store();
    let window = filter_store.window;
    let pending = use_pending_warnings();
    let toasts = crate::components::toast::use_toasts();
    let register_ref = NodeRef::<leptos::html::Div>::new();

    // MARK: Linked transaction

    // A row to open once the next settled rows land: named by `tx` in the
    // URL, or by a warned save. The register opens it if it is among them;
    // otherwise any warnings pending for it return as a sticky toast. A zero
    // timeout is a macrotask, so it runs after every effect the navigation or
    // save woke: a reset the change starts has already marked `loading`.
    let linked = RwSignal::new(None::<Linked>);
    let arm = move |target: Linked| {
        set_timeout(
            move || {
                let current = !target.from_url
                    || filter_store.tx.try_get_untracked().flatten().as_deref()
                        == Some(target.id.as_str());
                if current {
                    linked.try_set(Some(target));
                }
            },
            Duration::ZERO,
        );
    };
    Effect::new(move |_| {
        let Some(id) = filter_store.tx.get() else {
            return;
        };
        arm(Linked {
            id,
            announce: Vec::new(),
            view: None,
            from_url: true,
        });
    });
    Effect::new(move |_| {
        let Some(target) = linked.get() else {
            return;
        };
        let id = target.id;
        let Some(outcome) = register
            .with(|r| (!r.loading).then(|| link_outcome(&r.rows, &id, || pending.take(&id))))
        else {
            return;
        };
        linked.set(None);
        match outcome {
            LinkOutcome::Expand => {
                // An open row keeps its detail, and with it the band.
                if focus.expanded.get_untracked().as_deref() != Some(id.as_str()) {
                    focus.expanded.set(Some(id.clone()));
                }
                focus.selected.set(Some(id.clone()));
                if !target.announce.is_empty() {
                    toast_created(toasts, &target.announce, WarnedToast::Transient);
                }
                set_timeout(
                    move || {
                        // A detail that mounted took its warnings; one that
                        // stayed open had them already, so drop the leftover.
                        drop(pending.take(&id));
                        if let Some(el) = register_ref.try_get_untracked().flatten() {
                            register_scroll::reveal(&el, &id);
                        }
                    },
                    Duration::ZERO,
                );
            }
            // No row carries the warnings: a "View" that linked the row
            // dismissed the only toast with them, and a save had only the
            // band. They return as a sticky toast.
            LinkOutcome::Rewarn(warnings) => {
                toast_created(toasts, &warnings, WarnedToast::Sticky(target.view));
            }
            LinkOutcome::Silent => {}
        }
        if target.from_url {
            filter_store.clear_tx();
        }
    });
    let period_locked = Signal::derive(move || {
        filter_store
            .filter
            .with(crate::pages::accounts::query::query_sets_dates)
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
            // The draft is not a row: Escape would collapse the expanded row
            // and `j`/`k` would move the cursor.
            if t.closest(DRAFT_SELECTOR).ok().flatten().is_some() {
                return;
            }
        }

        if row_count.get_untracked() == 0 {
            return;
        }

        let rows_at_end = |id: Option<&str>| {
            register.with_untracked(|r| r.rows.last().map(|row| row.transaction.id.as_str()) == id)
        };
        match e.key().as_str() {
            "j" | "ArrowDown" => {
                let current = focus.selected.get_untracked();
                if rows_at_end(current.as_deref()) {
                    on_load_more.run(LoadTrigger::Explicit);
                }
                let next = register
                    .with_untracked(|r| next_selection(&r.rows, current.as_deref(), Step::Down));
                focus.selected.set(next);
                e.prevent_default();
            }
            "k" | "ArrowUp" => {
                let current = focus.selected.get_untracked();
                let next = register
                    .with_untracked(|r| next_selection(&r.rows, current.as_deref(), Step::Up));
                focus.selected.set(next);
                e.prevent_default();
            }
            "Enter" => {
                if let Some(id) = focus.selected.get_untracked() {
                    focus.expanded.update(|ex| {
                        *ex = if ex.as_deref() == Some(id.as_str()) {
                            None
                        } else {
                            Some(id)
                        };
                    });
                }
                e.prevent_default();
            }
            "Escape" => {
                focus.expanded.set(None);
                e.prevent_default();
            }
            _ => {}
        }
    };

    // Shared by every row; a rollup toggle recomputes headers in place.
    let perspective = Memo::new(move |_| RowPerspective::Account {
        account_ids: focal_account_ids.get(),
    });

    let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|_: String| {}));

    let on_saved_cb = Callback::new(move |date: jiff::civil::Date| {
        crate::pages::accounts::period_notify::notify_if_out_of_period(toasts, filter_store, date);
    });

    // A warned edit keeps its band while its row stays; a row the edit moved
    // out of the register leaves a sticky toast that opens it elsewhere.
    let link = use_transaction_link();
    let on_warned_cb = Callback::new(move |saved: Saved| {
        pending.set(saved.id.clone(), saved.warnings.clone());
        let view = link.view_action(&saved);
        arm(Linked {
            id: saved.id,
            announce: Vec::new(),
            view,
            from_url: false,
        });
    });

    // MARK: Draft

    // A warned save opens its row on the warnings when the row should appear
    // here, and falls back to a sticky toast if it does not; a row that
    // cannot appear gets the sticky toast with "View" at once.
    let on_created = Callback::new(move |created: Saved| {
        draft.set(None);
        on_change_cb.run(created.id.clone());
        crate::pages::accounts::period_notify::notify_if_out_of_period(
            toasts,
            filter_store,
            created.date,
        );
        if created.warnings.is_empty() {
            toast_created(toasts, &created.warnings, WarnedToast::Transient);
            return;
        }
        let shows = focal_account_ids.with_untracked(|ids| {
            created_row_shows(
                &created.accounts,
                ids,
                window.with_untracked(|w| w.contains(created.date)),
                filter_store
                    .filter
                    .with_untracked(|f| !f.query.trim().is_empty()),
            )
        });
        if shows {
            pending.set(created.id.clone(), created.warnings.clone());
            arm(Linked {
                id: created.id,
                announce: created.warnings,
                view: None,
                from_url: false,
            });
        } else {
            toast_created(
                toasts,
                &created.warnings,
                WarnedToast::Sticky(link.view_action(&created)),
            );
        }
    });
    let on_discard = Callback::new(move |()| draft.set(None));
    let draft_open = Memo::new(move |_| draft.with(Option::is_some));

    view! {
        <div
            class=style::register
            node_ref=register_ref
            style:--bc-date-col=move || crate::components::transaction_row::date_col_width(
                window.with(DisplayWindow::year).is_some(),
            )
            data-balance=move || {
                if balance_mode.get() == BalanceMode::Hidden { "hidden" } else { "shown" }
            }
            on:keydown=on_keydown
            tabindex="0"
            aria-label="transaction register"
            aria-busy=move || if busy.get() { "true" } else { "false" }
        >
            <div class=style::header>
                <crate::components::period_nav::WindowNav
                    window=window
                    on_change=Callback::new(move |(next, history): (DisplayWindow, History)| {
                        filter_store.set_window(next, history);
                    })
                    on_latest=Some(on_latest)
                    latest_busy=latest_busy
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

            // The draft sits outside `RowFocus`, so expanding a row leaves it
            // open.
            {move || {
                draft_open
                    .get()
                    .then(|| draft.get_untracked())
                    .flatten()
                    .map(|seed| {
                        view! {
                            <div class=style::draft data-testid="register-draft">
                                <TransactionDetail
                                    source=DetailSource::New(seed)
                                    on_created=on_created
                                    on_discard=on_discard
                                    accounts=accounts
                                />
                            </div>
                        }
                    })
            }}

            {move || {
                show_placeholder
                    .get()
                    .then(|| {
                        view! {
                            <div class=style::placeholder role="status">
                                "loading\u{2026}"
                            </div>
                        }
                    })
            }}

            {move || {
                let empty = register.with(|r| r.rows.is_empty() && r.fully_loaded() && !r.loading);
                empty
                    .then(|| {
                        let text = if window.with(|w| w.period().is_some()) {
                            "// no transactions in this period"
                        } else {
                            "// no transactions"
                        };
                        view! {
                            <div class=style::placeholder role="status">
                                {text}
                            </div>
                        }
                    })
            }}

            <ForEnumerate
                each=move || {
                    register
                        .with(|r| {
                            r.rows.iter().map(|row| row.transaction.id.clone()).collect::<Vec<_>>()
                        })
                }
                key=|id: &String| id.clone()
                children=move |index, id: String| {
                    let Some(initial) = register
                        .with_untracked(|r| r.rows.get(index.get_untracked()).cloned()) else {
                        return ().into_any();
                    };
                    let tx = row_field(
                        register,
                        index,
                        id.clone(),
                        initial.transaction,
                        |row| &row.transaction,
                    );
                    let matched = row_field(
                        register,
                        index,
                        id.clone(),
                        initial.matched_postings,
                        |row| &row.matched_postings,
                    );
                    let id_sel = id.clone();
                    let id_exp = id.clone();
                    let id_changed = id.clone();
                    let balance = Signal::derive(move || {
                        let mode = balance_mode.get();
                        let amount = register
                            .with(|r| balance_value(r.rows.get(index.get())?, mode).cloned())?;
                        let bounds = axes.with(|a| a.get(&amount.currency_code).copied())?;
                        Some(BalanceCell {
                            amount,
                            axis: bounds,
                        })
                    });
                    // `balance` reads the current rows directly: balances change
                    // on every amend without changing the row.
                    view! {
                        <TransactionRow
                            tx=tx
                            matched_postings=matched
                            perspective=perspective
                            selected=Signal::derive(move || {
                                focus.selected.with(|s| s.as_deref() == Some(id_sel.as_str()))
                            })
                            expanded=Signal::derive(move || {
                                focus.expanded.with(|s| s.as_deref() == Some(id_exp.as_str()))
                            })
                            on_toggle=Callback::new(move |()| {
                                focus
                                    .expanded
                                    .update(|ex| {
                                        *ex = if ex.as_deref() == Some(id.as_str()) {
                                            None
                                        } else {
                                            Some(id.clone())
                                        };
                                    });
                                focus.selected.set(Some(id.clone()));
                            })
                            on_change=Callback::new(move |()| on_change_cb.run(id_changed.clone()))
                            on_saved=on_saved_cb
                            on_warned=on_warned_cb
                            accounts=accounts
                            balance=balance
                            context_year=Signal::derive(move || window.with(DisplayWindow::year))
                        />
                    }
                        .into_any()
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
                                let failed = r.failed;
                                view! {
                                    <div class=style::sentinel data-testid="register-sentinel" />
                                    <button
                                        class=style::load_more
                                        data-testid="load-more"
                                        disabled=loading
                                        on:click=move |_| on_load_more.run(LoadTrigger::Explicit)
                                    >
                                        {if loading {
                                            "loading\u{2026}".to_owned()
                                        } else if failed {
                                            format!("retry \u{00b7} {remaining} remaining")
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
