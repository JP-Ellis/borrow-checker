//! Shared, posting-aware transaction row used by the accounts and budget pages.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::Posting;
use bc_ipc::Transaction;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
use rust_decimal::Decimal;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

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

/// Pure helpers for rendering and seeding per-posting accrual spreads.
pub mod spread;

// MARK: WASM bindings

#[cfg(target_arch = "wasm32")]
/// Bindings to JavaScript `Date.UTC()` for constructing UTC epoch milliseconds.
mod wasm_bindings {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date, js_name = "UTC")]
        /// Computes the epoch milliseconds for a UTC date.
        ///
        /// Wraps `Date.UTC(year, month, date)` where month is 0-based.
        pub fn utc(year: f64, month: f64, date: f64) -> f64;
    }
}

// MARK: Pure display helpers

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

/// Formats a [`jiff::civil::Date`] for display.
///
/// On WASM: delegates to the browser's `Intl.DateTimeFormat` using UTC timezone.
/// Having a typed `Date` means callers can also access `date.year()`,
/// `date.month()`, and `date.day()` directly to build locale-aware `Intl.*`
/// expressions without any intermediate string.
/// Fallback (native test builds): returns `"MM/DD"`.
///
/// # Arguments
///
/// * `date` - The civil date to format.
///
/// # Returns
///
/// A locale-formatted date string (e.g. `"04/30"` in `en-AU`).
#[must_use]
#[inline]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::arithmetic_side_effects,
        reason = "month() returns 1-12; minus one is 0-11 for JS Date.UTC()"
    )
)]
pub fn format_date_display(date: jiff::civil::Date) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        use js_sys::Array;
        use js_sys::Date;
        use js_sys::Intl::DateTimeFormat;
        use js_sys::Object;
        use js_sys::Reflect;
        use web_sys::wasm_bindgen::JsValue;

        let options = Object::new();
        drop(Reflect::set(
            &options,
            &JsValue::from_str("month"),
            &JsValue::from_str("2-digit"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("day"),
            &JsValue::from_str("2-digit"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("timeZone"),
            &JsValue::from_str("UTC"),
        ));

        let ts = wasm_bindings::utc(
            f64::from(date.year()),
            f64::from(i32::from(date.month()) - 1),
            f64::from(date.day()),
        );
        let js_date = Date::new(&JsValue::from_f64(ts));
        let fmt = DateTimeFormat::new(&Array::new(), &options);
        let format_fn = fmt.format();
        format_fn
            .call1(&JsValue::NULL, &js_date)
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| date.to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("{:02}/{:02}", date.month(), date.day())
    }
}

#[cfg(target_arch = "wasm32")]
import_style!(pub(crate) style, "row.module.scss");

