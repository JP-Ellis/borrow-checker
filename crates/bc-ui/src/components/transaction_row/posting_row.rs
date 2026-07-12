//! Posting lines and list for the expanded transaction detail.

use bc_ipc::AccountRef;
use jiff::civil::Date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::style;
use crate::components::ChipVariant;
use crate::components::account_picker::AccountPicker;
use crate::components::chip::Chip;
use crate::components::num::format_amount;
use crate::components::tag_picker::TagPicker;
use crate::components::transaction_row::edit_ctx::TxEditCtx;
use crate::components::transaction_row::editable::BalanceState;
use crate::components::transaction_row::editable::EditablePosting;
use crate::components::transaction_row::editable::EditableTransaction;
use crate::components::transaction_row::editable::derive_balance;
use crate::components::transaction_row::editable::parse_amount;
use crate::components::transaction_row::spread;
use crate::components::transaction_row::spread::SpreadDisplay;

/// Parses the transaction date string into a [`Date`], falling back to `2000-01-01`.
fn tx_date_from_working(working: &EditableTransaction) -> Date {
    working
        .date
        .trim()
        .parse::<Date>()
        .unwrap_or(Date::constant(2000, 1, 1))
}

/// Renders one always-editable posting row in the M-layout grid.
///
/// Every field is live regardless of edit mode. The row uses the four-column
/// M-grid: flow-bar | account+extras | amount | delete.
///
/// # Arguments
///
/// * `uid` - Stable identity of this posting in the buffer.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn PostingLine(
    /// Stable identity of this posting in the buffer.
    uid: u64,
) -> impl IntoView {
    let ctx = expect_context::<TxEditCtx>();
    let working = ctx.working;
    let ctx_accounts = ctx.accounts;
    let all_tags = ctx.all_tags;
    let currencies = ctx.currencies;
    let reset_epoch = ctx.reset_epoch;
    let matched = ctx.matched;

    // Resolve the live index from the stable uid on every access. Returns None
    // briefly if this row's posting was just removed.
    let index_of = move || working.with(|w| w.postings.iter().position(|p| p.uid == uid));

    // MARK: Account signals — synced into working via Effect.
    let selected_id = RwSignal::new(working.with_untracked(|w| {
        w.postings
            .iter()
            .find(|p| p.uid == uid)
            .map(|p| p.account_id.clone())
            .unwrap_or_default()
    }));
    let selected_name = RwSignal::new(working.with_untracked(|w| {
        w.postings
            .iter()
            .find(|p| p.uid == uid)
            .map(|p| p.account_name.clone())
            .unwrap_or_default()
    }));
    Effect::new(move |_| {
        let (id, name) = (selected_id.get(), selected_name.get());
        let Some(i) = working.with_untracked(|w| w.postings.iter().position(|p| p.uid == uid))
        else {
            return;
        };
        working.update(|w| {
            if let Some(p) = w.postings.get_mut(i)
                && (p.account_id != id || p.account_name != name)
            {
                p.account_id.clone_from(&id);
                p.account_name.clone_from(&name);
            }
        });
    });

    // MARK: Spread signals — string representations synced into working via Effect.
    let from_str = RwSignal::new(working.with_untracked(|w| {
        w.postings
            .iter()
            .find(|p| p.uid == uid)
            .and_then(|p| p.spread_from)
            .map(|d| d.to_string())
            .unwrap_or_default()
    }));
    let until_str = RwSignal::new(working.with_untracked(|w| {
        w.postings
            .iter()
            .find(|p| p.uid == uid)
            .and_then(|p| p.spread_until)
            .map(|d| d.to_string())
            .unwrap_or_default()
    }));
    Effect::new(move |_| {
        let (f, u) = (from_str.get(), until_str.get());
        let Some(i) = working.with_untracked(|w| w.postings.iter().position(|p| p.uid == uid))
        else {
            return;
        };
        working.update(|w| {
            if let Some(p) = w.postings.get_mut(i) {
                p.spread_from = f.trim().parse::<Date>().ok().or(if f.trim().is_empty() {
                    None
                } else {
                    p.spread_from
                });
                p.spread_until = u.trim().parse::<Date>().ok().or(if u.trim().is_empty() {
                    None
                } else {
                    p.spread_until
                });
            }
        });
    });

    // MARK: Note signal — synced into working via Effect.
    let note_sig = RwSignal::new(working.with_untracked(|w| {
        w.postings
            .iter()
            .find(|p| p.uid == uid)
            .map(|p| p.note.clone())
            .unwrap_or_default()
    }));
    Effect::new(move |_| {
        let n = note_sig.get();
        let Some(i) = working.with_untracked(|w| w.postings.iter().position(|p| p.uid == uid))
        else {
            return;
        };
        working.update(|w| {
            if let Some(p) = w.postings.get_mut(i)
                && p.note != n
            {
                p.note.clone_from(&n);
            }
        });
    });

    // MARK: Re-seed local inputs after an external reset (discard / escape).
    // Tracks only `reset_epoch`, never `working`, so in-progress typing is never
    // clobbered; each set is guarded so it cannot feed back into the forward
    // effects above.
    Effect::new(move |_| {
        reset_epoch.get();
        working.with_untracked(|w| {
            let Some(p) = w.postings.iter().find(|p| p.uid == uid) else {
                return;
            };
            if selected_id.get_untracked() != p.account_id {
                selected_id.set(p.account_id.clone());
            }
            if selected_name.get_untracked() != p.account_name {
                selected_name.set(p.account_name.clone());
            }
            let f = p.spread_from.map(|d| d.to_string()).unwrap_or_default();
            let u = p.spread_until.map(|d| d.to_string()).unwrap_or_default();
            if from_str.get_untracked() != f {
                from_str.set(f);
            }
            if until_str.get_untracked() != u {
                until_str.set(u);
            }
            if note_sig.get_untracked() != p.note {
                note_sig.set(p.note.clone());
            }
        });
    });

    // MARK: Local UI state.
    let show_note = RwSignal::new(false);
    let editing_spread = RwSignal::new(false);

    // MARK: Delete handler.
    let remove = move |_| {
        working.update(|w| {
            if let Some(i) = w.postings.iter().position(|p| p.uid == uid) {
                w.postings.remove(i);
            }
        });
    };

    // MARK: Flow direction — reactive class modifier on p_row.
    let row_class = move || {
        let amount_str = working.with(|w| {
            w.postings
                .iter()
                .find(|p| p.uid == uid)
                .map(|p| p.amount.clone())
                .unwrap_or_default()
        });
        let mut cls = style::p_row.to_owned();
        match parse_amount(&currencies.get(), amount_str.trim()) {
            Ok((v, _)) if v < Decimal::ZERO => {
                cls = format!("{} {}", cls, style::p_out);
            }
            Ok((v, _)) if v > Decimal::ZERO => {
                cls = format!("{} {}", cls, style::p_in);
            }
            _ => {}
        }
        cls
    };

    // MARK: Filter-match dimming — a leg whose original id is absent from the
    // active filter's matched set renders dimmed. A static open-time hint keyed
    // on the original posting id; newly added legs (no id) are never dimmed. An
    // empty matched set is treated as unfiltered (nothing dims) — the backend
    // never emits one, so a `Some(vec![])` would otherwise dim every leg.
    let is_dimmed = move || {
        matched.with_value(|m| {
            m.as_ref().filter(|ids| !ids.is_empty()).is_some_and(|ids| {
                working.with(|w| {
                    w.postings
                        .iter()
                        .find(|p| p.uid == uid)
                        .and_then(|p| p.id.as_ref())
                        .is_some_and(|pid| !ids.iter().any(|id| id == pid))
                })
            })
        })
    };

    // MARK: Ghost amount — detected when this posting is the sole inferred leg.
    let is_inferred = move || {
        let w = working.get();
        let is_elided = w
            .postings
            .iter()
            .find(|p| p.uid == uid)
            .is_some_and(EditablePosting::is_elided);
        is_elided
            && matches!(
                derive_balance(&w, &currencies.get()),
                BalanceState::Inferred { .. }
            )
    };

    let ghost_placeholder = move || {
        let w = working.get();
        let is_elided = w
            .postings
            .iter()
            .find(|p| p.uid == uid)
            .is_some_and(EditablePosting::is_elided);
        if !is_elided {
            return String::new();
        }
        match derive_balance(&w, &currencies.get()) {
            BalanceState::Inferred {
                remainder,
                currency,
            } => {
                let meta =
                    crate::components::num::meta::display_meta_for(&currency, &currencies.get());
                format_amount(&remainder, &meta)
            }
            BalanceState::Balanced
            | BalanceState::Unbalanced { .. }
            | BalanceState::Ambiguous
            | BalanceState::Invalid
            | BalanceState::Empty => String::new(),
        }
    };

    let amt_cell_class = move || {
        if is_inferred() {
            format!("{} {}", style::amt_cell, style::amt_cell_ghost)
        } else {
            style::amt_cell.to_owned()
        }
    };

    // MARK: Amount update handler.
    let set_amount = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        let Some(i) = index_of() else { return };
        working.update(|w| {
            if let Some(p) = w.postings.get_mut(i) {
                p.amount.clone_from(&v);
            }
        });
    };

    // MARK: Spread state helpers.
    let has_spread =
        move || !from_str.get().trim().is_empty() || !until_str.get().trim().is_empty();

    let spread_chip_text = move || {
        let tx_date_val = working.with(tx_date_from_working);
        working.with(|w| {
            let p = w.postings.iter().find(|p| p.uid == uid)?;
            let from = p.spread_from?;
            let until = p.spread_until?;
            let display = spread::spread_display(from, until, tx_date_val);
            Some(match display {
                SpreadDisplay::UntilOnly(u) => {
                    format!("\u{21b3} {}", spread::fmt_spread_date(u))
                }
                SpreadDisplay::FromUntil(f, u) => {
                    format!(
                        "{} \u{21b3} {}",
                        spread::fmt_spread_date(f),
                        spread::fmt_spread_date(u)
                    )
                }
            })
        })
    };

    let open_spread = move |_| {
        let tx_date_val = working.with(tx_date_from_working);
        let (f, u) = spread::default_spread(tx_date_val);
        from_str.set(f.to_string());
        until_str.set(u.to_string());
        editing_spread.set(true);
    };

    let clear_spread = Callback::new(move |()| {
        from_str.set(String::new());
        until_str.set(String::new());
        editing_spread.set(false);
    });

    let collapse_spread = move |_| editing_spread.set(false);
    let collapse_spread_key = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        if key == "Enter" || key == "Escape" {
            editing_spread.set(false);
        }
    };

    // Enter/Space opens the editor when the collapsed spread chip's label is
    // focused (the label carries role="button"/tabindex).
    let open_spread_key = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        if key == "Enter" || key == " " {
            ev.prevent_default();
            editing_spread.set(true);
        }
    };

    // MARK: Note visibility.
    let note_visible = move || show_note.get() || !note_sig.with(|n: &String| n.trim().is_empty());

    // MARK: "+ note" affordance, shared by the spread / no-spread tool rows.
    let add_note_btn = move || {
        (!note_visible()).then(|| {
            view! {
                <button class=style::tinytool on:click=move |_| show_note.set(true) type="button">
                    "+ note"
                </button>
            }
        })
    };

    view! {
        <div class=row_class data-testid="posting-row" data-dimmed=move || is_dimmed().to_string()>
            <div class=style::p_lead>
                <div class=style::p_flow></div>
            </div>

            <div class=style::p_acct_wrap>
                <div class=style::p_acct_row>
                    <AccountPicker
                        accounts=ctx_accounts.get_value()
                        selected_id=selected_id
                        selected_name=selected_name
                        on_pick=Callback::new(|_a: AccountRef| {})
                    />
                </div>

                <div class=style::p_extras>
                    <TagPicker
                        tags=Signal::derive(move || {
                            working
                                .with(|w| {
                                    w.postings
                                        .iter()
                                        .find(|p| p.uid == uid)
                                        .map(|p| p.tags.clone())
                                        .unwrap_or_default()
                                })
                        })
                        all_tags=Signal::derive(move || all_tags.get())
                        on_add=Callback::new(move |tag: String| {
                            working
                                .update(|w| {
                                    if let Some(p) = w.postings.iter_mut().find(|p| p.uid == uid)
                                        && !p.tags.contains(&tag)
                                    {
                                        p.tags.push(tag);
                                    }
                                });
                        })
                        on_remove=Callback::new(move |tag: String| {
                            working
                                .update(|w| {
                                    if let Some(p) = w.postings.iter_mut().find(|p| p.uid == uid) {
                                        p.tags.retain(|t| t != &tag);
                                    }
                                });
                        })
                        on_created=Callback::new(move |info: bc_ipc::TagInfo| {
                            all_tags.update(|v| v.push(info));
                        })
                        compact=true
                    />

                    {move || {
                        note_visible()
                            .then(|| {
                                view! {
                                    <input
                                        class=format!("{} {}", style::f, style::pnote_input)
                                        prop:value=move || note_sig.get()
                                        on:input=move |ev| note_sig.set(event_target_value(&ev))
                                        placeholder="add note…"
                                    />
                                }
                            })
                    }}

                    {move || {
                        (!has_spread() && !editing_spread.get())
                            .then(|| {
                                view! {
                                    <div class=style::posting_tools>
                                        {add_note_btn}
                                        <button
                                            class=style::tinytool
                                            on:click=open_spread
                                            type="button"
                                        >
                                            "\u{21b3} spread"
                                        </button>
                                    </div>
                                }
                            })
                    }}

                    {move || {
                        (has_spread() && !editing_spread.get())
                            .then(|| {
                                view! {
                                    <div class=style::posting_tools>{add_note_btn}</div>
                                    <Chip
                                        variant=ChipVariant::Accent
                                        on_remove=clear_spread
                                        remove_label="clear spread"
                                    >
                                        <span
                                            on:click=move |_| editing_spread.set(true)
                                            on:keydown=open_spread_key
                                            role="button"
                                            tabindex="0"
                                        >
                                            {spread_chip_text}
                                        </span>
                                    </Chip>
                                }
                            })
                    }}

                    {move || {
                        editing_spread
                            .get()
                            .then(|| {
                                view! {
                                    <div class=style::spread_edit>
                                        <span class=style::spread_edit_lbl>"from"</span>
                                        <input
                                            class=style::spread_edit_input
                                            prop:value=move || from_str.get()
                                            on:input=move |ev| from_str.set(event_target_value(&ev))
                                            on:keydown=collapse_spread_key
                                            on:blur=collapse_spread
                                            placeholder="YYYY-MM-DD"
                                            type="text"
                                        />
                                        <span class=style::spread_edit_lbl>"\u{21b3}"</span>
                                        <input
                                            class=style::spread_edit_input
                                            prop:value=move || until_str.get()
                                            on:input=move |ev| until_str.set(event_target_value(&ev))
                                            on:keydown=collapse_spread_key
                                            on:blur=collapse_spread
                                            placeholder="YYYY-MM-DD"
                                            type="text"
                                        />
                                        <button
                                            class=style::spread_edit_x
                                            on:click=move |_| clear_spread.run(())
                                            type="button"
                                            aria-label="clear spread"
                                        >
                                            "\u{00D7}"
                                        </button>
                                    </div>
                                }
                            })
                    }}
                </div>
            </div>

            <div class=amt_cell_class>
                <input
                    class=format!("{} {} {}", style::amt_input, style::f, style::f_num)
                    prop:value=move || {
                        working
                            .with(|w| {
                                w.postings
                                    .iter()
                                    .find(|p| p.uid == uid)
                                    .map(|p| p.amount.clone())
                                    .unwrap_or_default()
                            })
                    }
                    on:input=set_amount
                    placeholder=ghost_placeholder
                    data-testid="posting-amount"
                />
            </div>

            <button
                class=style::p_del_btn
                on:click=remove
                type="button"
                aria-label="remove posting"
            >
                "\u{00D7}"
            </button>
        </div>
    }
}

/// Renders the list of posting rows and the add-posting affordance.
///
/// Reads the shared working buffer from the [`TxEditCtx`] context.
#[component]
pub fn PostingsList() -> impl IntoView {
    let working = expect_context::<TxEditCtx>().working;
    let uids = move || working.with(|w| w.postings.iter().map(|p| p.uid).collect::<Vec<_>>());
    let add_leg = move |_| {
        working.update(|w| {
            w.push_blank_posting();
        });
    };
    view! {
        <div class=style::postings>
            <For each=uids key=|uid| *uid children=move |uid| view! { <PostingLine uid=uid /> } />
            <button
                class=format!("{} {}", style::addrow, style::addrow_btn)
                on:click=add_leg
                type="button"
            >
                "+ posting"
            </button>
        </div>
    }
}
