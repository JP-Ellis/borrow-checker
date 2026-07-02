//! Command palette (⌘K) — account search and keyboard navigation.

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountNode;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_navigate;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "palette.module.scss");

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
#[cfg(target_arch = "wasm32")]
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
    let navigate = StoredValue::new(use_navigate());
    let currencies = crate::currency_ctx::use_currency_store();

    /* Increment only when opening so closing does not reset the cached list. */
    let open_count = RwSignal::new(0_usize);
    Effect::new(move |_| {
        if open.get() {
            open_count.update(|n| *n = n.wrapping_add(1));
        }
    });
    let accounts_resource = LocalResource::new(move || async move {
        if open_count.get() == 0 {
            return Ok(vec![]);
        }
        bc_ipc::client::list_accounts().await
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
            <div class=style::overlay on:click=move |_| on_close.run(())>
                <div
                    class=style::modal
                    role="dialog"
                    aria-label="Command palette"
                    aria-modal="true"
                    on:click=move |e| e.stop_propagation()
                >
                    <input
                        node_ref=input_ref
                        class=style::input
                        type="text"
                        role="combobox"
                        placeholder="Search accounts…"
                        aria-label="Search accounts"
                        aria-expanded=move || open.get()
                        aria-controls="palette-listbox"
                        aria-activedescendant=move || {
                            let items = filtered.get();
                            if items.is_empty() {
                                String::new()
                            } else {
                                let idx = selected_idx.get();
                                items
                                    .get(idx)
                                    .map(|n| format!("palette-option-{}", n.id))
                                    .unwrap_or_default()
                            }
                        }
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
                    <div
                        id="palette-listbox"
                        class=style::list
                        role="listbox"
                        aria-label="Accounts"
                    >
                        {move || {
                            match accounts_resource.get() {
                                None => {
                                    return view! { <div class=style::empty>"Loading…"</div> }
                                        .into_any();
                                }
                                Some(Err(_)) => {
                                    return view! {
                                        <div class=style::empty>"Failed to load accounts."</div>
                                    }
                                        .into_any();
                                }
                                Some(Ok(_)) => {}
                            }
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
                                let currency_set = currencies.get();
                                items
                                    .into_iter()
                                    .enumerate()
                                    .map(|(idx, node)| {
                                        let option_id = format!("palette-option-{}", node.id);
                                        let path = format!("/accounts/{}", node.id);
                                        let nav = navigate.get_value();
                                        let close = on_close;
                                        let balance = node
                                            .balance
                                            .as_ref()
                                            .map_or_else(
                                                || "\u{2014}".to_owned(),
                                                |b| {
                                                    let (sym, after) = crate::currency_ctx::short_symbol(
                                                        &b.currency_code,
                                                        &currency_set,
                                                    );
                                                    b.format_short(sym.as_deref(), after)
                                                },
                                            );
                                        let type_label = node.account_type.label();
                                        let item_class = if idx == sel {
                                            format!("{} {}", style::item, style::item_selected)
                                        } else {
                                            style::item.to_owned()
                                        };
                                        view! {
                                            <div
                                                id=option_id
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

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn selected_idx_clamping_arrow_down_at_last() {
        /* ArrowDown at the last item (idx 2, count 3) stays at 2. */
        let i = 2_usize;
        let count = 3_usize;
        let next = i.saturating_add(1).min(count.saturating_sub(1));
        assert_eq!(next, 2);
    }

    #[test]
    fn selected_idx_clamping_arrow_up_at_first() {
        /* ArrowUp at the first item stays at 0. */
        let i = 0_usize;
        let prev = i.saturating_sub(1);
        assert_eq!(prev, 0);
    }

    #[test]
    fn selected_idx_clamping_empty_list_arrow_down() {
        /* ArrowDown on an empty list — guarded by count > 0 check — is a no-op. */
        let count = 0_usize;
        let i = 0_usize;
        if count > 0 {
            let _next: usize = i.saturating_add(1).min(count.saturating_sub(1));
            panic!("should not reach here when count == 0");
        }
        assert_eq!(i, 0);
    }

    #[test]
    fn selected_idx_clamping_single_item_arrow_down() {
        /* ArrowDown with one item stays at 0. */
        let i = 0_usize;
        let count = 1_usize;
        let next = i.saturating_add(1).min(count.saturating_sub(1));
        assert_eq!(next, 0);
    }
}
