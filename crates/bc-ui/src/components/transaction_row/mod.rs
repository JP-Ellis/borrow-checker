//! Shared, posting-aware transaction row used by the accounts and budget pages.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::Posting;
use bc_ipc::Quote;
use bc_ipc::Transaction;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
use rust_decimal::Decimal;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::MetaEditor;
#[cfg(target_arch = "wasm32")]
use crate::components::tag_picker::TagPicker;
#[cfg(target_arch = "wasm32")]
use crate::components::tag_token::TagToken;
#[cfg(target_arch = "wasm32")]
use crate::components::toast::ToastKind;
#[cfg(target_arch = "wasm32")]
use crate::components::toast::use_toasts;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::Gate;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::action_error;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::already_reversed;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::delete_headline;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::delete_toast;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::delete_warnings;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::offers_both_deletes;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::reverse_headline;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::actions::reverse_warnings;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::edit_ctx::TxEditCtx;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::editable::BalanceState;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::editable::EditableTransaction;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::editable::derive_balance;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::posting_row::PostingsList;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::server_copy::KnownBases;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::server_copy::ServerCopy;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::server_copy::on_server_copy;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::view::NameStyle;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::view::RowView;
#[cfg(target_arch = "wasm32")]
use crate::components::transaction_row::view::Sign;

/// Editor-friendly working-buffer model for the editable transaction view.
///
/// Contains pure data structures ([`editable::EditableTransaction`] and
/// [`editable::EditablePosting`]) for representing transactions and postings
/// in the process of being edited. These structures use strings for all
/// numeric/date fields to represent parse-in-progress values.
pub mod editable;

/// Pure timestamp de-duplication for the audit trail display.
///
/// Collapses consecutive entries sharing the same instant under one time label
/// so a run of changes made together renders cleanly in the gutter.
pub mod audit;

/// Shared edit context (mode, working buffer, accounts) for the detail view.
#[cfg(target_arch = "wasm32")]
pub mod edit_ctx;

/// Inert, register-aligned read row for a single posting in the expanded detail.
#[cfg(target_arch = "wasm32")]
pub mod posting_row;

/// The cost chip on a posting row and its inline editor.
#[cfg(target_arch = "wasm32")]
pub mod cost_chip;

/// Pure helpers for rendering and seeding per-posting accrual spreads.
pub mod spread;

/// Pure view model for the collapsed row's header.
pub mod view;

/// How the open editor answers a refetched server copy.
pub mod server_copy;

/// Pure helpers for the cost chip: buffer/[`bc_ipc::Cost`] conversion and
/// chip/quote text.
pub mod cost;

/// Pure text and decisions for the detail's delete and reverse gates.
#[cfg(target_arch = "wasm32")]
mod actions;

// MARK: Pure display helpers

/// Width of a list's date column: `MM-DD` when `short`, else `YYYY-MM-DD`.
///
/// Lists set it as `--bc-date-col` so the header and every row share one track.
#[cfg(target_arch = "wasm32")]
#[must_use]
#[inline]
pub const fn date_col_width(short: bool) -> &'static str {
    if short { "44px" } else { "80px" }
}

/// Returns the first ASCII letter of `payee` as uppercase, or `'?'` if none.
///
/// Used for the payee avatar circle in transaction rows.
///
/// # Arguments
///
/// * `payee` - The payee string to extract an initial from.
///
/// # Returns
///
/// The first ASCII alphabetic character, uppercased, or `'?'` when none exists.
#[must_use]
#[inline]
pub fn payee_initial(payee: &str) -> char {
    payee
        .chars()
        .find(char::is_ascii_alphabetic)
        .map_or('?', |c| c.to_ascii_uppercase())
}

/// A row's tags split for the payee cell: the one shown as a chip and the
/// rest, which collapse into a `+N` chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSummary {
    /// The tag shown in full, if the row has any.
    pub first: Option<String>,
    /// Every tag after the first.
    pub rest: Vec<String>,
}

/// Splits `tags` into the first and the rest.
#[must_use]
pub fn summarise_tags(tags: &[String]) -> TagSummary {
    let mut iter = tags.iter().cloned();
    TagSummary {
        first: iter.next(),
        rest: iter.collect(),
    }
}

/// Formats a [`jiff::civil::Date`] for display as an ISO date.
///
/// # Arguments
///
/// * `date` - The civil date to format.
/// * `context_year` - The year the surrounding view already makes obvious.
///
/// # Returns
///
/// `MM-DD` when `date` falls in `context_year`, otherwise `YYYY-MM-DD`.
#[must_use]
#[inline]
pub fn format_date_display(date: jiff::civil::Date, context_year: Option<i16>) -> String {
    if context_year == Some(date.year()) {
        format!("{:02}-{:02}", date.month(), date.day())
    } else {
        date.to_string()
    }
}

#[cfg(target_arch = "wasm32")]
import_style!(pub(crate) style, "row.module.scss");

/// Determines which postings are focal and how the headline amount is derived.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RowPerspective {
    /// Accounts page: focal postings are those on any of `account_ids`;
    /// headline is their net sum.
    Account {
        /// The account in view, plus its descendants when the page rolls them
        /// up.
        account_ids: Vec<String>,
    },
    /// Budget page: focal postings are those the row counted; the headline is
    /// core's valued sum of them.
    Budget {
        /// IDs of the postings the row counted.
        counted: Vec<String>,
        /// Their valued sum in the row's commodity, when core could value it.
        contribution: Option<Amount>,
    },
    /// Fallback: headline is the one-sided sum of positive postings.
    Global,
}

/// Returns the focal postings within `tx`: those on any of `account_ids`.
///
/// # Arguments
///
/// * `tx` - The transaction to search.
/// * `account_ids` - The account IDs to match against posting accounts.
///
/// # Returns
///
/// An iterator over postings whose account ID is in `account_ids`.
pub fn focal_on_accounts<'a>(
    tx: &'a Transaction,
    account_ids: &'a [String],
) -> impl Iterator<Item = &'a Posting> {
    tx.postings
        .iter()
        .filter(move |p| account_ids.contains(&p.account.id))
}

/// Returns the account names of `tx`'s non-focal postings, the input to the
/// Category column.
///
/// # Arguments
///
/// * `tx` - The transaction.
/// * `perspective` - The row's perspective; [`RowPerspective::Global`] treats
///   every posting as a counterpart.
#[must_use]
pub fn counterpart_names<'a>(tx: &'a Transaction, perspective: &RowPerspective) -> Vec<&'a str> {
    let is_focal = |p: &Posting| match perspective {
        RowPerspective::Account { account_ids } => account_ids.contains(&p.account.id),
        RowPerspective::Budget { counted, .. } => counted.contains(&p.id),
        RowPerspective::Global => false,
    };
    tx.postings
        .iter()
        .filter(|p| !is_focal(p))
        .map(|p| p.account.name.as_str())
        .collect()
}

