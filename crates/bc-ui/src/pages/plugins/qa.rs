//! QA showcase pages for the Plugins page (`/__test/page/plugins/*`).
//!
//! Renders each distinct visual state of the Plugins page using static mock data,
//! without calling real IPC commands.

use bc_ipc::PluginInfo;
use leptos::prelude::*;

use super::PluginsTable;
use super::style;
use crate::components::error_banner::ErrorBanner;

/// Returns two sample plugins for QA display: one current, one deprecated.
///
/// The deprecated entry uses `sdk_abi = 1` with `is_deprecated = true` to
/// represent the visual state that will occur when the ABI grace window opens
/// (i.e. when `HOST_ABI_MIN` is bumped above `HOST_ABI_DEPRECATED_MIN`). The
/// real registry gate would reject `sdk_abi = 0`, so that value is intentionally
/// avoided here.
fn sample_plugins() -> Vec<PluginInfo> {
    vec![
        PluginInfo::new(
            "commbank-au".to_owned(),
            1,
            "commbank-au.wasm".to_owned(),
            false,
        ),
        PluginInfo::new("amex-au".to_owned(), 1, "amex-au.wasm".to_owned(), true),
    ]
}

/// QA fixture: renders the plugins page empty state (no plugins installed).
#[component]
pub fn PluginsEmptyQa() -> impl IntoView {
    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>
            <p class=style::empty_state>
                "No plugins installed — drop " <code>".wasm"</code>
                " files into your plugins directory to add importers."
            </p>
        </div>
    }
}

/// QA fixture: renders the plugins page with a populated table (two mock rows).
#[component]
pub fn PluginsFullQa() -> impl IntoView {
    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>
            <PluginsTable plugins=sample_plugins() />
        </div>
    }
}

/// QA fixture: renders the plugins page error state (IPC failure).
#[component]
pub fn PluginsErrorQa() -> impl IntoView {
    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>
            <ErrorBanner message="Error loading plugins: plugin registry failed to initialise" />
        </div>
    }
}
