//! QA page for [`super::AccountSidebar`].

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::AccountSidebar;

/// Returns sample account nodes for the QA showcase.
fn sample_accounts() -> Vec<AccountNode> {
    vec![
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
            None,
            None,
        ),
        AccountNode::new(
            "commbank",
            "Bank",
            None::<&str>,
            Some(Amount::new(Decimal::new(6_421_000, 2), "AUD")),
            None::<&str>,
            AccountType::Asset,
            vec![],
            None,
            None,
        ),
        AccountNode::new(
            "amex-platinum",
            "Amex Platinum",
            Some("9001"),
            Some(Amount::new(Decimal::new(-244_000, 2), "AUD")),
            None::<&str>,
            AccountType::Liability,
            vec!["type:credit".to_owned()],
            None,
            None,
        ),
    ]
}

/// Renders [`AccountSidebar`] in expanded and collapsed states.
#[component]
pub fn AccountSidebarQa() -> impl IntoView {
    let selected_id: RwSignal<Option<String>> = RwSignal::new(Some("cb-smart-access".to_owned()));
    let (collapsed, _set_collapsed) = signal(false);
    let (collapsed_true, _set_collapsed_true) = signal(true);

    view! {
        <div style="display:flex;gap:32px;padding:24px">

            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "expanded — full tree"
                </p>
                <div style="width:200px;border:1px solid var(--bc-border)">
                    <AccountSidebar
                        nodes=sample_accounts()
                        selected_id=selected_id.read_only().into()
                        collapsed=collapsed
                    />
                </div>
            </div>

            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "collapsed — dot rail"
                </p>
                <div style="width:48px;border:1px solid var(--bc-border)">
                    <AccountSidebar
                        nodes=sample_accounts()
                        selected_id=selected_id.read_only().into()
                        collapsed=collapsed_true
                    />
                </div>
            </div>

        </div>
    }
}
