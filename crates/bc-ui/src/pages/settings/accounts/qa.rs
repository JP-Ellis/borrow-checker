//! QA showcase for [`super::AccountsTable`] — mock accounts, no IPC.

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountType;
use jiff::civil::date;
use leptos::prelude::*;

use super::AccountsTable;

/// Builds one mock account; `path` is the ":"-joined path and the last
/// segment is the name.
fn mock(
    id: &str,
    path: &str,
    parent: Option<&str>,
    account_type: AccountType,
    kind: AccountKindDto,
    commodities: &[&str],
) -> AccountDetail {
    let name = path.rsplit(':').next().unwrap_or(path);
    AccountDetail::builder()
        .id(id)
        .name(name)
        .path(path)
        .maybe_parent_id(parent.map(str::to_owned))
        .account_type(account_type)
        .kind(kind)
        .commodity_codes(commodities.iter().map(|c| (*c).to_owned()).collect())
        .tags(vec![])
        .opened_on(date(2024, 1, 1))
        .event_count(0)
        .build()
}

/// Mock set: nested assets, a closed account, an archived account and an
/// expense branch, listed out of tree order.
fn mock_accounts() -> Vec<AccountDetail> {
    let mut closed = mock(
        "old",
        "Assets:Bank:Old",
        Some("bank"),
        AccountType::Asset,
        AccountKindDto::DepositAccount,
        &["AUD"],
    );
    closed.closed_on = Some(date(2025, 6, 30));
    let mut archived = mock(
        "legacy",
        "Assets:Bank:Legacy",
        Some("bank"),
        AccountType::Asset,
        AccountKindDto::DepositAccount,
        &["USD", "AUD"],
    );
    archived.closed_on = Some(date(2023, 12, 31));
    archived.archived = true;
    vec![
        mock(
            "food",
            "Expenses:Food",
            Some("expenses"),
            AccountType::Expense,
            AccountKindDto::Group,
            &[],
        ),
        closed,
        mock(
            "expenses",
            "Expenses",
            None,
            AccountType::Expense,
            AccountKindDto::Group,
            &[],
        ),
        mock(
            "cheque",
            "Assets:Bank:Cheque",
            Some("bank"),
            AccountType::Asset,
            AccountKindDto::DepositAccount,
            &["AUD"],
        ),
        archived,
        mock(
            "assets",
            "Assets",
            None,
            AccountType::Asset,
            AccountKindDto::Group,
            &[],
        ),
        mock(
            "bank",
            "Assets:Bank",
            Some("assets"),
            AccountType::Asset,
            AccountKindDto::Group,
            &[],
        ),
    ]
}

/// Renders [`AccountsTable`] over the mock set, then over an empty list.
#[component]
pub fn AccountsPanelQa() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <AccountsTable details=mock_accounts() />
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin:24px 0 8px;">"Empty"</h2>
            <AccountsTable details=vec![] />
        </div>
    }
}
