//! QA page for [`super::StickyAccountBar`].

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::StickyAccountBar;

/// Constructs a sample account node for QA display.
fn sample_node() -> AccountNode {
    AccountNode::new(
        "cb-smart-access",
        "Smart Access",
        Some("4421"),
        Some(Amount::new(Decimal::new(421_842, 2), "AUD")),
        Some("commbank"),
        AccountType::Asset,
        vec![],
        None,
        None,
    )
}

/// Constructs sample account statistics (no active filter — no real balance).
fn sample_stats() -> bc_ipc::AccountStats {
    bc_ipc::AccountStats::new(
        Amount::new(Decimal::new(120_000, 2), "AUD"),
        Amount::new(Decimal::new(43_500, 2), "AUD"),
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

/// Renders [`StickyAccountBar`] in hidden and visible states.
#[component]
pub fn StickyAccountBarQa() -> impl IntoView {
    let (visible_true, _) = signal(true);
    let (visible_false, _) = signal(false);

    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "visible — account selected"
                </p>
                <StickyAccountBar
                    node=Signal::derive(|| Some(sample_node()))
                    stats=Signal::derive(|| Some(sample_stats()))
                    visible=visible_true
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "visible — filtered, muted real balance"
                </p>
                <StickyAccountBar
                    node=Signal::derive(|| Some(sample_node()))
                    stats=Signal::derive(|| Some(sample_stats_filtered()))
                    visible=visible_true
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "hidden (max-height: 0)"
                </p>
                <StickyAccountBar
                    node=Signal::derive(|| Some(sample_node()))
                    stats=Signal::derive(|| Some(sample_stats()))
                    visible=visible_false
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "visible — no account selected"
                </p>
                <StickyAccountBar
                    node=Signal::derive(|| None)
                    stats=Signal::derive(|| None)
                    visible=visible_true
                />
            </section>

        </div>
    }
}
