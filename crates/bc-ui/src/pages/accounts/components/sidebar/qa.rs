//! QA page for [`super::AccountSidebar`].

use leptos::prelude::*;

use super::AccountSidebar;
use crate::pages::accounts::types::ACCOUNTS;

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
                        nodes=&*ACCOUNTS
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
                        nodes=&*ACCOUNTS
                        selected_id=selected_id.read_only().into()
                        collapsed=collapsed_true
                    />
                </div>
            </div>

        </div>
    }
}
