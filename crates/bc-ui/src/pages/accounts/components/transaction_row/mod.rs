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
use crate::pages::accounts::types::format_date_display;
use crate::pages::accounts::types::headline_amount;
use crate::pages::accounts::types::payee_initial;

import_style!(style, "row.module.scss");

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
    tx: &'static Transaction,
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
    let amount = headline_amount(tx, &viewing_account_id);
    let currency = bc_ipc::currency_from_code(&amount.currency_code).unwrap_or(&bc_ipc::USD);
    let amount_str = crate::components::num::format_amount(amount.minor_units, currency);
    let amt_class = match amount.minor_units.cmp(&0) {
        core::cmp::Ordering::Greater => style::amt_pos,
        core::cmp::Ordering::Less => style::amt_neg,
        core::cmp::Ordering::Equal => style::amt_neu,
    };

    // TODO: This column is labelled "envelope" but derives from the first counterpart
    // account's account_path, not from posting.envelope_id. When envelope_id is
    // available on postings (after bc-ipc wiring), replace with a lookup via the
    // envelope system.
    let envelope = tx
        .postings
        .iter()
        .find(|p| p.account_id != viewing_account_id.as_str())
        .map_or_else(|| "—".to_owned(), |p| p.account_path.clone());

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
                <span class=style::payee>{tx.payee.clone()}</span>
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
            <span class=style::envelope>{envelope}</span>
            <span class=format!("{} {}", style::amount, amt_class)>{amount_str}</span>
            <span class=style::chevron aria-hidden="true">
                {move || if expanded.get() { "↓" } else { "›" }}
            </span>
        </div>
        {move || expanded.get().then(|| view! { <TransactionDetail tx=tx /> })}
    }
}

/// Inline detail panel shown below an expanded row.
///
/// Displays transaction metadata and postings in TOML-like format.
/// Press `a` to toggle between postings and audit log views.
#[component]
fn TransactionDetail(
    /// The transaction to render.
    tx: &'static Transaction,
) -> impl IntoView {
    let show_audit = RwSignal::new(false);
    let detail_ref = NodeRef::<leptos::html::Div>::new();

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

    view! {
        <div class=style::detail node_ref=detail_ref on:keydown=on_action_key tabindex="-1">
            <div class=style::data_panel>
                {move || {
                    if show_audit.get() {
                        view! {
                            <TomlArraySection comment=tx.id.clone()>"audit_log"</TomlArraySection>
                            {tx
                                .audit
                                .iter()
                                .map(|e| {
                                    let msg = e.message.clone();
                                    view! {
                                        <TomlAuditEntry time=e.time.clone() kind=e.kind.clone()>
                                            {msg}
                                        </TomlAuditEntry>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        }
                            .into_any()
                    } else {
                        view! {
                            <TomlSection>"transaction"</TomlSection>
                            <TomlKv>
                                <KvKey slot>"id"</KvKey>
                                <KvValue slot kind=KvKind::Str>
                                    {tx.id.clone()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"date"</KvKey>
                                <KvValue slot kind=KvKind::Date>
                                    {tx.date.clone()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"payee"</KvKey>
                                <KvValue slot kind=KvKind::Str>
                                    {tx.payee.clone()}
                                </KvValue>
                            </TomlKv>
                            <TomlKv>
                                <KvKey slot>"status"</KvKey>
                                <KvValue slot kind=KvKind::Keyword>
                                    {tx.status.label().to_owned()}
                                </KvValue>
                            </TomlKv>
                            {(!tx.tags.is_empty())
                                .then(|| {
                                    let tags = tx.tags.clone();
                                    view! {
                                        <TomlKv>
                                            <KvKey slot>"tags"</KvKey>
                                            <KvValue slot kind=KvKind::Tags tags=tags />
                                        </TomlKv>
                                    }
                                })}
                            <TomlArraySection>"postings"</TomlArraySection>
                            {tx
                                .postings
                                .iter()
                                .map(|p| {
                                    let account_path = p.account_path.clone();
                                    let amount = p.amount.clone();
                                    if let Some(note) = p.note.clone() {
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
                                .collect::<Vec<_>>()}
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
