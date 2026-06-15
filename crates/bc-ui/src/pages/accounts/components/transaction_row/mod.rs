//! Transaction register row and inline expanded detail panel.

use bc_ipc::Transaction;
use leptos::prelude::*;
use leptos::web_sys;
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
use crate::pages::accounts::types::format_date_display;
use crate::pages::accounts::types::headline_amount;
use crate::pages::accounts::types::payee_initial;

import_style!(style, "row.module.scss");

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

    // Two-pass render: overflow cannot be measured before first paint.
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

/// A single register row, optionally expanded to reveal the detail panel.
///
/// # Arguments
///
/// * `tx` - The transaction to render.
/// * `viewing_account_id` - Account currently in view (determines headline amount).
/// * `selected` - Whether this row has keyboard focus.
/// * `expanded` - Whether the inline detail panel is open.
/// * `on_toggle` - Called when the row or Enter key is pressed.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos props must take String for #[prop(into)] support"
)]
pub fn TransactionRow(
    /// The transaction.
    tx: Transaction,
    /// Account ID currently being viewed (for context-relative amount).
    #[prop(into)]
    viewing_account_id: String,
    /// Whether this row is keyboard-selected.
    selected: Signal<bool>,
    /// Whether the detail panel is expanded.
    expanded: Signal<bool>,
    /// Called to toggle expansion.
    on_toggle: Callback<()>,
) -> impl IntoView {
    let date = format_date_display(&tx.date);
    let initial = payee_initial(&tx.payee).to_string();
    let amount = headline_amount(&tx, &viewing_account_id);
    let currency = bc_ipc::currency_from_code(&amount.currency_code).unwrap_or(&bc_ipc::USD);
    let amount_str = crate::components::num::format_amount(amount.minor_units, currency);
    let amt_class = match amount.minor_units.cmp(&0) {
        core::cmp::Ordering::Greater => style::amt_pos,
        core::cmp::Ordering::Less => style::amt_neg,
        core::cmp::Ordering::Equal => style::amt_neu,
    };

    let counterpart_names: Vec<&str> = tx
        .postings
        .iter()
        .filter(|p| p.account.id != viewing_account_id.as_str())
        .map(|p| p.account.name.as_str())
        .collect();
    let category = category_label(&counterpart_names);

    let payee = tx.payee.clone();
    let tags: Vec<String> = tx.tags.clone();
    let tags_mobile = tags.clone();

    let on_toggle_kd = on_toggle;

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
            on:click=move |_| on_toggle.run(())
            on:keydown=move |e: web_sys::KeyboardEvent| {
                if e.key() == " " || e.key() == "Enter" {
                    on_toggle_kd.run(());
                    e.prevent_default();
                }
            }
            role="button"
            tabindex="0"
            aria-expanded=move || expanded.get().to_string()
        >
            <span class=style::date>{date.clone()}</span>
            <div class=style::payee_cell>
                <span class=style::avatar aria-hidden="true">
                    {initial}
                </span>
                <span class=style::payee>{payee}</span>
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
            <span class=format!("{} {}", style::amount, amt_class)>{amount_str}</span>
            <span class=style::chevron aria-hidden="true">
                {move || if expanded.get() { "↓" } else { "›" }}
            </span>
        </div>
        {move || expanded.get().then(|| view! { <TransactionDetail tx=tx.clone() /> })}
    }
}

/// Inline detail panel shown below an expanded row.
///
/// Displays transaction metadata and postings in TOML-like format.
/// Press `a` to toggle between postings and audit log views.
#[component]
fn TransactionDetail(
    /// The transaction to render.
    tx: Transaction,
) -> impl IntoView {
    let show_audit = RwSignal::new(false);
    let detail_ref = NodeRef::<leptos::html::Div>::new();

    // Store the transaction in a StoredValue so reactive closures can access it
    // multiple times without consuming it (FnMut-compatible).
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

    // Extract all display data from the stored transaction into StoredValues.
    // StoredValue<T> is Copy, which allows reactive closures (Fn) to read
    // the data repeatedly without consuming it.
    let tx_id = StoredValue::new(stored_tx.with_value(|t| t.id.clone()));
    let tx_date = StoredValue::new(stored_tx.with_value(|t| t.date.clone()));
    let tx_payee = StoredValue::new(stored_tx.with_value(|t| t.payee.clone()));
    let tx_status = StoredValue::new(stored_tx.with_value(|t| t.status.label().to_owned()));
    let stored_tags = StoredValue::new(stored_tx.with_value(|t| t.tags.clone()));
    let tx_has_tags = stored_tx.with_value(|t| !t.tags.is_empty());
    let stored_audit = StoredValue::new(stored_tx.with_value(|t| {
        t.audit
            .iter()
            .map(|e| (e.time.clone(), e.kind.clone(), e.message.clone()))
            .collect::<Vec<_>>()
    }));
    let stored_postings = StoredValue::new(stored_tx.with_value(|t| {
        t.postings
            .iter()
            .map(|p| (p.account.name.clone(), p.amount.clone(), p.note.clone()))
            .collect::<Vec<_>>()
    }));

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
                            <TomlKv>
                                <KvKey slot>"date"</KvKey>
                                <KvValue slot kind=KvKind::Date>
                                    {tx_date.get_value()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"payee"</KvKey>
                                <KvValue slot kind=KvKind::Str>
                                    {tx_payee.get_value()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"status"</KvKey>
                                <KvValue slot kind=KvKind::Keyword>
                                    {tx_status.get_value()}
                                </KvValue>
                            </TomlKv>
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
                                .with_value(|items| {
                                    items
                                        .iter()
                                        .map(|(account_path, amount, note)| {
                                            let account_path = account_path.clone();
                                            let amount = amount.clone();
                                            if let Some(note) = note.clone() {
                                                view! {
                                                    <TomlPosting amount=amount.clone() note=note>
                                                        {account_path}
                                                    </TomlPosting>
                                                }
                                                    .into_any()
                                            } else {
                                                view! {
                                                    <TomlPosting amount=amount>{account_path}</TomlPosting>
                                                }
                                                    .into_any()
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                })}
                        }
                            .into_any()
                    }
                }}
            </div>

            <div class=style::actions_panel>
                <div class=style::actions_label>"actions"</div>
                <ActionBtn label="recategorise" kbd="c" active=Signal::derive(|| false) />
                <ActionBtn label="split" kbd="s" active=Signal::derive(|| false) />
                <ActionBtn label="mark shared" kbd="#" active=Signal::derive(|| false) />
                <ActionBtn label="add note" kbd="n" active=Signal::derive(|| false) />
                <ActionBtn label="find similar" kbd="f" active=Signal::derive(|| false) />
                <ActionBtn label="create rule" kbd="r" active=Signal::derive(|| false) />
                <div class=style::actions_divider />
                <ActionBtn label="audit log" kbd="a" active=Signal::from(show_audit.read_only()) />
            </div>
        </div>
    }
}

/// A single action button with a keyboard shortcut hint.
#[component]
fn ActionBtn(
    /// Button label.
    label: &'static str,
    /// Keyboard shortcut character.
    kbd: &'static str,
    /// Whether this action is currently active/toggled.
    #[prop(into)]
    active: Signal<bool>,
) -> impl IntoView {
    view! {
        <button class=move || {
            if active.get() {
                format!("{} {}", style::action_btn, style::action_btn_active)
            } else {
                style::action_btn.to_owned()
            }
        }>{label} <kbd class=style::action_kbd>{kbd}</kbd></button>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