/// Determines which postings are focal and how the headline amount is derived.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RowPerspective {
    /// Accounts page: focal postings are those on `account_id`; headline is
    /// their net sum.
    Account {
        /// The account currently in view.
        account_id: String,
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

/// Returns the focal postings for `account_id` within `tx`.
///
/// # Arguments
///
/// * `tx` - The transaction to search.
/// * `account_id` - The account ID to match against posting accounts.
///
/// # Returns
///
/// An iterator over postings whose account ID matches `account_id`.
pub fn focal_on_account<'a>(
    tx: &'a Transaction,
    account_id: &'a str,
) -> impl Iterator<Item = &'a Posting> {
    tx.postings
        .iter()
        .filter(move |p| p.account.id == account_id)
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
        RowPerspective::Account { account_id } => {
            let inferred = inferred_amount(tx);
            let mut total = Decimal::ZERO;
            let mut currency = String::new();
            let mut any = false;
            for p in focal_on_account(tx, account_id) {
                let amt = match p.amount.as_ref() {
                    Some(a) => Some(a.clone()),
                    None => inferred.clone(),
                };
                if let Some(a) = amt {
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
            let inferred = inferred_amount(tx);
            let mut total = Decimal::ZERO;
            let mut currency = String::new();
            for p in focal_on_account(tx, account_id) {
                let contribution = match p.amount.as_ref() {
                    Some(a) => {
                        if currency.is_empty() {
                            currency.clone_from(&a.currency_code);
                        }
                        prorated_value(p, *window_start, *window_end)
                    }
                    None => match inferred.as_ref() {
                        Some(a) => {
                            if currency.is_empty() {
                                currency.clone_from(&a.currency_code);
                            }
                            a.value // inferred leg has no spread; contributes whole value
                        }
                        None => Decimal::ZERO,
                    },
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
                .filter_map(|p| p.amount.as_ref())
                .filter(|a| a.value > Decimal::ZERO),
        ),
    }
}

/// Returns the amount a single elided leg infers to, or `None` when zero or two
/// or more legs are elided (inference undefined). Mirrors `is_balanced`.
///
/// # Arguments
///
/// * `tx` - The transaction to infer from.
///
/// # Returns
///
/// The inferred [`Amount`] for the single elided leg, or `None` if inference
/// is undefined.
fn inferred_amount(tx: &Transaction) -> Option<Amount> {
    let elided = tx.postings.iter().filter(|p| p.amount.is_none()).count();
    if elided != 1 {
        return None;
    }
    let mut total = Decimal::ZERO;
    let mut currency = String::new();
    for a in tx.postings.iter().filter_map(|p| p.amount.as_ref()) {
        if currency.is_empty() {
            currency.clone_from(&a.currency_code);
        }
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "same-commodity sum within one transaction; no overflow in practice"
        )]
        {
            total += a.value;
        }
    }
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "same-commodity sum within one transaction; no overflow in practice"
    )]
    Some(Amount::new(Decimal::ZERO - total, currency))
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

/// Returns the transaction as the collapsed row should summarise it under
/// `strictness`. In `Strict` mode with a known matched-leg set, non-matching
/// legs are dropped from a clone (so the counterpart cell, split pill, and
/// headline reflect only matched legs). In `Lenient` mode, or when the matched
/// set is unknown, the transaction is returned unchanged (cloned).
///
/// # Arguments
///
/// * `tx` - The full transaction.
/// * `matched` - Ids of legs that matched the posting-scoped predicates, or
///   `None` when unfiltered (all legs match).
/// * `strictness` - The active presentation strictness.
#[must_use]
pub fn strict_render_tx(
    tx: &Transaction,
    matched: Option<&[String]>,
    strictness: crate::filter_ctx::Strictness,
) -> Transaction {
    let mut out = tx.clone();
    if let (crate::filter_ctx::Strictness::Strict, Some(ids)) = (strictness, matched) {
        out.postings.retain(|p| ids.iter().any(|id| id == &p.id));
    }
    out
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
    let Some(value) = p.amount.as_ref().map(|a| a.value) else {
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

/// Returns whether `tx` is structurally balanced.
///
/// Mirrors `bc_models::Transaction::balanced`: false with no concrete legs or
/// two-or-more elided legs; a single elided leg auto-balances; otherwise every
/// commodity's concrete legs must sum to zero.
///
/// # Arguments
///
/// * `tx` - The transaction to check.
///
/// # Returns
///
/// `true` if the transaction is balanced, `false` otherwise.
#[must_use]
pub fn is_balanced(tx: &Transaction) -> bool {
    let elided = tx.postings.iter().filter(|p| p.amount.is_none()).count();
    if elided >= 2 {
        return false;
    }
    let mut totals: BTreeMap<&str, Decimal> = BTreeMap::new();
    for a in tx.postings.iter().filter_map(|p| p.amount.as_ref()) {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "balance check: summing monetary values of the same commodity within a single transaction"
        )]
        {
            *totals.entry(a.currency_code.as_str()).or_default() += a.value;
        }
    }
    if totals.is_empty() {
        return false;
    }
    if elided == 1 {
        return true;
    }
    totals.values().all(Decimal::is_zero)
}

