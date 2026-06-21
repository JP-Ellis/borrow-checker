//! Shared, posting-aware transaction row used by the accounts and budget pages.

use std::collections::BTreeMap;

use bc_ipc::Amount;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use leptos::prelude::*;
use leptos::web_sys;
use rust_decimal::Decimal;
use stylance::import_style;

use crate::components::tag_token::TagToken;
use crate::components::toml_view::KvKey;
use crate::components::toml_view::KvKind;
use crate::components::toml_view::KvValue;
use crate::components::toml_view::TomlArraySection;
use crate::components::toml_view::TomlAuditEntry;
use crate::components::toml_view::TomlKv;
use crate::components::toml_view::TomlPosting;
use crate::components::toml_view::TomlSection;
use crate::label::category_label;
use crate::pages::budget::components::accrual_editor::AccrualEditor;

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
#[expect(
    clippy::arithmetic_side_effects,
    reason = "month() returns 1-12; minus one is 0-11 for JS Date.UTC()"
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

import_style!(style, "row.module.scss");

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
    /// are resolved through IPC it is unused for matching (see future-works).
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
            sum_focal(focal_on_account(tx, account_id).filter_map(|p| p.amount.as_ref()))
        }
        RowPerspective::Budget {
            account_id,
            window_start,
            window_end,
            ..
        } => {
            let focal: Vec<&Posting> = focal_on_account(tx, account_id)
                .filter(|p| p.amount.is_some())
                .collect();
            let currency = focal
                .first()
                .and_then(|p| p.amount.as_ref())
                .map_or("", |a| a.currency_code.as_str());
            let total: Decimal = focal
                .iter()
                .map(|p| prorated_value(p, *window_start, *window_end))
                .sum();
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
/// unbalanced pills, and a chevron. The expanded body is wired in Task 4.
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
    /// Called when the transaction is mutated; consumed by the expanded view in Task 4.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
) -> impl IntoView {
    let local_expanded = RwSignal::new(false);
    let expanded: Signal<bool> = expanded.unwrap_or_else(|| local_expanded.into());
    let toggle = move || match on_toggle {
        Some(cb) => cb.run(()),
        None => local_expanded.update(|e| *e = !*e),
    };

    let amount = headline_amount(&tx, &perspective);
    let currency = bc_ipc::currency_from_code(&amount.currency_code).unwrap_or(&bc_ipc::USD);
    let amount_str = if amount.currency_code.is_empty() {
        "\u{2014}".to_owned()
    } else {
        crate::components::num::format_amount(&amount.value, currency)
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
    let counterpart_names: Vec<&str> = tx
        .postings
        .iter()
        .filter(|p| focal_id.as_deref() != Some(p.account.id.as_str()))
        .map(|p| p.account.name.as_str())
        .collect();
    let category = category_label(&counterpart_names);

    let tags = tx.tags.clone();
    let tags_mobile = tags.clone();
    let split = tx.postings.len() > 2;
    let unbalanced = !is_balanced(&tx);
    let flagged = tx.reconciliation == bc_ipc::Reconciliation::Flagged;
    let unrec = tx.reconciliation == bc_ipc::Reconciliation::Unreconciled;
    let split_count = tx.postings.len();

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
            move || {
                expanded
                    .get()
                    .then(|| {
                        view! { <TransactionDetail tx=tx_detail.clone() on_change=on_change_cb /> }
                    })
            }
        }
    }
}

/// Per-posting display data for the expanded postings card.
#[derive(Clone)]
struct PostingData {
    /// Account display path, e.g. `"Assets :: Checking"`.
    account_path: String,
    /// Concrete amount, or `None` for an elided (auto-balancing) leg.
    amount: Option<Amount>,
    /// Optional inline note.
    note: Option<String>,
    /// Tag IDs attached to this posting.
    tag_ids: Vec<String>,
    /// Accrual spread start date, if set.
    spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date, if set.
    spread_until: Option<jiff::civil::Date>,
    /// Stable posting identifier.
    posting_id: String,
    /// Toggles the inline [`AccrualEditor`] for this posting.
    show_editor: RwSignal<bool>,
}

