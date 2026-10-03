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
use crate::label::category_label;

/// Editor-friendly working-buffer model for the editable transaction view.
///
/// Contains pure data structures ([`editable::EditableTransaction`] and
/// [`editable::EditablePosting`]) for representing transactions and postings
/// in the process of being edited. These structures use strings for all
/// numeric/date fields to represent parse-in-progress values.
pub mod editable;

/// Pure currency-marker resolution for amount inputs.
///
/// Maps a marker string (symbol, alias, or code) to a canonical commodity code
/// against the loaded commodity set, with longest-match precedence and
/// ambiguity detection. Native-testable — no Leptos or WASM here.
pub mod currency;

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

/// Pure helpers for the cost chip: buffer/[`bc_ipc::Cost`] conversion and
/// chip/quote text.
pub mod cost;

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
    /// Budget page: focal postings are those on `account_id`; headline is their
    /// period/spread-prorated sum over `[window_start, window_end]`.
    ///
    /// `tag_filter` is carried for future tag-filter narrowing; until tag paths
    /// are resolved through IPC it is unused for matching (see issue #182).
    Budget {
        /// The account this budget targets.
        account_id: String,
        /// Optional tag-filter path for a sub-budget (currently informational).
        tag_filter: Option<String>,
        /// Inclusive start of the displayed budget period.
        window_start: jiff::civil::Date,
        /// Inclusive end of the displayed budget period.
        window_end: jiff::civil::Date,
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
    let is_focal = |id: &str| match perspective {
        RowPerspective::Account { account_ids } => account_ids.iter().any(|a| a == id),
        RowPerspective::Budget { account_id, .. } => account_id == id,
        RowPerspective::Global => false,
    };
    tx.postings
        .iter()
        .filter(|p| !is_focal(&p.account.id))
        .map(|p| p.account.name.as_str())
        .collect()
}

/// Computes the headline [`Amount`] for `tx` under `perspective`.
///
/// Returns an `Amount` with an empty currency code (rendered as `—`) when no
/// focal posting carries a concrete amount.
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
        RowPerspective::Budget {
            account_id,
            window_start,
            window_end,
            ..
        } => {
            let mut total = Decimal::ZERO;
            let mut currency = String::new();
            for p in focal_on_accounts(tx, core::slice::from_ref(account_id)) {
                let Some(a) = p.amount.display_amount() else {
                    continue;
                };
                if currency.is_empty() {
                    currency.clone_from(&a.currency_code);
                }
                // A derived leg carries no spread, so it contributes whole.
                let contribution = if p.amount.is_elided() {
                    a.value
                } else {
                    prorated_value(p, *window_start, *window_end)
                };
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "prorated same-commodity focal sum"
                )]
                {
                    total += contribution;
                }
            }
            Amount::new(total, currency)
        }
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
/// (the same legs `headline_amount` sums); `Budget` prorates and has no
/// single price.
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

/// Returns the contribution of `p` to the period `[window_start, window_end]`.
///
/// A posting with a `spread_from`/`spread_until` range contributes its value
/// scaled by the fraction of spread days that fall inside the window. A posting
/// with no full spread range contributes its whole value.
///
/// # Arguments
///
/// * `p` - The posting to prorate.
/// * `window_start` - Inclusive start of the window.
/// * `window_end` - Inclusive end of the window.
///
/// # Returns
///
/// The prorated decimal value for the given window.
#[must_use]
pub fn prorated_value(
    p: &Posting,
    window_start: jiff::civil::Date,
    window_end: jiff::civil::Date,
) -> Decimal {
    let Some(value) = p.amount.stored().map(|a| a.value) else {
        return Decimal::ZERO;
    };
    let (Some(from), Some(until)) = (p.spread_from, p.spread_until) else {
        return value;
    };
    let total_days = inclusive_days(from, until);
    if total_days <= 0 {
        return value;
    }
    let overlap_start = from.max(window_start);
    let overlap_end = until.min(window_end);
    let overlap_days = inclusive_days(overlap_start, overlap_end).max(0);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "proration arithmetic: Decimal multiplication and division by bounded day counts; practical values never overflow"
    )]
    {
        value * Decimal::from(overlap_days) / Decimal::from(total_days)
    }
}

/// Returns the inclusive day count between two civil dates (`a`..=`b`).
///
/// Returns `0` when `b < a`.
fn inclusive_days(a: jiff::civil::Date, b: jiff::civil::Date) -> i64 {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "jiff Date subtraction returns a Span bounded by calendar range; +1 for inclusive count cannot overflow i64"
    )]
    {
        let days = i64::from((b - a).get_days());
        if days < 0 { 0 } else { days + 1 }
    }
}