/// Computes the headline [`Amount`] for `tx` under `perspective`.
///
/// Returns an `Amount` with an empty currency code (rendered as `—`) when no
/// focal posting carries a concrete amount. Under `Budget` the result is empty
/// when core has no contribution.
///
/// # Arguments
///
/// * `tx` - The transaction to compute a headline for.
/// * `perspective` - Determines which postings are focal and how the amount is derived.
///
/// # Returns
///
/// The headline [`Amount`] for the given perspective.
#[must_use]
pub fn headline_amount(tx: &Transaction, perspective: &RowPerspective) -> Amount {
    match perspective {
        RowPerspective::Account { account_ids } => {
            let mut total = Decimal::ZERO;
            let mut currency = String::new();
            let mut any = false;
            for p in focal_on_accounts(tx, account_ids) {
                let Some(a) = p.amount.display_amount() else {
                    continue;
                };
                if currency.is_empty() {
                    currency.clone_from(&a.currency_code);
                }
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "same-commodity focal sum within one transaction"
                )]
                {
                    total += a.value;
                }
                any = true;
            }
            if any {
                Amount::new(total, currency)
            } else {
                Amount::new(Decimal::ZERO, "")
            }
        }
        RowPerspective::Budget { contribution, .. } => contribution
            .clone()
            .unwrap_or_else(|| Amount::new(Decimal::ZERO, "")),
        RowPerspective::Global => sum_focal(
            tx.postings
                .iter()
                .filter_map(|p| p.amount.stored())
                .filter(|a| a.value > Decimal::ZERO),
        ),
    }
}

/// The price annotation to show under the headline amount, if the headline
/// comes from exactly one leg and that leg carries one.
///
/// `Account` looks at the focal legs, `Global` at the positive stored legs
/// (the same legs `headline_amount` sums); `Budget` carries core's sum and
/// has no single price.
///
/// # Arguments
///
/// * `tx` - The transaction.
/// * `perspective` - The row's perspective.
///
/// # Returns
///
/// The one leg's price, or `None`.
#[must_use]
pub fn headline_price(tx: &Transaction, perspective: &RowPerspective) -> Option<Quote> {
    let mut legs = match perspective {
        RowPerspective::Account { account_ids } => focal_on_accounts(tx, account_ids)
            .filter(|p| p.amount.display_amount().is_some())
            .collect::<Vec<_>>()
            .into_iter(),
        RowPerspective::Global => tx
            .postings
            .iter()
            .filter(|p| p.amount.stored().is_some_and(|a| a.value > Decimal::ZERO))
            .collect::<Vec<_>>()
            .into_iter(),
        RowPerspective::Budget { .. } => return None,
    };
    let only = legs.next()?;
    if legs.next().is_some() {
        return None;
    }
    only.price.clone()
}

/// Sums a sequence of amounts, taking the currency from the first one.
///
/// Returns an [`Amount`] with zero value and empty currency code when the
/// iterator is empty.
fn sum_focal<'a>(mut amounts: impl Iterator<Item = &'a Amount>) -> Amount {
    let Some(first) = amounts.next() else {
        return Amount::new(Decimal::ZERO, "");
    };
    let currency = first.currency_code.clone();
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "summing amounts of the same commodity within a transaction; overflow not reachable in practice"
    )]
    let total = amounts.fold(first.value, |acc, a| acc + a.value);
    Amount::new(total, currency)
}

/// Renders the Category column cell.
///
/// A label too wide for the cell is cut with an ellipsis; the full label is
/// the cell's tooltip. *split transaction* renders in muted italic.
///
/// # Arguments
///
/// * `label` - The expansion string from [`crate::label::category_label`] (e.g.
///   `"Expenses :: {Groceries, Healthcare}"` or `"—"`), as a signal that
///   follows the row's transaction and perspective.
#[cfg(target_arch = "wasm32")]
#[component]
fn CategoryCell(
    /// Computed category label — either an account name, a shell expansion, or `"—"`.
    #[prop(into)]
    label: Signal<String>,
) -> impl IntoView {
    let class = move || {
        if label.with(|l| l == crate::label::SPLIT_LABEL) {
            format!("{} {}", style::category, style::category_split)
        } else {
            style::category.to_owned()
        }
    };
    view! {
        <span class=class title=label>
            {label}
        </span>
    }
}