/// Renders a single posting: amount/`auto` line, chips row, and the optional
/// retained [`AccrualEditor`].
fn render_posting(p: PostingData, on_change_cb: Callback<()>) -> impl IntoView {
    let PostingData {
        account_path,
        amount,
        note,
        tag_ids,
        spread_from,
        spread_until,
        posting_id,
        show_editor,
    } = p;
    let has_spread = spread_from.is_some() && spread_until.is_some();

    let amount_view = match amount {
        Some(a) => view! { <TomlPosting amount=a>{account_path.clone()}</TomlPosting> }.into_any(),
        None => view! {
            <div class=style::posting_chips>
                <span class=style::posting_acct>{account_path.clone()}</span>
                <span class=style::amt_auto>"auto"</span>
            </div>
        }
        .into_any(),
    };

    let note_chip = note
        .filter(|n| !n.is_empty())
        .map(|n| view! { <span class=style::chip_note>"# "{n}</span> });
    let tag_chips = tag_ids
        .into_iter()
        .map(|t| view! { <TagToken label=t /> })
        .collect::<Vec<_>>();
    let spread_chip = spread_from.zip(spread_until).map(|(from, until)| {
        view! {
            <span class=style::chip_spread>
                "\u{27F3} "{from.to_string()}" \u{2192} "{until.to_string()}
            </span>
        }
    });

    let editor = move || {
        show_editor.get().then(|| {
            view! {
                <AccrualEditor
                    posting_id=posting_id.clone()
                    has_spread=has_spread
                    spread_from=spread_from
                    spread_until=spread_until
                    on_change=on_change_cb
                />
            }
        })
    };

    view! {
        {amount_view}
        <div class=style::posting_chips>
            {note_chip} {tag_chips} {spread_chip}
            <button class=style::spread_edit_btn on:click=move |_| show_editor.update(|v| *v = !*v)>
                "edit spread"
            </button>
        </div>
        {editor}
    }
}