/// Renders the Category column cell.
///
/// A label too wide for the cell is cut with an ellipsis; the full label is
/// the cell's tooltip. *split transaction* renders in muted italic.
///
/// # Arguments
///
/// * `label` - The expansion string from [`crate::label::category_label`] (e.g.
///   `"Expenses :: {Groceries, Healthcare}"` or `"—"`).
#[cfg(target_arch = "wasm32")]
#[component]
fn CategoryCell(
    /// Computed category label — either an account name, a shell expansion, or `"—"`.
    label: String,
) -> impl IntoView {
    let class = if label == crate::label::SPLIT_LABEL {
        format!("{} {}", style::category, style::category_split)
    } else {
        style::category.to_owned()
    };
    let title = label.clone();
    view! {
        <span class=class title=title>
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
/// * `tx` - The transaction to render.
/// * `perspective` - Determines which postings are focal and how amounts are derived.
/// * `selected` - Whether this row has keyboard focus.
/// * `expanded` - Optional external signal controlling expansion state.
/// * `on_toggle` - Optional callback called when the row is toggled.
/// * `on_change` - Optional callback called when the transaction is mutated.
/// * `context_year` - The year the view's period makes obvious; dates in it
///   drop their year.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props must be owned values"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn TransactionRow(
    /// The transaction to render.
    tx: Transaction,
    /// Determines which postings are focal and how the headline amount is derived.
    perspective: RowPerspective,
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
    #[prop(optional)]
    matched_postings: Option<Vec<String>>,
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

    let amount = headline_amount(&tx, &perspective);
    let currencies = crate::currency_ctx::use_currency_store();
    let amount_str = {
        let amount = amount.clone();
        move || {
            if amount.currency_code.is_empty() {
                "\u{2014}".to_owned()
            } else {
                let meta = crate::components::num::meta::display_meta_for(
                    &amount.currency_code,
                    &currencies.get(),
                );
                crate::components::num::format_amount(&amount.value, &meta)
            }
        }
    };
    let price_str = {
        let tx = tx.clone();
        let perspective = perspective.clone();
        move || {
            headline_price(&tx, &perspective).map(|q| {
                let meta = crate::components::num::meta::display_meta_for(
                    &q.amount().currency_code,
                    &currencies.get(),
                );
                crate::components::transaction_row::cost::quote_text(&q, |a| {
                    crate::components::num::format_amount(&a.value, &meta)
                })
            })
        }
    };
    let amt_class = match amount.value.cmp(&Decimal::ZERO) {
        core::cmp::Ordering::Greater => style::amt_pos,
        core::cmp::Ordering::Less => style::amt_neg,
        core::cmp::Ordering::Equal => style::amt_neu,
    };

    let tx_date = tx.date;
    let date = move || format_date_display(tx_date, context_year.get());
    // `payee` is an ordinary metadata key with no privileged position; it is read
    // like any other, and a flagged entry still reads as the text the user typed.
    let payee = crate::components::meta_editor::model::first_text_by_key(&tx.metadata, "payee")
        .filter(|text| !text.is_empty());
    let has_desc = !tx.description.is_empty();
    let initial = payee_initial(payee.unwrap_or(&tx.description)).to_string();
    let (display_name, name_class) = if let Some(name) = payee {
        (name.to_owned(), style::payee.to_owned())
    } else if has_desc {
        (
            tx.description.clone(),
            format!("{} {}", style::payee, style::name_dim),
        )
    } else {
        ("\u{2014}".to_owned(), style::payee.to_owned())
    };

    let category = category_label(&counterpart_names(&tx, &perspective));

    let tags: Vec<String> = tx.tags.iter().map(|t| t.path.clone()).collect();
    let tags_mobile = tags.clone();
    let split = tx.postings.len() > 2;
    let unbalanced = !tx.balanced;
    let flagged = tx.reconciliation == bc_ipc::Reconciliation::Flagged;
    let unrec = tx.reconciliation == bc_ipc::Reconciliation::Unreconciled;
    let split_count = tx.postings.len();

    let toggle_click = toggle;
    let toggle_key = toggle;

    let has_balance = balance.is_some();
    let tx_id = tx.id.clone();

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
                    {initial}
                </span>
                <span class=name_class>{display_name}</span>
                {flagged
                    .then(|| {
                        view! {
                            <span class=style::glyph_flag aria-label="flagged" title="flagged">
                                "\u{2691}"
                            </span>
                        }
                    })}
                {unrec
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
                    })}
                <div class=style::inline_tags>
                    {tags.into_iter().map(|t| view! { <TagToken label=t /> }).collect::<Vec<_>>()}
                </div>
            </div>
            <div class=style::tags_cell>
                {tags_mobile
                    .into_iter()
                    .map(|t| view! { <TagToken label=t /> })
                    .collect::<Vec<_>>()}
            </div>
            <CategoryCell label=category />
            <span class=format!("{} {}", style::amount, amt_class)>
                <span class=style::amt_stack>
                    <span>{amount_str}</span>
                    {move || {
                        let price = price_str();
                        (price.is_some() || split || unbalanced)
                            .then(|| {
                                view! {
                                    <span class=style::amt_sub>
                                        {price}
                                        {split
                                            .then(|| {
                                                view! {
                                                    <span class=style::pill_split>
                                                        "split \u{00b7} " {split_count}
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
            let tx_detail = tx.clone();
            let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));
            let on_saved_cb = on_saved.unwrap_or_else(|| Callback::new(|_| {}));
            let accounts = StoredValue::new(accounts);
            let all_tags = StoredValue::new(all_tags);
            let matched = StoredValue::new(matched_postings);
            move || {
                expanded
                    .get()
                    .then(|| {
                        view! {
                            <TransactionDetail
                                tx=tx_detail.clone()
                                on_change=on_change_cb
                                on_saved=on_saved_cb
                                accounts=accounts.get_value()
                                all_tags=all_tags.get_value()
                                matched_postings=matched.get_value()
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
/// * `on_change` - Optional callback run after a successful save; defaults to a
///   no-op when `None`.
/// * `accounts` - All selectable accounts for the recategorise picker; an empty
///   list degrades to free-text-only pickers.
/// * `all_tags` - Known tags to seed the pickers; empty falls back to the IPC
///   fetch.
#[cfg(target_arch = "wasm32")]
#[component]
fn TransactionDetail(
    /// The transaction to render.
    tx: Transaction,
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

    let ctx_reload = ctx.clone();
    let discard_and_reload = Callback::new(move |()| {
        // Same guard as `discard`: never race a save's own refetch.
        if saving.get_untracked() {
            return;
        }
        let id = ctx_reload.working.with_untracked(|w| w.id.clone());
        let ctx_reload = ctx_reload.clone();
        leptos::task::spawn_local(async move {
            let fetched = bc_ipc::client::get_transaction(&id).await;
            if working.is_disposed() {
                return;
            }
            match fetched {
                Ok(fresh) => {
                    let fresh = EditableTransaction::from(&fresh);
                    f_date.set(fresh.date.clone());
                    f_desc.set(fresh.description.clone());
                    ctx_reload.original.set_value(fresh);
                    ctx_reload.discard();
                    stale_base.set(false);
                    error.set(None);
                    on_change_cb.run(());
                    audit_version.update(|v| *v = v.wrapping_add(1));
                }
                Err(e) => error.set(Some(format!("Couldn't reload the transaction: {e}"))),
            }
        });
    });

    let ctx_save = ctx.clone();
    let save = Callback::new(move |()| {
        if saving.get_untracked()
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

        if key == "Escape" {
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
                                                disabled=move || save_disabled.get()
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
    use super::prorated_value;

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
    fn prorate_full_overlap_returns_full_value() {
        let mut p = posting("a", "insurance", Some(12_000));
        p.spread_from = Some(Date::constant(2026, 1, 1));
        p.spread_until = Some(Date::constant(2026, 1, 31));
        let v = prorated_value(&p, Date::constant(2026, 1, 1), Date::constant(2026, 1, 31));
        assert_eq!(v, Decimal::new(12_000, 2));
    }

    #[test]
    fn prorate_half_overlap_halves_value() {
        // 30-day spread (Jun 1-30); window covers Jun 1-15 = 15 of 30 days.
        let mut p = posting("a", "insurance", Some(30_000));
        p.spread_from = Some(Date::constant(2026, 6, 1));
        p.spread_until = Some(Date::constant(2026, 6, 30));
        let v = prorated_value(&p, Date::constant(2026, 6, 1), Date::constant(2026, 6, 15));
        assert_eq!(v, Decimal::new(15_000, 2));
    }

    #[test]
    fn prorate_no_spread_returns_full_value_inside_window() {
        let p = posting("a", "groceries", Some(8_420));
        let v = prorated_value(&p, Date::constant(2026, 4, 1), Date::constant(2026, 4, 30));
        assert_eq!(v, Decimal::new(8_420, 2));
    }

    #[test]
    fn budget_headline_prorates_spread_postings() {
        let mut p = posting("a", "insurance", Some(30_000));
        p.spread_from = Some(Date::constant(2026, 6, 1));
        p.spread_until = Some(Date::constant(2026, 6, 30));
        let t = tx(vec![p, posting("b", "expenses", Some(-30_000))]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Budget {
                account_id: "insurance".to_owned(),
                tag_filter: None,
                window_start: Date::constant(2026, 6, 1),
                window_end: Date::constant(2026, 6, 15),
            },
        );
        // 15 of 30 days → half value
        assert_eq!(amt.value, Decimal::new(15_000, 2));
        assert_eq!(amt.currency_code, "AUD");
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

    #[test]
    fn budget_headline_infers_elided_focal_leg() {
        let t = tx(vec![
            posting("a", "groceries", Some(8_420)),
            derived_posting("b", "checking", -8_420),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Budget {
                account_id: "checking".to_owned(),
                tag_filter: None,
                window_start: Date::constant(2026, 1, 1),
                window_end: Date::constant(2026, 12, 31),
            },
        );
        assert_eq!(amt.value, Decimal::new(-8_420, 2));
    }
}
