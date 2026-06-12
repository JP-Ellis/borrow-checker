//! QA showcase for [`ErrorBanner`].

use leptos::prelude::*;

use super::ErrorBanner;

/// QA fixture: renders an error banner with a sample message.
#[component]
pub fn ErrorBannerQa() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:600px;display:flex;flex-direction:column;gap:16px;">
            <ErrorBanner message="Error loading plugins: plugin registry failed to initialise" />
            <ErrorBanner message="Error: connection to database failed" />
        </div>
    }
}