/// Inline expanded detail panel shown below an expanded [`TransactionRow`].
///
/// Renders a meta card (key/value rows, empty fields omitted) and a postings
/// card (account path, concrete amount or an `auto` token for elided legs, plus
/// note/tag/spread chips and the retained [`AccrualEditor`]) built from the
/// `toml_view` primitives, followed by a trimmed actions row. Press `a` to
/// toggle the audit log.
///
/// # Arguments
///
/// * `tx` - The transaction to render.
/// * `on_change` - Optional callback run after a successful mutation (reverse or
///   spread edit); defaults to a no-op when `None`.
#[component]
fn TransactionDetail(
    /// The transaction to render.
    tx: Transaction,
    /// Called after a successful mutation; defaults to a no-op when `None`.
    #[prop(optional)]
    on_change: Option<Callback<()>>,
) -> impl IntoView {
    let show_audit = RwSignal::new(false);
    let detail_ref = NodeRef::<leptos::html::Div>::new();
    let on_change_cb = on_change.unwrap_or_else(|| Callback::new(|()| {}));

    let stored_tx = StoredValue::new(tx);

    Effect::new(move |_| {
        if let Some(el) = detail_ref.get() {
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "focus() returns Result<(), JsValue>; errors are benign in this context"
            )]
            let _ = el.focus();
        }
    });

    let on_action_key = move |e: web_sys::KeyboardEvent| {
        if e.key() == "a" {
            show_audit.update(|v| *v = !*v);
            e.prevent_default();
        }
    };

    /* Meta-card scalars, captured as Copy StoredValues so Fn closures can read
    them repeatedly without consuming the transaction. */
    let tx_id = StoredValue::new(stored_tx.with_value(|t| t.id.clone()));
    let tx_date = StoredValue::new(stored_tx.with_value(|t| t.date));
    let tx_extra_dates = StoredValue::new(stored_tx.with_value(|t| {
        t.extra_dates
            .iter()
            .map(|(label, d)| format!("{label} {d}"))
            .collect::<Vec<_>>()
            .join("  ")
    }));
    let tx_status = StoredValue::new(stored_tx.with_value(|t| t.reconciliation.label().to_owned()));
    let tx_payee = StoredValue::new(stored_tx.with_value(|t| t.payee.clone()));
    let tx_has_payee = stored_tx.with_value(|t| !t.payee.is_empty());
    let tx_desc = StoredValue::new(stored_tx.with_value(|t| t.description.clone()));
    let tx_has_desc = stored_tx.with_value(|t| !t.description.is_empty());
    let tx_note = StoredValue::new(stored_tx.with_value(|t| t.note.clone().unwrap_or_default()));
    let tx_has_note = stored_tx.with_value(|t| t.note.as_ref().is_some_and(|n| !n.is_empty()));
    let stored_tags = StoredValue::new(stored_tx.with_value(|t| t.tags.clone()));
    let tx_has_tags = stored_tx.with_value(|t| !t.tags.is_empty());

    let stored_audit = StoredValue::new(stored_tx.with_value(|t| {
        t.audit
            .iter()
            .map(|e| (e.time_label(), e.kind.clone(), e.message.clone()))
            .collect::<Vec<_>>()
    }));

    /* Posting display data, with one Copy toggle signal per posting driving the
    retained AccrualEditor. Stored as plain data so the audit-toggle closure
    can rebuild the postings card without consuming non-Clone views. */
    let stored_postings = StoredValue::new(stored_tx.with_value(|t| {
        t.postings
            .iter()
            .map(|p| PostingData {
                account_path: p.account.name.clone(),
                amount: p.amount.clone(),
                note: p.note.clone(),
                tag_ids: p.tag_ids.clone(),
                spread_from: p.spread_from,
                spread_until: p.spread_until,
                posting_id: p.id.clone(),
                show_editor: RwSignal::new(false),
            })
            .collect::<Vec<_>>()
    }));

    let tx_id_reverse = stored_tx.with_value(|t| t.id.clone());
    let do_reverse = move |()| {
        let id = tx_id_reverse.clone();
        leptos::task::spawn_local(async move {
            if bc_ipc::client::reverse_transaction(&id).await.is_ok() {
                on_change_cb.run(());
            }
        });
    };

    view! {
        <div class=style::detail node_ref=detail_ref on:keydown=on_action_key tabindex="-1">
            <div class=style::data_panel>
                {move || {
                    if show_audit.get() {
                        view! {
                            <TomlArraySection comment=tx_id
                                .get_value()>"audit_log"</TomlArraySection>
                            {stored_audit
                                .with_value(|entries| {
                                    entries
                                        .iter()
                                        .map(|(time, kind, msg)| {
                                            let msg = msg.clone();
                                            view! {
                                                <TomlAuditEntry time=time.clone() kind=kind.clone()>
                                                    {msg}
                                                </TomlAuditEntry>
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                })}
                        }
                            .into_any()
                    } else {
                        view! {
                            <TomlSection>"transaction"</TomlSection>
                            <TomlKv>
                                <KvKey slot>"id"</KvKey>
                                <KvValue slot kind=KvKind::Str>
                                    {tx_id.get_value()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv comment=tx_extra_dates.get_value()>
                                <KvKey slot>"date"</KvKey>
                                <KvValue slot kind=KvKind::Date>
                                    {tx_date.get_value().to_string()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"status"</KvKey>
                                <KvValue slot kind=KvKind::Keyword>
                                    {tx_status.get_value()}
                                </KvValue>
                            </TomlKv>
                            {tx_has_payee
                                .then(|| {
                                    view! {
                                        <TomlKv>
                                            <KvKey slot>"payee"</KvKey>
                                            <KvValue slot kind=KvKind::Str>
                                                {tx_payee.get_value()}
                                            </KvValue>
                                        </TomlKv>
                                    }
                                })}
                            {tx_has_desc
                                .then(|| {
                                    view! {
                                        <TomlKv>
                                            <KvKey slot>"description"</KvKey>
                                            <KvValue slot kind=KvKind::Str>
                                                {tx_desc.get_value()}
                                            </KvValue>
                                        </TomlKv>
                                    }
                                })}
                            {tx_has_note
                                .then(|| {
                                    view! {
                                        <TomlKv>
                                            <KvKey slot>"note"</KvKey>
                                            <KvValue slot kind=KvKind::Str>
                                                {tx_note.get_value()}
                                            </KvValue>
                                        </TomlKv>
                                    }
                                })}
                            {tx_has_tags
                                .then(|| {
                                    let tags = stored_tags.get_value();
                                    view! {
                                        <TomlKv>
                                            <KvKey slot>"tags"</KvKey>
                                            <KvValue slot kind=KvKind::Tags tags=tags />
                                        </TomlKv>
                                    }
                                })}
                            <TomlArraySection>"postings"</TomlArraySection>
                            {stored_postings
                                .get_value()
                                .into_iter()
                                .map(|p| render_posting(p, on_change_cb))
                                .collect::<Vec<_>>()}
                        }
                            .into_any()
                    }
                }}
            </div>

            <div class=style::actions_panel>
                <div class=style::actions_label>"actions"</div>
                <ActionBtn
                    label="edit"
                    kbd="e"
                    active=Signal::derive(|| false)
                    disabled=Signal::derive(|| true)
                />
                <ActionBtn
                    label="reverse"
                    kbd="x"
                    active=Signal::derive(|| false)
                    on_click=Callback::new(do_reverse)
                />
                <ActionBtn
                    label="find similar"
                    kbd="f"
                    active=Signal::derive(|| false)
                    disabled=Signal::derive(|| true)
                />
                <div class=style::actions_divider />
                <ActionBtn label="audit log" kbd="a" active=Signal::from(show_audit.read_only()) />
            </div>
        </div>
    }
}

/// A single action button with a keyboard shortcut hint.
///
/// # Arguments
///
/// * `label` - Button label.
/// * `kbd` - Keyboard shortcut character.
/// * `active` - Whether this action is currently active/toggled.
/// * `disabled` - Whether the button is disabled (greyed out, no click).
/// * `on_click` - Optional callback run when the button is clicked.
#[component]
fn ActionBtn(
    /// Button label.
    label: &'static str,
    /// Keyboard shortcut character.
    kbd: &'static str,
    /// Whether this action is currently active/toggled.
    #[prop(into)]
    active: Signal<bool>,
    /// Whether the button is disabled.
    #[prop(optional, into)]
    disabled: Signal<bool>,
    /// Optional callback run on click.
    #[prop(optional)]
    on_click: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <button
            class=move || {
                let mut c = vec![style::action_btn];
                if active.get() {
                    c.push(style::action_btn_active);
                }
                if disabled.get() {
                    c.push(style::action_btn_disabled);
                }
                c.join(" ")
            }
            disabled=move || disabled.get()
            on:click=move |_| {
                if let Some(cb) = on_click {
                    cb.run(());
                }
            }
        >
            {label}
            <kbd class=style::action_kbd>{kbd}</kbd>
        </button>
    }
}

#[cfg(debug_assertions)]
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
}
