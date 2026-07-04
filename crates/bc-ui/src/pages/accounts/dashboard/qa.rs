//! QA page for [`super::AccountDashboard`].

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::AccountDashboard;

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
            "institution:commbank".to_owned(),
            "type:transactional".to_owned(),
        ],
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
        vec!["type:credit".to_owned()],
    )
}

/// Constructs an account node with no mask, no tags, no parent.
fn no_mask_node() -> AccountNode {
    AccountNode::new(
        "macquarie",
        "Macquarie",
        None::<&str>,
        Some(Amount::new(Decimal::new(14_210_000, 2), "AUD")),
        None::<&str>,
        AccountType::Asset,
        vec![],
    )
}

/// Renders [`AccountDashboard`] in three account configurations.
///
/// # Period coverage
///
/// The sparkline granularity now follows the page `period_window` prop (there
/// are no per-sparkline controls). The first two sections render Monthly; the
/// third renders Calendar year to exercise the "Last 12 Months" title path.
///
/// Note: `stats_resource` always resolves to an error in this QA harness
/// because there is no real IPC connection. The stat cards and balance headline
/// will show "—" (the fallback value for a failed or pending resource). To
/// visually verify the non-error rendering (e.g. a numeric count with
/// warn/neutral tone), use the stat card QA page directly.
#[component]
pub fn AccountDashboardQa() -> impl IntoView {
    let monthly = Signal::derive(|| bc_ipc::Period::Monthly);
    let yearly = Signal::derive(|| bc_ipc::Period::CalendarYear);
    let window_start = Signal::derive(|| jiff::Zoned::now().date());

    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "asset account with mask and tags (default: monthly)"
                </p>
                <AccountDashboard
                    node=asset_node()
                    period_window=monthly
                    window_start=window_start
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "liability — negative balance (default: monthly)"
                </p>
                <AccountDashboard
                    node=liability_node()
                    period_window=monthly
                    window_start=window_start
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "no mask, no tags, no parent (calendar year)"
                </p>
                <AccountDashboard
                    node=no_mask_node()
                    period_window=yearly
                    window_start=window_start
                />
            </section>

        </div>
    }
}
