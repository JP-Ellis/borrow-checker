//! Plugins page — installed WASM importer plugin list.

use leptos::prelude::*;
use stylance::import_style;

use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;

import_style!(style, "plugins.module.scss");

/// Plugins page — lists every installed importer plugin with its ABI version
/// and the source `.wasm` filename.
#[component]
pub fn Plugins() -> impl IntoView {
    let plugins_resource = LocalResource::new(bc_ipc::client::list_plugins);

    view! {
        <div class=format!("page {}", style::page_plugins)>
            <h1 class=style::heading>"Plugins"</h1>

            {move || match plugins_resource.get() {
                None => view! { <p class=style::empty_state>"Loading plugins…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! {
                        <p class=style::empty_state>{format!("Error loading plugins: {e}")}</p>
                    }
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
                                            let file_name = std::path::Path::new(&plugin.source_path)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or(&plugin.source_path)
                                                .to_owned();
                                            view! {
                                                <tr class=style::row>
                                                    <td class=style::td>
                                                        <span class=style::plugin_name>{plugin.name.clone()}</span>
                                                    </td>
                                                    <td class=style::td>
                                                        <code class=style::abi>{plugin.sdk_abi}</code>
                                                    </td>
                                                    <td class=style::td>
                                                        <code class=style::file_name>{file_name}</code>
                                                    </td>
                                                    <td class=style::td>
                                                        <StatusPill label="loaded".to_owned() tone=Tone::Good />
                                                    </td>
                                                </tr>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                            .into_any()
                    }
                }
            }}
        </div>
    }
}
