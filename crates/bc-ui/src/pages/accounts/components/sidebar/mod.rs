//! Account tree sidebar — full tree and collapsed dot-rail states.

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use leptos::prelude::*;
use leptos_router::components::A;
use stylance::import_style;

import_style!(style, "sidebar.module.scss");

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
#[expect(clippy::too_many_lines, reason = "Leptos view! block")]
#[component]
pub fn AccountSidebar(
    /// All account nodes.
    nodes: Vec<AccountNode>,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
    /// Whether the sidebar is collapsed to dot-rail.
    collapsed: ReadSignal<bool>,
) -> impl IntoView {
    let all_types = [
        (AccountType::Asset, "assets"),
        (AccountType::Liability, "liabilities"),
        (AccountType::Equity, "equity"),
        (AccountType::Income, "income"),
        (AccountType::Expense, "expenses"),
    ];

    let sections: Vec<(AccountType, &'static str, Vec<AccountNode>)> = all_types
        .into_iter()
        .filter_map(|(ty, label)| {
            let roots: Vec<AccountNode> = nodes
                .iter()
                .filter(|n| n.account_type == ty && n.parent_id.is_none())
                .cloned()
                .collect();
            if roots.is_empty() {
                None
            } else {
                Some((ty, label, roots))
            }
        })
        .collect();

    // Use StoredValue so the vecs can be retrieved from reactive closures
    // (Leptos Show/fallback children require Fn, not FnOnce).
    let stored_nodes = StoredValue::new(nodes);
    let stored_sections = StoredValue::new(sections);

    view! {
        <>
            // Desktop sidebar — hidden on mobile via CSS
            <nav class=style::nav aria-label="account navigation">
                <Show
                    when=move || collapsed.get()
                    fallback=move || {
                        view! {
                            <div class=style::tree>
                                {stored_sections
                                    .with_value(|secs| {
                                        let all_nodes = stored_nodes.get_value();
                                        secs.iter()
                                            .map(|(_, label, roots)| {
                                                view! {
                                                    <SidebarSection
                                                        label=label
                                                        nodes=all_nodes.clone()
                                                        roots=roots.clone()
                                                        selected_id=selected_id
                                                    />
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                    })}
                            </div>
                        }
                    }
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

            // Mobile: dot-rail trigger button — shown below bp-md
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

            // Mobile: full sidebar as a popover overlay
            <nav
                id="bc-sidebar-drawer"
                class=style::drawer
                popover="auto"
                aria-label="account navigation"
            >
                <div class=style::tree>
                    {stored_sections
                        .with_value(|secs| {
                            secs.iter()
                                .map(|(_, label, roots)| {
                                    view! {
                                        <SidebarSection
                                            label=label
                                            nodes=stored_nodes.get_value()
                                            roots=roots.clone()
                                            selected_id=selected_id
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()
                        })}
                </div>
            </nav>
        </>
    }
}

/// Renders one account type section of the account tree.
#[component]
fn SidebarSection(
    /// Section label shown as eyebrow.
    label: &'static str,
    /// Full node vec (needed to find children).
    nodes: Vec<AccountNode>,
    /// Top-level nodes for this section.
    roots: Vec<AccountNode>,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class=style::section>
            <div class=style::section_label>{label}</div>
            {roots
                .into_iter()
                .map(|root| {
                    let children: Vec<AccountNode> = nodes
                        .iter()
                        .filter(|n| n.parent_id.as_deref() == Some(root.id.as_str()))
                        .cloned()
                        .collect();
                    view! {
                        <SidebarRow node=root.clone() selected_id=selected_id indent=false />
                        {children
                            .into_iter()
                            .map(|child| {
                                view! {
                                    <SidebarRow node=child selected_id=selected_id indent=true />
                                }
                            })
                            .collect::<Vec<_>>()}
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

/// A single row in the account tree.
#[component]
fn SidebarRow(
    /// Account node to render.
    node: AccountNode,
    /// Currently selected account ID.
    selected_id: Signal<Option<String>>,
    /// Whether this row is indented (child account).
    indent: bool,
) -> impl IntoView {
    let id = node.id.clone();
    let balance = node
        .balance
        .as_ref()
        .map_or_else(|| "\u{2014}".into(), bc_ipc::Amount::format_short);
    let is_active = Signal::derive(move || selected_id.get().as_deref() == Some(id.as_str()));
    let balance_class = if node.balance.as_ref().is_some_and(|b| b.minor_units < 0) {
        style::bal_neg
    } else {
        style::bal
    };
    let href = format!("/accounts/{}", node.id);

    view! {
        <A
            href=href
            attr:class=move || {
                let mut cls = vec![style::row];
                if indent {
                    cls.push(style::row_indent);
                }
                if is_active.get() {
                    cls.push(style::row_active);
                }
                cls.join(" ")
            }
        >
            <span class=style::row_name>{node.name.clone()}</span>
            <span class=balance_class>{balance}</span>
        </A>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
