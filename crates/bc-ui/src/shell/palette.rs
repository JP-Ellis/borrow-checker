//! Command palette (⌘K) — account search and keyboard navigation.

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use leptos::prelude::*;
use leptos::web_sys;
use leptos_router::hooks::use_navigate;
use stylance::import_style;

use crate::pages::accounts::components::sidebar::format_balance_short;

import_style!(style, "palette.module.scss");

/// Returns the display label for an [`AccountType`].
fn account_type_label(ty: AccountType) -> &'static str {
    match ty {
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
        AccountType::Equity => "equity",
        AccountType::Income => "income",
        AccountType::Expense => "expense",
        _ => "account",
    }
}

/// Command palette modal triggered by ⌘K.
///
/// Renders a full-screen overlay with a search input that filters accounts by
/// name. Keyboard navigation (Arrow keys, Enter, Escape) and click-to-navigate
/// are supported. The palette auto-fetches accounts when opened and closes on
/// navigation or Escape.
///
/// # Arguments
///
/// * `open` - Read signal controlling whether the palette is visible.
/// * `on_close` - Callback invoked when the palette should close.
#[component]
#[expect(clippy::too_many_lines, reason = "Leptos view! block")]
pub fn CommandPalette(
    /// Whether the palette is visible.
    open: ReadSignal<bool>,
    /// Called when the palette should close (Escape, backdrop click, navigation).
    on_close: Callback<()>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let selected_idx = RwSignal::new(0_usize);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    /* StoredValue allows the navigate fn to be copied into multiple closures. */
    let navigate = StoredValue::new(use_navigate());

    /* Re-fetch every time the palette opens so results stay current. */
    let accounts_resource = LocalResource::new(move || {
        open.get(); /* subscribes so resource refreshes each open */
        bc_ipc::client::list_accounts()
    });

    /* Filtered list — recomputes when query or resource changes. */
    let filtered = Memo::new(move |_| {
        let q = query.get();
        let q_lower = q.to_lowercase();
        accounts_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q_lower))
            .collect::<Vec<AccountNode>>()
    });

    /* Autofocus the input and reset state whenever the palette opens. */
    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            selected_idx.set(0);
            if let Some(el) = input_ref.get() {
                #[expect(
                    clippy::let_underscore_must_use,
                    clippy::let_underscore_untyped,
                    let_underscore_drop,
                    reason = "focus() returns Result<(), JsValue>; errors are benign"
                )]
                let _ = el.focus();
            }
        }
    });

    view! {
        <Show when=move || open.get()>
            <div
                class=style::overlay
                on:click=move |_| on_close.run(())
                role="dialog"
                aria-label="Command palette"
                aria-modal="true"
            >
                <div class=style::modal on:click=move |e| e.stop_propagation()>
                    <input
                        node_ref=input_ref
                        class=style::input
                        type="text"
                        placeholder="Search accounts…"
                        aria-label="Search accounts"
                        prop:value=move || query.get()
                        on:input=move |e| {
                            let val = event_target_value(&e);
                            query.set(val);
                            selected_idx.set(0);
                        }
                        on:keydown=move |e: web_sys::KeyboardEvent| {
                            match e.key().as_str() {
                                "Escape" => {
                                    on_close.run(());
                                    e.prevent_default();
                                }
                                "ArrowDown" => {
                                    let count = filtered.get().len();
                                    if count > 0 {
                                        selected_idx
                                            .update(|i| {
                                                *i = i.saturating_add(1).min(count.saturating_sub(1));
                                            });
                                    }
                                    e.prevent_default();
                                }
                                "ArrowUp" => {
                                    selected_idx
                                        .update(|i| {
                                            *i = i.saturating_sub(1);
                                        });
                                    e.prevent_default();
                                }
                                "Enter" => {
                                    let items = filtered.get();
                                    if let Some(node) = items.get(selected_idx.get()) {
                                        let path = format!("/accounts/{}", node.id);
                                        navigate
                                            .get_value()(
                                            &path,
                                            leptos_router::NavigateOptions::default(),
                                        );
                                        on_close.run(());
                                    }
                                    e.prevent_default();
                                }
                                _ => {}
                            }
                        }
                    />
                    <div class=style::list role="listbox">
                        {move || {
                            let items = filtered.get();
                            if items.is_empty() {
                                let q = query.get();
                                let msg = if q.is_empty() {
                                    "no accounts found".to_owned()
                                } else {
                                    format!("no accounts match \"{q}\"")
                                };
                                view! { <div class=style::empty>{msg}</div> }.into_any()
                            } else {
                                let sel = selected_idx.get();
                                items
                                    .into_iter()
                                    .enumerate()
                                    .map(|(idx, node)| {
                                        let path = format!("/accounts/{}", node.id);
                                        let nav = navigate.get_value();
                                        let close = on_close;
                                        let balance = format_balance_short(&node.balance);
                                        let type_label = account_type_label(node.account_type);
                                        let item_class = if idx == sel {
                                            format!("{} {}", style::item, style::item_selected)
                                        } else {
                                            style::item.to_owned()
                                        };
                                        view! {
                                            <div
                                                class=item_class
                                                role="option"
                                                aria-selected=idx == sel
                                                on:click=move |_| {
                                                    nav(&path, leptos_router::NavigateOptions::default());
                                                    close.run(());
                                                }
                                                on:mouseenter=move |_| selected_idx.set(idx)
                                            >
                                                <span class=style::item_name>{node.name.clone()}</span>
                                                <span class=style::badge>{type_label}</span>
                                                <span class=style::balance>{balance}</span>
                                            </div>
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .into_any()
                            }
                        }}
                    </div>
                </div>
            </div>
        </Show>
    }
}