/// A single register row, collapsed, optionally expanded to reveal the detail panel.
///
/// Renders date, payee avatar, name (payee or dim description), flag/unreconciled
/// glyphs, inline and mobile tags, category cell, headline amount with split and
/// unbalanced pills, and a chevron. Expanding reveals the editable detail panel.
///
/// Self-managed expansion is used when `expanded` and `on_toggle` are `None`.
///
/// # Arguments
///
/// * `tx` - The transaction to render; every header value follows it.
/// * `perspective` - Determines which postings are focal and how amounts are
///   derived; every header value follows it.
/// * `matched_postings` - Ids of the filter-matched legs, read once when the
///   detail opens.
/// * `selected` - Whether this row has keyboard focus.
/// * `expanded` - Optional external signal controlling expansion state.
/// * `on_toggle` - Optional callback called when the row is toggled.
/// * `on_change` - Optional callback called when the transaction is mutated.
/// * `context_year` - The year the view's period makes obvious; dates in it
///   drop their year.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn TransactionRow(
    /// The transaction to render.
    #[prop(into)]
    tx: Signal<Transaction>,
    /// Determines which postings are focal and how the headline amount is derived.
    #[prop(into)]
    perspective: Signal<RowPerspective>,
    /// Whether this row is keyboard-selected.
    #[prop(optional, into)]
    selected: Signal<bool>,
    /// Whether the detail panel is expanded (parent-controlled).
    #[prop(optional)]
    expanded: Option<Signal<bool>>,
    /// Called to toggle expansion (parent-controlled).
    #[prop(optional)]
    on_toggle: Option<Callback<()>>,
    /// Called when the transaction is mutated; consumed by the expanded detail view.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
    /// Called with the saved date after a successful edit; forwarded to the detail.
    #[prop(optional)]
    on_saved: Option<Callback<jiff::civil::Date>>,
    /// All selectable accounts for the recategorise picker in the detail view.
    #[prop(optional)]
    accounts: Vec<AccountRef>,
    /// All known tags for the transaction/posting tag pickers in the detail
    /// view; when empty the detail fetches them over IPC instead.
    #[prop(optional)]
    all_tags: Vec<bc_ipc::TagInfo>,
    /// Ids of legs that matched the posting-scoped filter predicates; `None`
    /// when the register is unfiltered. Forwarded to the expanded detail, where
    /// non-matching legs are dimmed as an open-time hint.
    #[prop(optional, into)]
    matched_postings: MaybeProp<Vec<String>>,
    /// Running balance for the register's balance column; `None` omits the
    /// column (budget and global perspectives).
    #[prop(optional)]
    balance: Option<Signal<Option<crate::components::balance_cell::BalanceCell>>>,
    /// The year the view's period makes obvious; `None` shows every date in
    /// full.
    #[prop(into, optional)]
    context_year: MaybeProp<i16>,
) -> impl IntoView {
    let local_expanded = RwSignal::new(false);
    let expanded: Signal<bool> = expanded.unwrap_or_else(|| local_expanded.into());
    let toggle = move || match on_toggle {
        Some(cb) => cb.run(()),
        None => local_expanded.update(|e| *e = !*e),
    };

    let row_view = Memo::new(move |_| tx.with(|t| perspective.with(|p| RowView::new(t, p))));
    let currencies = crate::currency_ctx::use_currency_store();

    let amount_str = move || {
        row_view.with(|v| {
            if v.amount.currency_code.is_empty() {
                "\u{2014}".to_owned()
            } else {
                let meta = crate::components::num::meta::display_meta_for(
                    &v.amount.currency_code,
                    &currencies.get(),
                );
                crate::components::num::format_amount(&v.amount.value, &meta)
            }
        })
    };
    let price_str = move || {
        row_view.with(|v| {
            v.price.as_ref().map(|q| {
                let meta = crate::components::num::meta::display_meta_for(
                    &q.amount().currency_code,
                    &currencies.get(),
                );
                crate::components::transaction_row::cost::quote_text(q, |a| {
                    crate::components::num::format_amount(&a.value, &meta)
                })
            })
        })
    };
    let amount_class = move || {
        let sign = match row_view.with(|v| v.sign) {
            Sign::Positive => style::amt_pos,
            Sign::Negative => style::amt_neg,
            Sign::Zero => style::amt_neu,
        };
        format!("{} {}", style::amount, sign)
    };
    let date = move || row_view.with(|v| format_date_display(v.date, context_year.get()));
    let name_class = move || match row_view.with(|v| v.name_style) {
        NameStyle::Payee => style::payee.to_owned(),
        NameStyle::Description => format!("{} {}", style::payee, style::name_dim),
    };
    let tag_tokens = move || {
        row_view
            .with(|v| v.tags.clone())
            .into_iter()
            .map(|t| view! { <TagToken label=t /> })
            .collect::<Vec<_>>()
    };

    let inline_tags = move || {
        let all = row_view.with(|v| v.tags.clone());
        let TagSummary { first, rest } = summarise_tags(&all);
        let count = (!all.is_empty()).then(|| {
            view! {
                <span class=style::tag_count title=all.join(", ")>
                    {format!("+{}", all.len())}
                </span>
            }
        });
        let more = (!rest.is_empty()).then(|| {
            view! {
                <span class=style::tag_more title=all.join(", ")>
                    {format!("+{}", rest.len())}
                </span>
            }
        });
        view! {
            {first
                .map(|t| {
                    view! {
                        <span class=style::tag_first>
                            <TagToken label=t />
                        </span>
                    }
                })}
            {more}
            {count}
        }
    };

    let toggle_click = toggle;
    let toggle_key = toggle;

    let has_balance = balance.is_some();
    // A mounted row is keyed on this id, so it never changes.
    let tx_id = tx.with_untracked(|t| t.id.clone());

    view! {
        <div
            class=move || {
                let mut cls = vec![style::row];
                if selected.get() {
                    cls.push(style::row_selected);
                }
                if expanded.get() {
                    cls.push(style::row_expanded);
                }
                if has_balance {
                    cls.push(style::row_with_balance);
                }
                cls.join(" ")
            }
            on:click=move |_| toggle_click()
            on:keydown=move |e: web_sys::KeyboardEvent| {
                if e.key() == " " || e.key() == "Enter" {
                    toggle_key();
                    e.prevent_default();
                }
            }
            role="button"
            data-tx-id=tx_id
            tabindex="0"
            aria-expanded=move || expanded.get().to_string()
        >
            <span class=style::date>{date}</span>
            <div class=style::payee_cell>
                <span class=style::avatar aria-hidden="true">
                    {move || row_view.with(|v| v.initial.to_string())}
                </span>
                <span class=name_class>{move || row_view.with(|v| v.name.clone())}</span>
                {move || {
                    row_view
                        .with(|v| v.flagged)
                        .then(|| {
                            view! {
                                <span class=style::glyph_flag aria-label="flagged" title="flagged">
                                    "\u{2691}"
                                </span>
                            }
                        })
                }}
                {move || {
                    row_view
                        .with(|v| v.unreconciled)
                        .then(|| {
                            view! {
                                <span
                                    class=style::glyph_unrec
                                    aria-label="unreconciled"
                                    title="unreconciled"
                                >
                                    "\u{25CB}"
                                </span>
                            }
                        })
                }}
                <div class=style::inline_tags>{inline_tags}</div>
            </div>
            <div class=style::tags_cell>{tag_tokens}</div>
            <CategoryCell label=Signal::derive(move || row_view.with(|v| v.category.clone())) />
            <span class=amount_class>
                <span class=style::amt_stack>
                    <span>{amount_str}</span>
                    {move || {
                        let price = price_str();
                        let (split, unbalanced) = row_view.with(|v| (v.split, v.unbalanced));
                        (price.is_some() || split.is_some() || unbalanced)
                            .then(|| {
                                view! {
                                    <span class=style::amt_sub>
                                        {price}
                                        {split
                                            .map(|count| {
                                                view! {
                                                    <span class=style::pill_split>
                                                        "split \u{00b7} " {count}
                                                    </span>
                                                }
                                            })}
                                        {unbalanced
                                            .then(|| {
                                                view! {
                                                    <span class=style::pill_unbalanced>
                                                        "\u{26A0} unbalanced"
                                                    </span>
                                                }
                                            })}
                                    </span>
                                }
                            })
                    }}
                </span>
            </span>
            {balance
                .map(|cell| {
                    view! {
                        <span class=style::balance_col>
                            <crate::components::balance_cell::BalanceCellView cell=cell />
                        </span>
                    }
                })}
            <span class=style::chevron aria-hidden="true">
                {move || if expanded.get() { "\u{2193}" } else { "\u{203A}" }}
            </span>
        </div>
        {
            let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));
            let on_saved_cb = on_saved.unwrap_or_else(|| Callback::new(|_| {}));
            let accounts = StoredValue::new(accounts);
            let all_tags = StoredValue::new(all_tags);
            move || {
                expanded
                    .get()
                    .then(|| {
                        view! {
                            <TransactionDetail
                                server=tx
                                on_change=on_change_cb
                                on_saved=on_saved_cb
                                accounts=accounts.get_value()
                                all_tags=all_tags.get_value()
                                matched_postings=matched_postings.get_untracked()
                            />
                        }
                    })
            }
        }
    }
}

