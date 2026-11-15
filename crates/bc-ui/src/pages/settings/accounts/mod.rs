//! Account list for Settings — every account in tree order, read-only.

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
/// Ordering and display text for the account rows.
pub(crate) mod rows;

use bc_ipc::AccountDetail;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;

import_style!(style, "accounts.module.scss");

/// The Settings accounts section: loads every account and lists it.
#[component]
pub fn AccountsPanel() -> impl IntoView {
    let resource =
        LocalResource::new(move || async move { bc_ipc::client::list_account_details().await });

    view! {
        <div class=style::panel data-testid="accounts-panel">
            <h1 class=style::title>"Accounts"</h1>
            <p class=style::sub>"Every account in tree order, archived ones dimmed."</p>
            {move || match resource.get() {
                None => view! { <p class=style::empty>"// loading accounts…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("Failed to load accounts: {e}") /> }
                        .into_any()
                }
                Some(Ok(details)) => view! { <AccountsTable details /> }.into_any(),
            }}
        </div>
    }
}

/// The account table over an already-loaded list.
///
/// Separate from [`AccountsPanel`] so the QA harness renders it without IPC.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos props are owned; the list is only read here"
)]
pub fn AccountsTable(
    /// The accounts to list, in any order; the table orders them.
    details: Vec<AccountDetail>,
) -> impl IntoView {
    if details.is_empty() {
        return view! {
            <p class=style::empty data-testid="accounts-empty">
                "// no accounts yet"
            </p>
        }
        .into_any();
    }

    let body = rows::tree_order(&details)
        .into_iter()
        .map(|(depth, d)| {
            let row_class = d.archived.then_some(style::row_archived);
            let indent = format!("padding-left: calc({depth} * var(--bc-space-5))");
            let state = rows::state_label(d);
            view! {
                <tr class=row_class data-testid="account-row" data-account-id=d.id.clone()>
                    <td class=style::cell_path>
                        <span class=style::name style=indent>
                            {d.name.clone()}
                        </span>
                    </td>
                    <td>{rows::kind_label(d.kind)}</td>
                    <td>{rows::commodities_text(d)}</td>
                    <td>{rows::date_text(d.opened_on)}</td>
                    <td>{rows::date_text(d.closed_on)}</td>
                    <td class=format!("{} {}", style::state, state_class(state))>{state}</td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <div class=style::table_scroll>
            <table class=style::table data-testid="accounts-table">
                <thead>
                    <tr>
                        <th>"Account"</th>
                        <th>"Kind"</th>
                        <th>"Commodities"</th>
                        <th>"Opened"</th>
                        <th>"Closed"</th>
                        <th>"State"</th>
                    </tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        </div>
    }
    .into_any()
}

/// The colour class for a state label from [`rows::state_label`].
fn state_class(state: &str) -> &'static str {
    match state {
        "archived" => style::state_archived,
        "closed" => style::state_closed,
        _ => style::state_active,
    }
}
