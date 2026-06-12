//! Plugins page — installed WASM importer plugin list.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::PluginInfo;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;
use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;

import_style!(pub(crate) style, "plugins.module.scss");

/// Plugins page — lists every installed importer plugin with its ABI version
/// and the source `.wasm` filename.
#[component]
pub fn Plugins() -> impl IntoView {
    // Plugins are a startup snapshot — not wired to data_version (no hot-reload).
    let plugins_resource = LocalResource::new(bc_ipc::client::list_plugins);

    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>

            {move || match plugins_resource.get() {
                None => view! { <PluginsSkeleton /> }.into_any(),
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("Error loading plugins: {e}") /> }
                        .into_any()
                }
                Some(Ok(plugins)) => {
                    if plugins.is_empty() {
                        view! {
                            <p class=style::empty_state>
                                "No plugins installed — drop " <code>".wasm"</code>
                                " files into your plugins directory to add importers."
                            </p>
                        }
                            .into_any()
                    } else {
                        view! { <PluginsTable plugins=plugins /> }.into_any()
                    }
                }
            }}
        </div>
    }
}

/// Renders the plugin list table. Extracted so QA fixtures can exercise the
/// table rendering without duplicating markup.
#[component]
pub fn PluginsTable(
    /// The list of plugins to render.
    plugins: Vec<PluginInfo>,
) -> impl IntoView {
    view! {
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
    }
}

/// Loading skeleton for the plugins table — prevents layout shift while the
/// IPC response is in flight.
#[component]
fn PluginsSkeleton() -> impl IntoView {
    view! {
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
                {core::iter::repeat_with(|| {
                        view! {
                            <tr class=style::row>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:8rem" />
                                </td>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:2rem" />
                                </td>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:10rem" />
                                </td>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:4rem" />
                                </td>
                            </tr>
                        }
                    })
                    .take(3)
                    .collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