/// Inline, always-editable detail panel shown below an expanded [`TransactionRow`].
///
/// Provides a [`TxEditCtx`] and renders, top to bottom: the editable
/// [`PostingsList`], a quiet balance line, a statement-style meta bar (date,
/// clickable reconciliation pill, transaction tags, note), a raw TOML view of
/// the remaining transaction fields, the optional audit log, and a dirty-gated
/// save bar. Saving wires to [`bc_ipc::client::edit_transaction`] (plus
/// [`bc_ipc::client::set_reconciliation`] when the status changed).
///
/// # Arguments
///
/// * `server` - The latest server copy of the transaction. The editor seeds
///   from it at mount. A later copy the editor has not held is adopted under a
///   clean draft, and flags the base as stale under a dirty one; a copy that
///   arrives while a save is in flight, equals any base held since opening, or
///   carries another transaction's id is ignored.
/// * `on_change` - Optional callback run after a successful save; defaults to a
///   no-op when `None`.
/// * `accounts` - All selectable accounts for the recategorise picker; an empty
///   list degrades to free-text-only pickers.
/// * `all_tags` - Known tags to seed the pickers; empty falls back to the IPC
///   fetch.
#[cfg(target_arch = "wasm32")]
#[component]
fn TransactionDetail(
    /// The latest server copy of the transaction: the register's row, or a
    /// fixed value. Seeds the editor at mount; later copies are adopted or
    /// flagged, except one that arrives during a save, equals any base held
    /// since opening, or belongs to another transaction.
    server: Signal<Transaction>,
    /// Called after a successful mutation; defaults to a no-op when `None`.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
    /// Called with the saved date after a successful edit; defaults to a no-op.
    #[prop(optional)]
    on_saved: Option<Callback<jiff::civil::Date>>,
    /// All selectable accounts for the recategorise picker.
    #[prop(optional)]
    accounts: Vec<AccountRef>,
    /// Known tags to seed the pickers; empty falls back to the IPC fetch.
    #[prop(optional)]
    all_tags: Vec<bc_ipc::TagInfo>,
    /// Original ids of the filter-matched legs, or `None` when unfiltered;
    /// non-matching legs render dimmed as an open-time hint.
    #[prop(optional_no_strip)]
    matched_postings: Option<Vec<String>>,
) -> impl IntoView {
    let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));
    let on_saved_cb = on_saved.unwrap_or_else(|| Callback::new(|_| {}));
    let tx = server.get_untracked();
    let known_bases = StoredValue::new(KnownBases::new(tx.clone()));
    let editable = EditableTransaction::from(&tx);
    let ctx = TxEditCtx::new(editable, accounts, matched_postings);
    provide_context(ctx.clone());
    let editor_accounts = ctx.accounts;

    if !all_tags.is_empty() {
        ctx.all_tags.set(all_tags);
    }

    #[expect(
        clippy::shadow_unrelated,
        reason = "prop vec consumed into ctx; name reused for the context signal"
    )]
    let all_tags = ctx.all_tags;
    let _tags_resource = LocalResource::new(move || async move {
        if let Ok(list) = bc_ipc::client::list_tags().await {
            all_tags.set(list);
        }
    });

    let currencies = ctx.currencies;
    let shared_currencies = crate::currency_ctx::use_currency_store();
    Effect::new(move |_| {
        currencies.set(shared_currencies.get());
    });

    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let saving = RwSignal::new(false);
    // Set when `original` no longer matches the stored transaction, so a save
    // built on it would conflict. Only "Discard and reload" clears it.
    let stale_base = RwSignal::new(false);

    let gate = RwSignal::new(Gate::Closed);
    let provenance: RwSignal<Option<Result<bc_ipc::TransactionProvenance, ()>>> =
        RwSignal::new(None);
    let reversed_before = RwSignal::new(false);
    let acting = RwSignal::new(false);
    let toasts = use_toasts();

    let show_audit = RwSignal::new(false);
    let audit_version = RwSignal::new(0_u32);
    let tx_id_audit = ctx.working.with(|w| w.id.clone());
    let audit_resource = LocalResource::new(move || {
        audit_version.get();
        let id = tx_id_audit.clone();
        async move {
            if show_audit.get_untracked() {
                bc_ipc::client::get_transaction_audit(&id).await
            } else {
                Ok(vec![])
            }
        }
    });

    let f_date = RwSignal::new(ctx.working.with(|w| w.date.clone()));
    let f_desc = RwSignal::new(ctx.working.with(|w| w.description.clone()));

    {
        let working = ctx.working;
        Effect::new(move |_| {
            let (date, desc) = (f_date.get(), f_desc.get());
            working.update(|w| {
                w.date = date;
                w.description = desc;
            });
        });
    }

    let working = ctx.working;
    let original = ctx.original;

    let balance_state =
        Signal::derive(move || working.with(|w| derive_balance(w, &currencies.get())));
    // Unbalanced transactions are saveable (flagged, not blocked) so partial,
    // iterative edits can be persisted. Only the genuinely unrepresentable states
    // block saving: Ambiguous (two-plus elided legs), Invalid (an amount does not
    // parse), and Empty (no amounts to record).
    let save_disabled = Signal::derive(move || {
        matches!(
            balance_state.get(),
            BalanceState::Ambiguous | BalanceState::Invalid | BalanceState::Empty
        )
    });

    let cycle_recon = move |_| {
        working.update(|w| {
            w.reconciliation = match w.reconciliation {
                bc_ipc::Reconciliation::Unreconciled => bc_ipc::Reconciliation::Flagged,
                bc_ipc::Reconciliation::Flagged => bc_ipc::Reconciliation::Reconciled,
                bc_ipc::Reconciliation::Reconciled | _ => bc_ipc::Reconciliation::Unreconciled,
            };
        });
    };

    let ctx_discard = ctx.clone();
    let discard = Callback::new(move |()| {
        // A save in flight owns `working`/`original` until its refetch
        // settles; discarding underneath it would leave the buffer at the
        // pre-save values while `original` moves on to the saved state,
        // inverting the save.
        if saving.get_untracked() {
            return;
        }
        ctx_discard.discard();
        original.with_value(|o| {
            f_date.set(o.date.clone());
            f_desc.set(o.description.clone());
        });
        error.set(None);
    });

    // Rebases the editor on a server copy: the copy becomes `original` and
    // replaces the draft. The copy is recorded first, so the refetch that
    // any caller triggers afterwards reads as known.
    let ctx_rebase = ctx.clone();
    let rebase = Callback::new(move |fresh: Transaction| {
        known_bases.update_value(|k| k.record(fresh.clone()));
        let fresh = EditableTransaction::from(&fresh);
        f_date.set(fresh.date.clone());
        f_desc.set(fresh.description.clone());
        original.set_value(fresh);
        ctx_rebase.discard();
        stale_base.set(false);
        error.set(None);
        audit_version.update(|v| *v = v.wrapping_add(1));
    });

    let ctx_reload = ctx.clone();
    let discard_and_reload = Callback::new(move |()| {
        // Same guard as `discard`: never race a save's own refetch.
        if saving.get_untracked() {
            return;
        }
        let id = ctx_reload.working.with_untracked(|w| w.id.clone());
        leptos::task::spawn_local(async move {
            let fetched = bc_ipc::client::get_transaction(&id).await;
            if working.is_disposed() {
                return;
            }
            match fetched {
                Ok(fresh) => {
                    rebase.run(fresh);
                    on_change_cb.run(());
                }
                Err(e) => error.set(Some(format!("Couldn't reload the transaction: {e}"))),
            }
        });
    });

    let ctx_save = ctx.clone();
    let save = Callback::new(move |()| {
        // A delete or reverse in flight acts on the saved transaction; saving
        // under it would change what a reversal negates.
        if saving.get_untracked()
            || acting.get_untracked()
            || stale_base.get_untracked()
            || !ctx_save.dirty()
            || save_disabled.get_untracked()
        {
            return;
        }
        let working_now = working.get_untracked();
        let saved_date = working_now.date.parse::<jiff::civil::Date>().ok();
        let edit = match working_now.to_edit_transaction(&currencies.get_untracked()) {
            Ok(d) => d,
            Err(e) => {
                error.set(Some(e.to_string()));
                return;
            }
        };
        let base = match original.with_value(|o| o.to_edit_transaction(&currencies.get_untracked()))
        {
            Ok(b) => b,
            Err(e) => {
                error.set(Some(e.to_string()));
                return;
            }
        };
        let recon_changed = original.with_value(|o| o.reconciliation) != working_now.reconciliation;
        let id = working_now.id.clone();
        let recon = working_now.reconciliation;
        // Snapshot of exactly what was submitted, so the refetch below can
        // tell whether the user kept editing during the round trip.
        let pre_save_snapshot = working_now;
        let ctx_task = ctx_save.clone();
        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::edit_transaction(&edit, &base).await {
                Ok(()) => {
                    if let Some(date) = saved_date {
                        on_saved_cb.try_run(date);
                    }
                    // A failed follow-up reconciliation change does not undo
                    // the edit; the refetch below still runs and picks up
                    // whatever the database actually holds.
                    let recon_error = if recon_changed {
                        bc_ipc::client::set_reconciliation(&id, recon).await.err()
                    } else {
                        None
                    };
                    // Refetch rather than reuse the working copy: a posting
                    // added this save still has `id: None` locally, and using
                    // it as the next save's base would mint a fresh posting ID
                    // server-side and false-conflict. Typing stays live
                    // during this round trip; the `pre_save_snapshot` check
                    // below keeps any keystroke. `saving` stays true so a
                    // second save or a discard cannot race the refetch.
                    let refetched = bc_ipc::client::get_transaction(&id).await;
                    // Escape or opening another row closes the editor, but
                    // not this task. The save stands; only the register is
                    // left to refresh.
                    if working.is_disposed() {
                        on_change_cb.try_run(());
                        return;
                    }
                    match refetched {
                        Ok(fresh) => {
                            known_bases.update_value(|k| k.record(fresh.clone()));
                            let fresh = EditableTransaction::from(&fresh);
                            original.set_value(fresh);
                            // The buffer only still equals what was submitted
                            // when nothing was typed during the edit/reconcile/
                            // refetch round trip; only then is it safe to
                            // replace it wholesale with the fresh copy. Edits
                            // typed meanwhile are kept and diffed against the
                            // fresh `original` instead of being overwritten.
                            if working.with_untracked(|w| w == &pre_save_snapshot) {
                                // Route the replacement through `discard`
                                // rather than `working.set` directly: it also
                                // bumps `reset_epoch`, which per-posting rows
                                // need to re-seed their cached account/spread
                                // signals. Without it, a row whose `uid`
                                // happens to persist across the swap (e.g. the
                                // server reordered postings, or a delete
                                // shifted the position-based `uid`s the fresh
                                // buffer assigns) would keep showing its
                                // pre-save account/spread text next to the
                                // fresh posting's live amount.
                                ctx_task.discard();
                                original.with_value(|o| {
                                    f_date.set(o.date.clone());
                                    f_desc.set(o.description.clone());
                                });
                            }
                            if recon_error.is_some() {
                                // The reconciliation change did not persist;
                                // reassert it instead of losing it, on top of
                                // the fresh state or of whatever the user kept
                                // editing.
                                working.update(|w| w.reconciliation = recon);
                            }
                            saving.set(false);
                            on_change_cb.run(());
                            audit_version.update(|v| *v = v.wrapping_add(1));
                            if let Some(e) = recon_error {
                                error.set(Some(friendly_save_error(&e)));
                            }
                        }
                        Err(e) => {
                            // The edit is stored, but `original` still holds
                            // the pre-save copy. Only a reload gives the next
                            // save a base it will not conflict with.
                            saving.set(false);
                            stale_base.set(true);
                            on_change_cb.run(());
                            audit_version.update(|v| *v = v.wrapping_add(1));
                            error.set(Some(format!(
                                "Saved, but couldn't refresh the transaction: {e}. \
                                 Reload to keep editing."
                            )));
                        }
                    }
                }
                Err(e) => {
                    saving.set(false);
                    stale_base.set(matches!(e, bc_ipc::BcError::Conflict(_)));
                    error.set(Some(friendly_save_error(&e)));
                }
            }
        });
    });

    // A newer server copy reaches the open editor through the register.
    // Copies this editor has held are its own echoes, and a copy of another
    // transaction is never its business; anything else is adopted under a
    // clean draft and flagged under a dirty one.
    let ctx_server = ctx.clone();
    Effect::new(move |_| {
        let copy = server.get();
        let busy = saving.get();
        let origin = known_bases.with_value(|k| k.origin(&copy));
        let dirty = untrack(|| ctx_server.dirty());
        match on_server_copy(busy, origin, dirty) {
            ServerCopy::Ignore => {}
            ServerCopy::Adopt => rebase.run(copy),
            // A base already flagged keeps its message: a failed refetch
            // after a save explains itself.
            ServerCopy::MarkStale => {
                if !stale_base.get_untracked() {
                    stale_base.set(true);
                    error.set(Some(CONFLICT_MESSAGE.to_owned()));
                }
            }
        }
    });

    let tx_id = ctx.working.with_untracked(|w| w.id.clone());

    let tx_id_open_delete = tx_id.clone();
    let open_delete = move |_| {
        gate.set(Gate::Delete);
        provenance.set(None);
        let id = tx_id_open_delete.clone();
        leptos::task::spawn_local(async move {
            let result = bc_ipc::client::transaction_provenance(&id)
                .await
                .map_err(|e| {
                    leptos::logging::error!("Couldn't read the import history of {id}: {e}");
                });
            provenance.try_set(Some(result));
        });
    };

    let tx_id_open_reverse = tx_id.clone();
    let open_reverse = move |_| {
        gate.set(Gate::Reverse);
        let id = tx_id_open_reverse.clone();
        leptos::task::spawn_local(async move {
            let reversed = match bc_ipc::client::get_transaction_audit(&id).await {
                Ok(entries) => already_reversed(&entries),
                Err(e) => {
                    leptos::logging::error!("Couldn't read the audit trail of {id}: {e}");
                    false
                }
            };
            reversed_before.try_set(reversed);
        });
    };

    let tx_id_delete = tx_id.clone();
    let confirm_delete = Callback::new(move |forget: bool| {
        if acting.get_untracked() || saving.get_untracked() {
            return;
        }
        acting.set(true);
        let id = tx_id_delete.clone();
        leptos::task::spawn_local(async move {
            match bc_ipc::client::delete_transaction(&id, forget).await {
                Ok(outcome) => {
                    toasts.push(ToastKind::Success, delete_toast(&outcome, &id), None);
                    gate.try_set(Gate::Closed);
                    acting.try_set(false);
                    // The register refetches and drops the selection, which
                    // unmounts this detail.
                    on_change_cb.try_run(());
                }
                Err(e) => {
                    toasts.push(ToastKind::Error, action_error("delete", &e), None);
                    acting.try_set(false);
                }
            }
        });
    });

    let tx_id_reverse = tx_id;
    let confirm_reverse = Callback::new(move |()| {
        if acting.get_untracked() || saving.get_untracked() {
            return;
        }
        acting.set(true);
        let id = tx_id_reverse.clone();
        leptos::task::spawn_local(async move {
            match bc_ipc::client::reverse_transaction(&id).await {
                Ok(_) => {
                    toasts.push(ToastKind::Success, "Reversal added.", None);
                    on_change_cb.try_run(());
                    audit_version.try_update(|v| *v = v.wrapping_add(1));
                    gate.try_set(Gate::Closed);
                }
                Err(e) => {
                    toasts.push(ToastKind::Error, action_error("reverse", &e), None);
                }
            }
            acting.try_set(false);
        });
    });
    let close_gate = Callback::new(move |()| gate.set(Gate::Closed));

    let is_reconciled = Signal::derive(move || {
        original.with_value(|o| o.reconciliation) == bc_ipc::Reconciliation::Reconciled
    });
    let ctx_dirty = ctx.clone();
    let is_dirty = Signal::derive(move || ctx_dirty.dirty());
    let mount_date = tx.date;
    let persisted_date = move || {
        original
            .with_value(|o| o.date.parse::<jiff::civil::Date>().ok())
            .unwrap_or(mount_date)
    };

    let detail_ref = NodeRef::<leptos::html::Div>::new();
    let on_key = move |e: web_sys::KeyboardEvent| {
        let key = e.key();
        let key = key.as_str();

        let typing_in_field = e
            .target()
            .and_then(|t| web_sys::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .is_some_and(|el| {
                let tag = el.tag_name();
                tag == "INPUT"
                    || tag == "TEXTAREA"
                    || web_sys::wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el)
                        .is_ok_and(|h| h.is_content_editable())
            });

        if typing_in_field {
            return;
        }

        if key == "Escape" && gate.get_untracked() != Gate::Closed {
            gate.set(Gate::Closed);
            e.prevent_default();
            // The register collapses the detail on Escape; this one only
            // closes the gate.
            e.stop_propagation();
        } else if key == "Escape" {
            if stale_base.get_untracked() {
                discard_and_reload.run(());
            } else {
                discard.run(());
            }
            e.prevent_default();
        } else if (e.meta_key() || e.ctrl_key()) && (key == "s" || key == "S") {
            save.run(());
            e.prevent_default();
        } else if key == "a" {
            show_audit.update(|v| *v = !*v);
            audit_version.update(|v| *v = v.wrapping_add(1));
            e.prevent_default();
        }
    };

    let ctx_bar = ctx.clone();

    view! {
        <div class=style::detail node_ref=detail_ref on:keydown=on_key tabindex="-1">
            <PostingsList />

            {move || {
                let render = |amounts: &[bc_ipc::Amount]| {
                    let known = currencies.get();
                    amounts
                        .iter()
                        .map(|a| {
                            let meta = crate::components::num::meta::display_meta_for(
                                &a.currency_code,
                                &known,
                            );
                            crate::components::num::format_amount(&a.value, &meta)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let (extra, text) = match balance_state.get() {
                    BalanceState::Balanced => (style::balance_ok, "balances".to_owned()),
                    BalanceState::Inferred { remainder } => {
                        (
                            style::balance_ok,
                            format!("balances \u{2014} auto {}", render(&remainder)),
                        )
                    }
                    BalanceState::Empty => (style::balance_ok, "no amounts yet".to_owned()),
                    BalanceState::Unbalanced { delta } => {
                        (
                            style::balance_bad,
                            format!("unbalanced \u{2014} \u{03A3} = {}", render(&delta)),
                        )
                    }
                    BalanceState::Ambiguous => {
                        (style::balance_bad, "more than one blank amount".to_owned())
                    }
                    BalanceState::Invalid => {
                        (style::balance_bad, "an amount does not parse".to_owned())
                    }
                };
                // Each commodity is rendered with its own display metadata and
                // joined; nothing is summed across commodities.
                view! {
                    <div class=style::balance>
                        <span class=format!("{} {}", style::bal_text, extra)>{text}</span>
                    </div>
                }
            }}

            <div class=style::metamix>
                <div class=style::mm_main>
                    <div class=style::mm_fields>
                        <span class=style::metamix_lbl>"Description"</span>
                        <div class=style::mm_val>
                            <input
                                class=format!("{} {}", style::f, style::textfield)
                                prop:value=move || f_desc.get()
                                on:input=move |ev| f_desc.set(event_target_value(&ev))
                                placeholder="description"
                            />
                        </div>
                        <span class=style::metamix_lbl>"Status"</span>
                        <div class=style::mm_val>
                            <span
                                class=move || {
                                    let variant = working
                                        .with(|w| match w.reconciliation {
                                            bc_ipc::Reconciliation::Flagged => style::status_flagged,
                                            bc_ipc::Reconciliation::Reconciled => style::status_ok,
                                            bc_ipc::Reconciliation::Unreconciled | _ => {
                                                style::status_unrec
                                            }
                                        });
                                    format!("{} {}", style::status_pill, variant)
                                }
                                on:click=cycle_recon
                                role="button"
                                tabindex="0"
                                data-testid="status-pill"
                            >
                                <span class=style::status_dot></span>
                                {move || working.with(|w| w.reconciliation.label().to_owned())}
                            </span>
                        </div>
                        <span class=style::metamix_lbl>"Tags"</span>
                        <div class=style::mm_val>
                            <TagPicker
                                tags=Signal::derive(move || working.with(|w| w.tags.clone()))
                                all_tags=Signal::derive(move || all_tags.get())
                                on_add=Callback::new(move |tag: bc_ipc::TagInfo| {
                                    working
                                        .update(|w| {
                                            if !w.tags.iter().any(|t| t.id == tag.id) {
                                                w.tags.push(tag);
                                            }
                                        });
                                })
                                on_remove=Callback::new(move |tag: bc_ipc::TagInfo| {
                                    working.update(|w| w.tags.retain(|t| t.id != tag.id));
                                })
                                on_created=Callback::new(move |info: bc_ipc::TagInfo| {
                                    all_tags.update(|v| v.push(info));
                                })
                                compact=true
                            />
                        </div>
                        <span class=style::metamix_lbl>"Fields"</span>
                        <div class=style::mm_val>
                            <MetaEditor
                                rows=Signal::derive(move || working.with(|w| w.metadata.clone()))
                                on_change=Callback::new(move |rows| {
                                    working.update(|w| w.metadata = rows);
                                })
                                accounts=editor_accounts.get_value()
                                default_commodity=Signal::derive(move || {
                                    working.with(EditableTransaction::default_currency)
                                })
                            />
                        </div>
                    </div>
                    <div class=style::mm_dates>
                        <span class=style::metamix_lbl>"Date"</span>
                        <input
                            class=format!("{} {}", style::f, style::f_num)
                            prop:value=move || f_date.get()
                            on:input=move |ev| f_date.set(event_target_value(&ev))
                            placeholder="YYYY-MM-DD"
                        />
                    </div>
                </div>
            </div>

            {move || {
                show_audit
                    .get()
                    .then(|| {
                        view! {
                            <div class=style::audit_hdr>"Audit"</div>
                            {move || match audit_resource.get() {
                                Some(Ok(entries)) => {
                                    let rows = audit::audit_rows(&entries);
                                    view! {
                                        <div class=style::audit_list>
                                            {rows
                                                .into_iter()
                                                .map(|r| {
                                                    view! {
                                                        <div class=style::audit_row>
                                                            <span class=style::audit_time>
                                                                {r.time.unwrap_or_default()}
                                                            </span>
                                                            <span class=style::audit_kind>{r.kind}</span>
                                                            <span class=style::audit_msg>{r.message}</span>
                                                        </div>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </div>
                                    }
                                        .into_any()
                                }
                                Some(Err(err)) => {
                                    view! { <div class=style::diag_error>{err.to_string()}</div> }
                                        .into_any()
                                }
                                None => {
                                    view! {
                                        <div class=style::audit_loading>"loading audit…"</div>
                                    }
                                        .into_any()
                                }
                            }}
                        }
                    })
            }}

            <div class=style::actions_row>
                {move || match gate.get() {
                    Gate::Closed => {
                        view! {
                            <button
                                class=style::action_btn
                                type="button"
                                disabled=move || acting.get() || saving.get()
                                on:click=open_reverse.clone()
                            >
                                "Reverse"
                            </button>
                            <button
                                class=style::action_btn
                                type="button"
                                disabled=move || acting.get() || saving.get()
                                on:click=open_delete.clone()
                            >
                                "Delete"
                            </button>
                        }
                            .into_any()
                    }
                    Gate::Delete => {
                        view! {
                            <DeleteGate
                                provenance=provenance.read_only().into()
                                reconciled=is_reconciled
                                dirty=is_dirty
                                acting=acting.read_only().into()
                                on_confirm=confirm_delete
                                on_cancel=close_gate
                            />
                        }
                            .into_any()
                    }
                    Gate::Reverse => {
                        view! {
                            <ReverseGate
                                date=persisted_date()
                                already_reversed=reversed_before.read_only().into()
                                dirty=is_dirty
                                acting=acting.read_only().into()
                                on_confirm=confirm_reverse
                                on_cancel=close_gate
                            />
                        }
                            .into_any()
                    }
                }}
            </div>

            {move || {
                ctx_bar
                    .dirty()
                    .then(|| {
                        view! {
                            <div class=style::savebar>
                                <div class=style::savebar_note>
                                    {move || {
                                        error.get().unwrap_or_else(|| "unsaved changes".to_owned())
                                    }}
                                </div>
                                {move || {
                                    if stale_base.get() {
                                        view! {
                                            <button
                                                class=style::action_btn
                                                on:click=move |_| discard_and_reload.run(())
                                                type="button"
                                                aria-label="discard and reload"
                                            >
                                                "Discard and reload"
                                            </button>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <button
                                                class=style::action_btn
                                                disabled=move || saving.get()
                                                on:click=move |_| discard.run(())
                                                type="button"
                                                aria-label="discard changes"
                                            >
                                                "Discard"
                                            </button>
                                            <button
                                                class=style::action_btn
                                                disabled=move || save_disabled.get() || acting.get()
                                                on:click=move |_| save.run(())
                                                type="button"
                                                aria-label="save transaction"
                                            >
                                                "Save"
                                            </button>
                                        }
                                            .into_any()
                                    }
                                }}
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// Delete confirmation in the register detail.
///
/// Shows the import history headline and any warnings, then either the two
/// re-import choices or, for a hand entry, a single Delete. While the history
/// is loading the single button shows disabled.
#[cfg(target_arch = "wasm32")]
#[component]
fn DeleteGate(
    /// The import history lookup: `None` in flight, `Err(())` failed.
    provenance: Signal<Option<Result<bc_ipc::TransactionProvenance, ()>>>,
    /// Whether the saved transaction is reconciled.
    reconciled: Signal<bool>,
    /// Whether the detail holds unsaved edits.
    dirty: Signal<bool>,
    /// Whether a delete or reverse is in flight.
    acting: Signal<bool>,
    /// Runs the delete; `true` forgets the statement rows so a re-import
    /// recreates the transaction.
    on_confirm: Callback<bool>,
    /// Closes the gate.
    on_cancel: Callback<()>,
) -> impl IntoView {
    let primary = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        if let Some(button) = primary.get() {
            drop(button.focus());
        }
    });
    view! {
        <span>{move || provenance.with(|p| delete_headline(p.as_ref()))}</span>
        {move || {
            delete_warnings(reconciled.get(), dirty.get())
                .into_iter()
                .map(|w| view! { <span class=style::gate_warning>{w}</span> })
                .collect_view()
        }}
        {move || {
            if provenance.with(|p| offers_both_deletes(p.as_ref())) {
                view! {
                    <button
                        class=style::action_btn
                        type="button"
                        node_ref=primary
                        disabled=move || acting.get()
                        on:click=move |_| on_confirm.run(false)
                    >
                        "Delete, skip on re-import"
                    </button>
                    <button
                        class=style::action_btn
                        type="button"
                        disabled=move || acting.get()
                        on:click=move |_| on_confirm.run(true)
                    >
                        "Delete, allow re-import"
                    </button>
                }
                    .into_any()
            } else {
                view! {
                    <button
                        class=style::action_btn
                        type="button"
                        node_ref=primary
                        disabled=move || acting.get() || provenance.with(Option::is_none)
                        on:click=move |_| on_confirm.run(false)
                    >
                        "Delete"
                    </button>
                }
                    .into_any()
            }
        }}
        <button class=style::action_btn type="button" on:click=move |_| on_cancel.run(())>
            "Cancel"
        </button>
    }
}

/// Reverse confirmation in the register detail.
#[cfg(target_arch = "wasm32")]
#[component]
fn ReverseGate(
    /// The saved transaction's date, which the reversal takes.
    date: jiff::civil::Date,
    /// Whether the audit trail records an earlier reversal.
    already_reversed: Signal<bool>,
    /// Whether the detail holds unsaved edits.
    dirty: Signal<bool>,
    /// Whether a delete or reverse is in flight.
    acting: Signal<bool>,
    /// Adds the reversing transaction.
    on_confirm: Callback<()>,
    /// Closes the gate.
    on_cancel: Callback<()>,
) -> impl IntoView {
    let primary = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        if let Some(button) = primary.get() {
            drop(button.focus());
        }
    });
    view! {
        <span>{reverse_headline(date)}</span>
        {move || {
            reverse_warnings(already_reversed.get(), dirty.get())
                .into_iter()
                .map(|w| view! { <span class=style::gate_warning>{w}</span> })
                .collect_view()
        }}
        <button
            class=style::action_btn
            type="button"
            node_ref=primary
            disabled=move || acting.get()
            on:click=move |_| on_confirm.run(())
        >
            "Reverse"
        </button>
        <button class=style::action_btn type="button" on:click=move |_| on_cancel.run(())>
            "Cancel"
        </button>
    }
}

/// Save-bar text when the transaction changed under an open edit.
#[cfg(target_arch = "wasm32")]
const CONFLICT_MESSAGE: &str = "This transaction changed since you opened it. \
     Your edits are still here. Copy what you need, then reload.";

/// Maps a [`bc_ipc::BcError`] from a failed save to a friendly message.
///
/// # Arguments
///
/// * `error` - The error returned by the save IPC call.
///
/// # Returns
///
/// A short, user-facing description of the failure.
#[cfg(target_arch = "wasm32")]
fn friendly_save_error(error: &bc_ipc::BcError) -> String {
    match error {
        bc_ipc::BcError::Validation(message) => format!("Couldn't save: {message}"),
        bc_ipc::BcError::Conflict(_) => CONFLICT_MESSAGE.to_owned(),
        bc_ipc::BcError::NotFound(_) | bc_ipc::BcError::Internal(_) | _ => {
            format!("Couldn't save changes: {error}")
        }
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::Posting;
    use bc_ipc::PostingAmount;
    use bc_ipc::Quote;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::RowPerspective;
    use super::counterpart_names;
    use super::headline_amount;
    use super::headline_price;
    use super::summarise_tags;

    /// Builds a posting; `minor` gives cents for a stored amount, `None` for a
    /// zero-residual derived (elided) leg.
    fn posting(id: &str, acct: &str, minor: Option<i64>) -> Posting {
        let amount = match minor {
            Some(m) => PostingAmount::Stored(Amount::new(Decimal::new(m, 2), "AUD")),
            None => PostingAmount::Derived(vec![]),
        };
        Posting::new(
            id,
            AccountRef::new(acct, acct),
            amount,
            vec![],
            vec![],
            None,
            None,
        )
    }

    /// Builds a posting whose amount is derived to a known single-commodity
    /// residual (mirrors what the backend would compute for a single elided leg).
    fn derived_posting(id: &str, acct: &str, minor: i64) -> Posting {
        Posting::new(
            id,
            AccountRef::new(acct, acct),
            PostingAmount::Derived(vec![Amount::new(Decimal::new(minor, 2), "AUD")]),
            vec![],
            vec![],
            None,
            None,
        )
    }

    fn tx(postings: Vec<Posting>) -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "",
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            postings,
            vec![],
            true,
        )
    }

    /// Builds a posting whose units carry a price annotation.
    fn priced(id: &str, acct: &str, units: Decimal, code: &str, price: Quote) -> Posting {
        Posting::new(
            id,
            AccountRef::new(acct, acct),
            PostingAmount::Stored(Amount::new(units, code)),
            vec![],
            vec![],
            None,
            None,
        )
        .with_price(Some(price))
    }

    /// Builds a total-price [`Quote`] in AUD cents.
    fn total_aud(cents: i64) -> Quote {
        Quote::Total(Amount::new(Decimal::new(cents, 2), "AUD"))
    }

    #[test]
    fn account_headline_sums_focal_postings() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_ids: vec!["checking".to_owned()],
            },
        );
        assert_eq!(amt.value, Decimal::new(-8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn account_headline_nets_postings_across_the_rollup_set() {
        let t = tx(vec![
            posting("a", "savings", Some(-8_420)),
            posting("b", "holiday", Some(2_000)),
            posting("c", "groceries", Some(6_420)),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_ids: vec![
                    "offset".to_owned(),
                    "savings".to_owned(),
                    "holiday".to_owned(),
                ],
            },
        );
        assert_eq!(amt.value, Decimal::new(-6_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn counterparts_exclude_every_account_in_the_rollup_set() {
        let t = tx(vec![
            posting("a", "savings", Some(-8_420)),
            posting("b", "holiday", Some(2_000)),
            posting("c", "groceries", Some(6_420)),
        ]);
        let perspective = RowPerspective::Account {
            account_ids: vec![
                "offset".to_owned(),
                "savings".to_owned(),
                "holiday".to_owned(),
            ],
        };
        assert_eq!(counterpart_names(&t, &perspective), vec!["groceries"]);
    }

    #[test]
    fn account_headline_unknown_account_is_empty() {
        let t = tx(vec![posting("a", "checking", Some(-8_420))]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_ids: vec!["savings".to_owned()],
            },
        );
        assert_eq!(amt.value, Decimal::ZERO);
        assert_eq!(amt.currency_code, "");
    }

    #[test]
    fn global_headline_sums_positive_legs() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        let amt = headline_amount(&t, &RowPerspective::Global);
        assert_eq!(amt.value, Decimal::new(8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn headline_price_for_one_priced_focal_leg() {
        let t = tx(vec![
            priced("a", "usd", Decimal::new(400, 2), "USD", total_aud(637)),
            posting("b", "aud", Some(-637)),
        ]);
        let usd = RowPerspective::Account {
            account_ids: vec!["usd".to_owned()],
        };
        assert_eq!(headline_price(&t, &usd), Some(total_aud(637)));
        let aud = RowPerspective::Account {
            account_ids: vec!["aud".to_owned()],
        };
        assert_eq!(headline_price(&t, &aud), None);
    }

    #[test]
    fn headline_price_is_none_when_several_legs_sum() {
        let t = tx(vec![
            priced("a", "usd", Decimal::new(400, 2), "USD", total_aud(637)),
            priced("c", "usd", Decimal::new(100, 2), "USD", total_aud(159)),
            posting("b", "aud", Some(-796)),
        ]);
        let usd = RowPerspective::Account {
            account_ids: vec!["usd".to_owned()],
        };
        assert_eq!(headline_price(&t, &usd), None);
    }

    #[test]
    fn headline_price_global_uses_the_single_positive_leg() {
        let t = tx(vec![
            priced("a", "usd", Decimal::new(400, 2), "USD", total_aud(637)),
            posting("b", "aud", Some(-637)),
        ]);
        assert_eq!(
            headline_price(&t, &RowPerspective::Global),
            Some(total_aud(637))
        );
        let two_positive = tx(vec![
            priced("a", "usd", Decimal::new(400, 2), "USD", total_aud(637)),
            posting("c", "aud", Some(100)),
            posting("b", "aud", Some(-737)),
        ]);
        assert_eq!(headline_price(&two_positive, &RowPerspective::Global), None);
    }

    #[test]
    fn headline_price_is_none_when_an_elided_leg_shares_the_account() {
        let t = tx(vec![
            priced("a", "usd", Decimal::new(400, 2), "USD", total_aud(637)),
            derived_posting("c", "usd", 100),
            posting("b", "aud", Some(-737)),
        ]);
        let usd = RowPerspective::Account {
            account_ids: vec!["usd".to_owned()],
        };
        assert_eq!(headline_price(&t, &usd), None);
    }

    #[test]
    fn budget_headline_is_core_contribution() {
        let t = tx(vec![
            posting("a", "groceries", Some(4_000)),
            posting("b", "bank", Some(-4_000)),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Budget {
                counted: vec!["a".to_owned()],
                contribution: Some(Amount::new(Decimal::new(4_000, 2), "AUD")),
            },
        );
        assert_eq!(amt, Amount::new(Decimal::new(4_000, 2), "AUD"));
    }

    #[test]
    fn budget_headline_without_contribution_is_empty() {
        let t = tx(vec![
            posting("a", "groceries", Some(4_000)),
            posting("b", "bank", Some(-4_000)),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Budget {
                counted: vec!["a".to_owned()],
                contribution: None,
            },
        );
        assert_eq!(amt.currency_code, "");
    }

    #[test]
    fn budget_counterparts_exclude_counted_postings() {
        let t = tx(vec![
            posting("a", "groceries", Some(4_000)),
            posting("b", "bank", Some(-4_000)),
        ]);
        let names = counterpart_names(
            &t,
            &RowPerspective::Budget {
                counted: vec!["a".to_owned()],
                contribution: None,
            },
        );
        assert_eq!(names, vec!["bank"]);
    }

    #[test]
    fn payee_initial_first_letter() {
        assert_eq!(super::payee_initial("Coles"), 'C');
    }

    #[test]
    fn payee_initial_skips_non_alpha() {
        assert_eq!(super::payee_initial("123 Foo"), 'F');
    }

    #[test]
    fn payee_initial_empty_returns_question_mark() {
        assert_eq!(super::payee_initial(""), '?');
    }

    #[rstest]
    #[case::no_context_year(None, "2026-04-30")]
    #[case::matching_context_year(Some(2026), "04-30")]
    #[case::other_context_year(Some(2025), "2026-04-30")]
    fn format_date_display_is_iso(#[case] context_year: Option<i16>, #[case] expected: &str) {
        assert_eq!(
            super::format_date_display(jiff::civil::Date::constant(2026, 4, 30), context_year),
            expected
        );
    }

    #[test]
    fn account_headline_infers_elided_focal_leg() {
        let t = tx(vec![
            posting("a", "groceries", Some(8_420)),
            derived_posting("b", "checking", -8_420), // elided focal leg
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_ids: vec!["checking".to_owned()],
            },
        );
        assert_eq!(amt.value, Decimal::new(-8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn account_headline_ambiguous_elided_is_empty() {
        let t = tx(vec![
            posting("a", "checking", None),
            posting("b", "groceries", None),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_ids: vec!["checking".to_owned()],
            },
        );
        assert_eq!(amt.currency_code, "");
    }

    #[rstest]
    #[case::none(&[], None, &[])]
    #[case::one(&["recurring"], Some("recurring"), &[])]
    #[case::three(&["recurring", "subscription", "shared"], Some("recurring"), &["subscription", "shared"])]
    fn summarise_tags_keeps_the_first_and_counts_the_rest(
        #[case] tags: &[&str],
        #[case] first: Option<&str>,
        #[case] rest: &[&str],
    ) {
        let tag_strings: Vec<String> = tags.iter().map(ToString::to_string).collect();
        let summary = summarise_tags(&tag_strings);
        assert_eq!(summary.first.as_deref(), first);
        assert_eq!(
            summary.rest,
            rest.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }
}
