//! QA page for [`super::StickyAccountBar`].

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;

use super::StickyAccountBar;

/// Constructs a sample account node for QA display.
fn sample_node() -> AccountNode {
    AccountNode::new(
        "cb-smart-access",
        "Smart Access",
        Some("4421"),
        Amount::new(421_842, "AUD"),
        Some("commbank"),
        AccountType::Asset,
        vec![],
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
                    visible=visible_true
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "hidden (max-height: 0)"
                </p>
                <StickyAccountBar
                    node=Signal::derive(|| Some(sample_node()))
                    visible=visible_false
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "visible — no account selected"
                </p>
                <StickyAccountBar node=Signal::derive(|| None) visible=visible_true />
            </section>

        </div>
    }
}
