//! QA page for [`super::AccountDashboard`].

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;

use super::AccountDashboard;

/// Constructs an asset account node for QA display.
fn asset_node() -> AccountNode {
    AccountNode::new(
        "cb-smart-access",
        "Smart Access",
        Some("4421"),
        Amount::new(421_842, "AUD", 2),
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
        Amount::new(-244_000, "AUD", 2),
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
        Amount::new(14_210_000, "AUD", 2),
        None::<&str>,
        AccountType::Asset,
        vec![],
    )
}

/// Renders [`AccountDashboard`] in three account configurations.
///
/// # Period toggle coverage
///
/// All three sections start with the default period (Monthly). To exercise the
/// period toggle interactively, click the weekly/monthly/quarterly buttons in
/// any section. The sparkline will re-fetch with 8 buckets for Weekly and 6
/// buckets for all other periods.
///
/// CalendarYear and FinancialYear variants are not yet exposed in the toggle UI
/// (see the `#[expect(clippy::wildcard_enum_match_arm)]` comment in `mod.rs`).
#[component]
pub fn AccountDashboardQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "asset account with mask and tags (default: monthly)"
                </p>
                <AccountDashboard node=asset_node() />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "liability — negative balance (default: monthly)"
                </p>
                <AccountDashboard node=liability_node() />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "no mask, no tags, no parent (default: monthly)"
                </p>
                <AccountDashboard node=no_mask_node() />
            </section>

        </div>
    }
}
