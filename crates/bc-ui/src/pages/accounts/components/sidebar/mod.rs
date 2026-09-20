//! Account tree sidebar — full recursive tree and collapsed dot-rail states.

use std::collections::HashSet;

use bc_ipc::AccountNode;
use leptos::prelude::*;
use leptos_router::components::A;
use stylance::import_style;

use crate::pages::accounts::tree::children_of;
use crate::pages::accounts::tree::ordered_roots;

import_style!(style, "sidebar.module.scss");

/// `localStorage` key holding the expanded account ids as a JSON array.
pub const EXPANDED_STORAGE_KEY: &str = "bc.sidebar.expanded";

/// Reads the persisted expansion set, or `None` when absent or unreadable.
#[must_use]
pub fn load_expanded() -> Option<HashSet<String>> {
    let raw = crate::storage::get(EXPANDED_STORAGE_KEY)?;
    serde_json::from_str::<Vec<String>>(&raw)
        .ok()
        .map(|v| v.into_iter().collect())
}

/// Persists the expansion set as a JSON array.
///
/// # Arguments
///
/// * `expanded` - The ids of expanded accounts.
pub fn save_expanded(expanded: &HashSet<String>) {
    let mut ids: Vec<&String> = expanded.iter().collect();
    ids.sort();
    if let Ok(raw) = serde_json::to_string(&ids) {
        crate::storage::set(EXPANDED_STORAGE_KEY, &raw);
    }
}

