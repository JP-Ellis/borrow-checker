//! QA page for [`super::AccountDashboard`].

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::TagInfo;
use jiff::civil::date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::AccountDashboard;
use super::header::AccountHeader;

/// Constructs an asset account node for QA display.
fn asset_node() -> AccountNode {
    AccountNode::new(
        "cb-smart-access",
        "Smart Access",
        Some("4421"),
        Some(Amount::new(Decimal::new(421_842, 2), "AUD")),
        Some("commbank"),
        AccountType::Asset,
        vec![
            TagInfo::new("t-institution", "institution:commbank"),
            TagInfo::new("t-type", "type:transactional"),
        ],
        None,
        None,
    )
}

/// Constructs a liability account node with negative balance.
fn liability_node() -> AccountNode {
    AccountNode::new(
        "amex-platinum",
        "Amex Platinum",
        Some("9001"),
        Some(Amount::new(Decimal::new(-244_000, 2), "AUD")),
        None::<&str>,
        AccountType::Liability,
        vec![TagInfo::new("t-credit", "type:credit")],
        None,
        None,
    )
}

/// Constructs an account node with no mask, no tags, no parent.
fn no_mask_node() -> AccountNode {
    AccountNode::new(
        "examplebank",
        "Examplebank",
        None::<&str>,
        Some(Amount::new(Decimal::new(14_210_000, 2), "AUD")),
        None::<&str>,
        AccountType::Asset,
        vec![],
        None,
        None,
    )
}

/// The details of `node` as an open deposit account holding AUD.
fn detail_of(node: &AccountNode) -> AccountDetail {
    let mut detail = AccountDetail::builder()
        .id(node.id.clone())
        .name(node.name.clone())
        .path(node.name.clone())
        .account_type(node.account_type)
        .kind(AccountKindDto::DepositAccount)
        .commodity_codes(vec!["AUD".to_owned()])
        .tags(node.tags.clone())
        .event_count(3)
        .build();
    detail.opened_on = Some(date(2024, 7, 1));
    detail
}

/// One header state under its caption.
fn header_case(
    caption: &'static str,
    node: AccountNode,
    detail: Option<Result<AccountDetail, BcError>>,
) -> impl IntoView {
    view! {
        <section>
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">{caption}</p>
            <AccountHeader
                node
                detail=Signal::derive(move || detail.clone())
                on_change=Callback::new(|()| {})
            />
        </section>
    }
}

/// Renders [`AccountHeader`] loading, open, closed, archived and failed.
#[component]
fn HeaderStates() -> impl IntoView {
    let open = asset_node();
    let mut closed = detail_of(&open);
    closed.closed_on = Some(date(2026, 6, 30));
    let mut archived = closed.clone();
    archived.archived = true;
    view! {
        {header_case("header — loading", open.clone(), None)}
        {header_case("header — open, with metadata", open.clone(), Some(Ok(detail_of(&open))))}
        {header_case("header — closed (reopen)", open.clone(), Some(Ok(closed)))}
        {header_case("header — archived (unarchive)", open.clone(), Some(Ok(archived)))}
        {header_case(
            "header — details unavailable",
            open,
            Some(Err(BcError::Internal("details offline".to_owned()))),
        )}
    }
}

/// Constructs sample account statistics (no active filter — no real balance).
fn sample_stats() -> bc_ipc::AccountStats {
    bc_ipc::AccountStats::new(
        Amount::new(Decimal::new(120_000, 2), "AUD"),
        Amount::new(Decimal::new(43_500, 2), "AUD"),
        Amount::new(Decimal::new(25_000, 2), "AUD"),
        Amount::new(Decimal::new(76_500, 2), "AUD"),
        Amount::new(Decimal::new(345_342, 2), "AUD"),
        Amount::new(Decimal::new(421_842, 2), "AUD"),
        12,
    )
}

/// Constructs sample account statistics with a muted real closing (filter active).
fn sample_stats_filtered() -> bc_ipc::AccountStats {
    sample_stats().with_real_balances(
        Amount::new(Decimal::new(345_342, 2), "AUD"),
        Amount::new(Decimal::new(421_842, 2), "AUD"),
    )
}

/// Renders [`AccountDashboard`] in three account configurations.
///
/// # Period coverage
///
/// The sparkline granularity now follows the page `window` prop (there are no
/// per-sparkline controls). The first two sections render Monthly; the third
/// renders Calendar year to exercise the "Last 12 Months" title path.
///
/// Note: the sparkline resource always resolves to an error in this QA harness
/// because there is no real IPC connection, so the trend chart renders empty.
/// `stats` is supplied directly (no resource) so the balance headline and stat
/// cards render sample data; the second section also exercises the muted
/// real-balance path.
#[component]
pub fn AccountDashboardQa() -> impl IntoView {
    let monthly = Signal::derive(|| crate::components::period_nav::DisplayWindow::Period {
        period: bc_ipc::Period::Monthly,
        start: crate::components::period_nav::window_containing(
            &bc_ipc::Period::Monthly,
            jiff::Zoned::now().date(),
        ),
    });
    let yearly = Signal::derive(|| crate::components::period_nav::DisplayWindow::Period {
        period: bc_ipc::Period::CalendarYear,
        start: crate::components::period_nav::window_containing(
            &bc_ipc::Period::CalendarYear,
            jiff::Zoned::now().date(),
        ),
    });

    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px">
            <HeaderStates />
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "asset account with mask and tags (default: monthly)"
                </p>
                <AccountDashboard
                    node=asset_node()
                    has_children=false
                    stats=Signal::derive(|| Some(sample_stats()))
                    window=monthly
                />
            </section>
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "liability — negative balance, filtered (muted real balance)"
                </p>
                <AccountDashboard
                    node=liability_node()
                    has_children=false
                    stats=Signal::derive(|| Some(sample_stats_filtered()))
                    window=monthly
                />
            </section>
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "no mask, no tags, no parent (calendar year)"
                </p>
                <AccountDashboard
                    node=no_mask_node()
                    has_children=true
                    stats=Signal::derive(|| None)
                    window=yearly
                />
            </section>

        </div>
    }
}
