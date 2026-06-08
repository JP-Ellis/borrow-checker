//! QA showcase pages for the Plugins page (`/__test/page/plugins/*`).
//!
//! Renders each distinct visual state of the Plugins page using static mock data,
//! without calling real IPC commands.

use bc_ipc::PluginInfo;
use leptos::prelude::*;

use super::style;
use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;

/// Returns two sample plugins for QA display: one current, one deprecated.
fn sample_plugins() -> Vec<PluginInfo> {
    vec![
        PluginInfo::new(
            "commbank-au".to_owned(),
            1,
            "commbank-au.wasm".to_owned(),
            false,
        ),
        PluginInfo::new("amex-au".to_owned(), 0, "amex-au.wasm".to_owned(), true),
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
    let plugins = sample_plugins();

    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>
            <table class=style::table>
                <thead>
                    <tr>
                        <th class=style::th>"Name"</th>
                        <th class=style::th>"ABI"</th>
                        <th class=style::th>"File"</th>
                        <th class=style::th>"Status"</th>
                    </tr>
                </thead>
                <tbody>
                    {plugins
                        .into_iter()
                        .map(|plugin| {
                            let (label, tone) = if plugin.is_deprecated {
                                ("deprecated".to_owned(), Tone::Warn)
                            } else {
                                ("loaded".to_owned(), Tone::Good)
                            };
                            view! {
                                <tr class=style::row>
                                    <td class=style::td>
                                        <span class=style::plugin_name>{plugin.name}</span>
                                    </td>
                                    <td class=style::td>
                                        <code class=style::abi>{plugin.sdk_abi}</code>
                                    </td>
                                    <td class=style::td>
                                        <code class=style::file_name>{plugin.file_name}</code>
                                    </td>
                                    <td class=style::td>
                                        <StatusPill label=label tone=tone />
                                    </td>
                                </tr>
                            }
                        })
                        .collect::<Vec<_>>()}
                </tbody>
            </table>
        </div>
    }
}

/// QA fixture: renders the plugins page error state (IPC failure).
#[component]
pub fn PluginsErrorQa() -> impl IntoView {
    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>
            <p class=style::empty_state>
                "Error loading plugins: plugin registry failed to initialise"
            </p>
        </div>
    }
}