/// Renders the Category column cell with overflow-aware fallback.
///
/// Displays the pre-computed `label` string. If the rendered text overflows
/// the cell, replaces it with *split transaction* in muted italic style.
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
    let span_ref = NodeRef::<leptos::html::Span>::new();
    let is_split = label == crate::label::SPLIT_LABEL;
    let use_fallback = RwSignal::new(is_split);
    let label = StoredValue::new(label);

    Effect::new(move |_| {
        if let Some(el) = span_ref.get()
            && el.scroll_width() > el.client_width()
        {
            use_fallback.set(true);
        }
    });

    view! {
        <span
            class=move || {
                if use_fallback.get() {
                    format!("{} {}", style::category, style::category_split)
                } else {
                    style::category.to_owned()
                }
            }
            node_ref=span_ref
        >
            {move || {
                if use_fallback.get() {
                    crate::label::SPLIT_LABEL.to_owned()
                } else {
                    label.get_value()
                }
            }}
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
/// * `on_change` - Optional callback called when the transaction is mutated (wired in Task 4).
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
    /// when the register is unfiltered. Drives strict-mode leg hiding.
    #[prop(optional)]
    matched_postings: Option<Vec<String>>,
    /// Presentation strictness for the collapsed leg summary.
    #[prop(optional)]
    strictness: crate::filter_ctx::Strictness,
) -> impl IntoView {
    let local_expanded = RwSignal::new(false);
    let expanded: Signal<bool> = expanded.unwrap_or_else(|| local_expanded.into());
    let toggle = move || match on_toggle {
        Some(cb) => cb.run(()),
        None => local_expanded.update(|e| *e = !*e),
    };

    let render_tx = strict_render_tx(&tx, matched_postings.as_deref(), strictness);
    let amount = headline_amount(&render_tx, &perspective);
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
    let amt_class = match amount.value.cmp(&Decimal::ZERO) {
        core::cmp::Ordering::Greater => style::amt_pos,
        core::cmp::Ordering::Less => style::amt_neg,
        core::cmp::Ordering::Equal => style::amt_neu,
    };

    let date = format_date_display(tx.date);
    let has_payee = !tx.payee.is_empty();
    let has_desc = !tx.description.is_empty();
    let initial = payee_initial(if has_payee {
        &tx.payee
    } else {
        &tx.description
    })
    .to_string();
    let (display_name, name_class) = if has_payee {
        (tx.payee.clone(), style::payee.to_owned())
    } else if has_desc {
        (
            tx.description.clone(),
            format!("{} {}", style::payee, style::name_dim),
        )
    } else {
        ("\u{2014}".to_owned(), style::payee.to_owned())
    };

    let focal_id: Option<String> = match &perspective {
        RowPerspective::Account { account_id } | RowPerspective::Budget { account_id, .. } => {
            Some(account_id.clone())
        }
        RowPerspective::Global => None,
    };
    let counterpart_names: Vec<&str> = render_tx
        .postings
        .iter()
        .filter(|p| focal_id.as_deref() != Some(p.account.id.as_str()))
        .map(|p| p.account.name.as_str())
        .collect();
    let category = category_label(&counterpart_names);

    let tags = tx.tags.clone();
    let tags_mobile = tags.clone();
    let split = render_tx.postings.len() > 2;
    let unbalanced = !is_balanced(&render_tx);
    let flagged = tx.reconciliation == bc_ipc::Reconciliation::Flagged;
    let unrec = tx.reconciliation == bc_ipc::Reconciliation::Unreconciled;
    let split_count = render_tx.postings.len();

    let toggle_click = toggle;
    let toggle_key = toggle;

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
                            <span class=style::glyph_flag aria-label="flagged">
                                "\u{2691}"
                            </span>
                        }
                    })}
                {unrec
                    .then(|| {
                        view! {
                            <span class=style::glyph_unrec aria-label="unreconciled">
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
            <span class=format!(
                "{} {}",
                style::amount,
                amt_class,
            )>
                {amount_str}
                {split
                    .then(|| {
                        view! {
                            <span class=style::pill_split>"split \u{00b7} " {split_count}</span>
                        }
                    })}
                {unbalanced
                    .then(|| {
                        view! { <span class=style::pill_unbalanced>"\u{26A0} unbalanced"</span> }
                    })}
            </span>
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
) -> impl IntoView {
    let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));
    let on_saved_cb = on_saved.unwrap_or_else(|| Callback::new(|_| {}));
    let editable = EditableTransaction::from(&tx);
    let ctx = TxEditCtx::new(editable, accounts);
    provide_context(ctx.clone());

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
    let f_payee = RwSignal::new(ctx.working.with(|w| w.payee.clone()));
    let f_desc = RwSignal::new(ctx.working.with(|w| w.description.clone()));
    let f_note = RwSignal::new(ctx.working.with(|w| w.note.clone()));

    {
        let working = ctx.working;
        Effect::new(move |_| {
            let (date, payee, desc, note) =
                (f_date.get(), f_payee.get(), f_desc.get(), f_note.get());
            working.update(|w| {
                w.date = date;
                w.payee = payee;
                w.description = desc;
                w.note = note;
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
        ctx_discard.discard();
        original.with_value(|o| {
            f_date.set(o.date.clone());
            f_payee.set(o.payee.clone());
            f_desc.set(o.description.clone());
            f_note.set(o.note.clone());
        });
        error.set(None);
    });

    let ctx_save = ctx.clone();
    let save = Callback::new(move |()| {
        if saving.get_untracked() || !ctx_save.dirty() || save_disabled.get_untracked() {
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
        let recon_changed = original.with_value(|o| o.reconciliation) != working_now.reconciliation;
        let id = working_now.id.clone();
        let recon = working_now.reconciliation;
        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::edit_transaction(&edit).await {
                Ok(()) => {
                    if let Some(date) = saved_date {
                        on_saved_cb.run(date);
                    }
                    if recon_changed
                        && let Err(e) = bc_ipc::client::set_reconciliation(&id, recon).await
                    {
                        saving.set(false);
                        // The edit persisted but the reconciliation change did
                        // not. Snapshot the saved (non-reconciliation) state as
                        // the new pristine so Discard won't revert it, leaving
                        // only the reconciliation change marked dirty.
                        let mut saved = working.get_untracked();
                        saved.reconciliation = original.with_value(|o| o.reconciliation);
                        original.set_value(saved);
                        working.update(|_| {});
                        on_change_cb.run(());
                        audit_version.update(|v| *v = v.wrapping_add(1));
                        error.set(Some(friendly_save_error(&e)));
                        return;
                    }
                    saving.set(false);
                    original.set_value(working.get_untracked());
                    working.update(|_| {});
                    on_change_cb.run(());
                    audit_version.update(|v| *v = v.wrapping_add(1));
                }
                Err(e) => {
                    saving.set(false);
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
            discard.run(());
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
                let (extra, text) = match balance_state.get() {
                    BalanceState::Balanced => (style::balance_ok, "balances".to_owned()),
                    BalanceState::Inferred { remainder, currency } => {
                        let meta = crate::components::num::meta::display_meta_for(
                            &currency,
                            &currencies.get(),
                        );
                        let amt = crate::components::num::format_amount(&remainder, &meta);
                        (style::balance_ok, format!("balances \u{2014} auto {amt}"))
                    }
                    BalanceState::Empty => (style::balance_ok, "no amounts yet".to_owned()),
                    BalanceState::Unbalanced { delta, currency } => {
                        let meta = crate::components::num::meta::display_meta_for(
                            &currency,
                            &currencies.get(),
                        );
                        let amt = crate::components::num::format_amount(&delta, &meta);
                        (style::balance_bad, format!("unbalanced \u{2014} \u{03A3} = {amt}"))
                    }
                    BalanceState::Ambiguous => {
                        (style::balance_bad, "more than one blank amount".to_owned())
                    }
                    BalanceState::Invalid => {
                        (style::balance_bad, "an amount does not parse".to_owned())
                    }
                };
                view! {
                    <div class=style::balance>
                        <span class=format!("{} {}", style::bal_text, extra)>{text}</span>
                    </div>
                }
            }}

            <div class=style::metamix>
                <div class=style::mm_main>
                    <div class=style::mm_fields>
                        <span class=style::metamix_lbl>"Payee"</span>
                        <div class=style::mm_val>
                            <input
                                class=format!("{} {}", style::f, style::textfield)
                                prop:value=move || f_payee.get()
                                on:input=move |ev| f_payee.set(event_target_value(&ev))
                                placeholder="payee"
                            />
                        </div>
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
                                on_add=Callback::new(move |p: String| {
                                    working
                                        .update(|w| {
                                            if !w.tags.contains(&p) {
                                                w.tags.push(p);
                                            }
                                        });
                                })
                                on_remove=Callback::new(move |p: String| {
                                    working.update(|w| w.tags.retain(|t| t != &p));
                                })
                                on_created=Callback::new(move |info: bc_ipc::TagInfo| {
                                    all_tags.update(|v| v.push(info));
                                })
                                compact=true
                            />
                        </div>
                        <span class=style::metamix_lbl>"Note"</span>
                        <div class=style::mm_val>
                            <input
                                class=format!("{} {}", style::f, style::note_input)
                                prop:value=move || f_note.get()
                                on:input=move |ev| f_note.set(event_target_value(&ev))
                                placeholder="add note…"
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
                        <span></span>
                        // Positional index key is safe here: each row reads/writes
                        // directly into `working.extra_dates[i]` with no per-row local
                        // signal that could go stale on reorder (unlike the postings
                        // list fixed in #210).
                        <For
                            each=move || {
                                working.with(|w| (0..w.extra_dates.len()).collect::<Vec<_>>())
                            }
                            key=|i| *i
                            children=move |i| {
                                view! {
                                    <input
                                        class=style::f
                                        prop:value=move || {
                                            working
                                                .with(|w| {
                                                    w.extra_dates
                                                        .get(i)
                                                        .map(|(l, _)| l.clone())
                                                        .unwrap_or_default()
                                                })
                                        }
                                        on:input=move |ev| {
                                            working
                                                .update(|w| {
                                                    if let Some(e) = w.extra_dates.get_mut(i) {
                                                        e.0 = event_target_value(&ev);
                                                    }
                                                });
                                        }
                                        placeholder="label"
                                    />
                                    <input
                                        class=format!("{} {}", style::f, style::f_num)
                                        prop:value=move || {
                                            working
                                                .with(|w| {
                                                    w.extra_dates
                                                        .get(i)
                                                        .map(|(_, d)| d.clone())
                                                        .unwrap_or_default()
                                                })
                                        }
                                        on:input=move |ev| {
                                            working
                                                .update(|w| {
                                                    if let Some(e) = w.extra_dates.get_mut(i) {
                                                        e.1 = event_target_value(&ev);
                                                    }
                                                });
                                        }
                                        placeholder="YYYY-MM-DD"
                                    />
                                    <span
                                        class=style::date_x
                                        role="button"
                                        tabindex="0"
                                        on:click=move |_| {
                                            working
                                                .update(|w| {
                                                    if i < w.extra_dates.len() {
                                                        w.extra_dates.remove(i);
                                                    }
                                                });
                                        }
                                    >
                                        "×"
                                    </span>
                                }
                            }
                        />
                        <button
                            class=style::add_date
                            type="button"
                            on:click=move |_| {
                                working
                                    .update(|w| w.extra_dates.push((String::new(), String::new())));
                            }
                        >
                            "+ date"
                        </button>
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
                                <button
                                    class=style::action_btn
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
                            </div>
                        }
                    })
            }}
        </div>
    }
}

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
        bc_ipc::BcError::NotFound(_) | bc_ipc::BcError::Internal(_) | _ => {
            format!("Couldn't save changes: {error}")
        }
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;

#[cfg(test)]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::Posting;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::RowPerspective;
    use super::headline_amount;
    use super::is_balanced;
    use super::prorated_value;

    fn posting(id: &str, acct: &str, minor: Option<i64>) -> Posting {
        Posting::new(
            id,
            AccountRef::new(acct, acct),
            minor.map(|m| Amount::new(Decimal::new(m, 2), "AUD")),
            None::<&str>,
            vec![],
            None,
            None,
        )
    }

    fn tx(postings: Vec<Posting>) -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "",
            None::<&str>,
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            postings,
            vec![],
        )
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
                account_id: "checking".to_owned(),
            },
        );
        assert_eq!(amt.value, Decimal::new(-8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn account_headline_unknown_account_is_empty() {
        let t = tx(vec![posting("a", "checking", Some(-8_420))]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_id: "savings".to_owned(),
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
    fn balanced_zero_sum_is_true() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        assert!(is_balanced(&t));
    }

    #[test]
    fn one_sided_import_is_unbalanced() {
        let t = tx(vec![posting("a", "checking", Some(-5_000))]);
        assert!(!is_balanced(&t));
    }

    #[test]
    fn single_elided_leg_is_balanced() {
        let t = tx(vec![
            posting("a", "checking", Some(-5_000)),
            posting("b", "groceries", None),
        ]);
        assert!(is_balanced(&t));
    }

    #[test]
    fn two_elided_legs_is_unbalanced() {
        let t = tx(vec![
            posting("a", "checking", None),
            posting("b", "groceries", None),
        ]);
        assert!(!is_balanced(&t));
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
        assert_eq!(super::payee_initial("Coles Carlton"), 'C');
    }

    #[test]
    fn payee_initial_skips_non_alpha() {
        assert_eq!(super::payee_initial("123 Foo"), 'F');
    }

    #[test]
    fn payee_initial_empty_returns_question_mark() {
        assert_eq!(super::payee_initial(""), '?');
    }

    #[test]
    fn format_date_display_standard() {
        assert_eq!(
            super::format_date_display(jiff::civil::Date::constant(2026, 4, 30)),
            "04/30"
        );
    }

    #[test]
    fn account_headline_infers_elided_focal_leg() {
        let t = tx(vec![
            posting("a", "groceries", Some(8_420)),
            posting("b", "checking", None), // elided focal leg
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_id: "checking".to_owned(),
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
                account_id: "checking".to_owned(),
            },
        );
        assert_eq!(amt.currency_code, "");
    }

    #[test]
    fn budget_headline_infers_elided_focal_leg() {
        let t = tx(vec![
            posting("a", "groceries", Some(8_420)),
            posting("b", "checking", None),
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

    fn sample_split_tx() -> Transaction {
        tx(vec![
            posting("p0", "checking", Some(-8_420)),
            posting("p1", "groceries", Some(4_210)),
            posting("p2", "dining", Some(4_210)),
        ])
    }

    #[test]
    fn strict_render_keeps_all_legs_in_lenient() {
        let tx = sample_split_tx();
        let out = super::strict_render_tx(
            &tx,
            Some(&["p0".to_owned()]),
            crate::filter_ctx::Strictness::Lenient,
        );
        assert_eq!(out.postings.len(), tx.postings.len());
    }

    #[test]
    fn strict_render_hides_unmatched_legs_in_strict() {
        let tx = sample_split_tx();
        let out = super::strict_render_tx(
            &tx,
            Some(&["p0".to_owned(), "p1".to_owned()]),
            crate::filter_ctx::Strictness::Strict,
        );
        let ids: Vec<_> = out.postings.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids, vec!["p0".to_owned(), "p1".to_owned()]);
    }

    #[test]
    fn strict_render_with_none_matched_is_identity() {
        let tx = sample_split_tx();
        let out = super::strict_render_tx(&tx, None, crate::filter_ctx::Strictness::Strict);
        assert_eq!(out.postings.len(), tx.postings.len());
    }
}