/// Account tree sidebar.
///
/// In expanded state renders the full account hierarchy with balances.
/// In collapsed state renders a dot rail — one dot per account, active dot highlighted.
///
/// # Arguments
///
/// * `nodes` - All account nodes (flat vec; hierarchy via `parent_id`).
/// * `selected_id` - Currently selected account ID (derived from route).
/// * `collapsed` - Whether the sidebar is in dot-rail mode.
/// * `expanded` - Ids whose children are shown; owned by the page.
#[expect(clippy::too_many_lines, reason = "Leptos view! block")]
#[component]
pub fn AccountSidebar(
    /// All account nodes.
    nodes: Vec<AccountNode>,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
    /// Whether the sidebar is collapsed to dot-rail.
    collapsed: ReadSignal<bool>,
    /// Ids whose children are shown.
    expanded: RwSignal<HashSet<String>>,
) -> impl IntoView {
    let roots = ordered_roots(&nodes);
    let stored_nodes = StoredValue::new(nodes);
    let stored_roots = StoredValue::new(roots);

    let tree = move || {
        let all = stored_nodes.get_value();
        stored_roots
            .get_value()
            .into_iter()
            .map(|root| {
                view! {
                    <SidebarNode
                        node=root
                        nodes=all.clone()
                        depth=0
                        selected_id=selected_id
                        expanded=expanded
                    />
                }
            })
            .collect::<Vec<_>>()
    };

    view! {
        <>
            <nav class=style::nav aria-label="account navigation">
                <Show
                    when=move || collapsed.get()
                    fallback=move || view! { <div class=style::tree>{tree()}</div> }
                >
                    <div class=style::rail>
                        {move || {
                            stored_nodes
                                .with_value(|all_nodes| {
                                    all_nodes
                                        .iter()
                                        .map(|node| {
                                            let id = node.id.clone();
                                            let title = node.name.clone();
                                            let is_active = Signal::derive(move || {
                                                selected_id.get().as_deref() == Some(id.as_str())
                                            });
                                            let href = format!("/accounts/{}", node.id);
                                            view! {
                                                <A
                                                    href=href
                                                    attr:class=move || {
                                                        if is_active.get() {
                                                            format!("{} {}", style::dot, style::dot_active)
                                                        } else {
                                                            style::dot.to_owned()
                                                        }
                                                    }
                                                    attr:title=title.clone()
                                                    attr:aria-label=title
                                                >
                                                    ""
                                                </A>
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                })
                        }}
                    </div>
                </Show>
            </nav>

            <button
                class=style::mobile_trigger
                popovertarget="bc-sidebar-drawer"
                aria-label="Open account navigation"
            >
                <div class=style::rail>
                    {stored_nodes
                        .get_value()
                        .iter()
                        .map(|node| {
                            let title = node.name.clone();
                            view! { <span class=style::dot aria-hidden="true" title=title /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </button>

            <nav
                id="bc-sidebar-drawer"
                class=style::drawer
                popover="auto"
                aria-label="account navigation"
            >
                <div class=style::tree>{tree()}</div>
            </nav>
        </>
    }
}

/// One account row plus, when expanded, its children rendered recursively.
#[component]
fn SidebarNode(
    /// Account node to render.
    node: AccountNode,
    /// Full node vec (needed to find children).
    nodes: Vec<AccountNode>,
    /// Nesting depth; drives the indent.
    depth: u32,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
    /// Ids whose children are shown.
    expanded: RwSignal<HashSet<String>>,
) -> AnyView {
    let children = children_of(&nodes, &node.id);
    let has_children = !children.is_empty();
    let id = node.id.clone();
    let id_for_toggle = node.id.clone();
    let is_open = Signal::derive(move || expanded.with(|e| e.contains(&id)));
    let toggle = move |_: leptos::ev::MouseEvent| {
        expanded.update(|e| {
            if !e.remove(&id_for_toggle) {
                e.insert(id_for_toggle.clone());
            }
        });
    };
    let stored_children = StoredValue::new(children);
    let stored_nodes = StoredValue::new(nodes);
    let name = node.name.clone();

    view! {
        <div class=style::node style=format!("--depth: {depth}")>
            <div class=style::row_line>
                {if has_children {
                    view! {
                        <button
                            class=style::chevron
                            on:click=toggle
                            aria-expanded=move || if is_open.get() { "true" } else { "false" }
                            aria-label=format!("toggle {name}")
                        >
                            {move || if is_open.get() { "▾" } else { "▸" }}
                        </button>
                    }
                        .into_any()
                } else {
                    view! { <span class=style::chevron_spacer aria-hidden="true" /> }.into_any()
                }} <SidebarRow node=node selected_id=selected_id />
            </div>
            <Show when=move || {
                has_children && is_open.get()
            }>
                {move || {
                    let all = stored_nodes.get_value();
                    stored_children
                        .get_value()
                        .into_iter()
                        .map(|child| {
                            view! {
                                <SidebarNode
                                    node=child
                                    nodes=all.clone()
                                    depth=depth + 1
                                    selected_id=selected_id
                                    expanded=expanded
                                />
                            }
                        })
                        .collect::<Vec<_>>()
                }}
            </Show>
        </div>
    }
    .into_any()
}

/// A single account link with its balance figure.
#[component]
fn SidebarRow(
    /// Account node to render.
    node: AccountNode,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let include_descendants = crate::pages::accounts::rollup::use_include_descendants();
    let id = node.id.clone();
    let own = node.balance.clone();
    let rollup = node.rollup.clone();

    // (formatted figure, is_negative, extra commodity count)
    let figure = Memo::new(move |_| {
        let shown = if include_descendants.get() {
            rollup.first().cloned()
        } else {
            own.clone()
        };
        let extra = if include_descendants.get() {
            rollup.len().saturating_sub(1)
        } else {
            0
        };
        match shown {
            None => ("\u{2014}".to_owned(), false, extra),
            Some(b) => {
                let (sym, after) =
                    crate::currency_ctx::short_symbol(&b.currency_code, &currencies.get());
                (
                    b.format_short(sym.as_deref(), after),
                    b.value < rust_decimal::Decimal::ZERO,
                    extra,
                )
            }
        }
    });
    let is_active = Signal::derive(move || selected_id.get().as_deref() == Some(id.as_str()));
    let href = format!("/accounts/{}", node.id);

    view! {
        <A
            href=href
            attr:class=move || {
                if is_active.get() {
                    format!("{} {}", style::row, style::row_active)
                } else {
                    style::row.to_owned()
                }
            }
        >
            <span class=style::row_name>{node.name.clone()}</span>
            <span class=move || {
                if figure.with(|f| f.1) { style::bal_neg } else { style::bal }
            }>
                {move || figure.with(|f| f.0.clone())}
                {move || {
                    let extra = figure.with(|f| f.2);
                    (extra > 0)
                        .then(|| view! { <span class=style::badge>{format!("+{extra}")}</span> })
                }}
            </span>
        </A>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
