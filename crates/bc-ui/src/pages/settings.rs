//! Settings page — read-only view of application configuration.

/// Editable currency-registry panel — see [`currencies::CurrenciesPanel`].
pub(crate) mod currencies;

use bc_ipc::SettingsInfo;
use currencies::CurrenciesPanel;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;
use crate::format::month_name;
use crate::format::ordinal_suffix;

import_style!(style, "settings.module.scss");

/// Inner panel component that renders a [`SettingsInfo`] snapshot.
///
/// Separated from [`Settings`] so that QA harness pages can render the panel
/// directly with mock data without going through the Tauri IPC layer.
#[component]
pub fn SettingsPanel(
    /// The settings snapshot to display.
    info: SettingsInfo,
) -> impl IntoView {
    let day = info.financial_year_start_day;
    let month = info.financial_year_start_month;
    let fy_start = format!("{}{} {}", day, ordinal_suffix(day), month_name(month));
    let anchor = info
        .fortnightly_anchor
        .unwrap_or_else(|| "not set".to_owned());
    let db_path = info.db_path;
    let db_filename = db_path.rsplit('/').next().unwrap_or(&db_path).to_owned();
    let plugin_paths = info.plugin_paths;
    let display_commodity = info.display_commodity;
    let config_hint = info
        .config_file_path
        .unwrap_or_else(|| "the borrow-checker config file".to_owned());

    view! {
        <div class=style::settings_panel>

            <section class=style::section>
                <h2 class=style::section_title>"Financial Year"</h2>
                <dl class=style::field_list>
                    <div class=style::field_row>
                        <dt class=style::field_label>"Start"</dt>
                        <dd class=style::field_value>{fy_start}</dd>
                    </div>
                    <div class=style::field_row>
                        <dt class=style::field_label>"Fortnightly anchor"</dt>
                        <dd class=style::field_value>{anchor}</dd>
                    </div>
                </dl>
            </section>

            <section class=style::section>
                <h2 class=style::section_title>"Display"</h2>
                <dl class=style::field_list>
                    <div class=style::field_row>
                        <dt class=style::field_label>"Currency"</dt>
                        <dd class=style::field_value>{display_commodity}</dd>
                    </div>
                </dl>
            </section>

            <section class=style::section>
                <h2 class=style::section_title>"Data"</h2>
                <dl class=style::field_list>
                    <div class=style::field_row>
                        <dt class=style::field_label>"Database"</dt>
                        <dd class=style::field_value>{db_filename}</dd>
                    </div>
                    <div class=style::field_row>
                        <dt class=style::field_label>"Plugin directories"</dt>
                        <dd class=style::field_value>
                            {if plugin_paths.is_empty() {
                                view! { <span class=style::muted>"none configured"</span> }
                                    .into_any()
                            } else {
                                view! {
                                    <ul class=style::path_list>
                                        {plugin_paths
                                            .into_iter()
                                            .map(|p| view! { <li>{p}</li> })
                                            .collect::<Vec<_>>()}
                                    </ul>
                                }
                                    .into_any()
                            }}
                        </dd>
                    </div>
                </dl>
            </section>

            <p class=style::hint>
                "To change settings, edit " <code class=style::code>{config_hint}</code>
            </p>
        </div>
    }
}

/// Loading skeleton for the settings panel — prevents layout shift while the
/// IPC response is in flight.
#[component]
fn SettingsSkeleton() -> impl IntoView {
    view! {
        <div class=style::settings_panel>
            {core::iter::repeat_with(|| {
                    view! {
                        <section class=style::section>
                            <span
                                class=style::skeleton_bar
                                style="width:6rem;margin-bottom:var(--bc-space-2)"
                            />
                            <dl class=style::field_list>
                                <div class=style::field_row>
                                    <dt class=style::field_label>
                                        <span class=style::skeleton_bar style="width:5rem" />
                                    </dt>
                                    <dd class=style::field_value>
                                        <span class=style::skeleton_bar style="width:10rem" />
                                    </dd>
                                </div>
                                <div class=style::field_row>
                                    <dt class=style::field_label>
                                        <span class=style::skeleton_bar style="width:8rem" />
                                    </dt>
                                    <dd class=style::field_value>
                                        <span class=style::skeleton_bar style="width:7rem" />
                                    </dd>
                                </div>
                            </dl>
                        </section>
                    }
                })
                .take(3)
                .collect::<Vec<_>>()}
        </div>
    }
}

/// Which settings section is currently shown in the main area.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    /// The read-only configuration panel (financial year, display, data, plugins).
    General,
    /// The editable currency registry.
    Currencies,
}

/// Settings page — sidebar shell with sections for read-only configuration
/// and (in future) editable settings such as the currency registry.
///
/// Settings are fetched from the native backend via the `get_settings` Tauri
/// command. The `General` section is intentionally read-only; to change those
/// settings the user must edit the TOML config file directly.
///
/// Settings are immutable at runtime; no `data_version` subscription is needed
/// because no mutation command can invalidate this resource.
// TODO: add a manual refresh button once general settings become editable
#[component]
pub fn Settings() -> impl IntoView {
    let settings = LocalResource::new(move || async move { bc_ipc::client::get_settings().await });
    let section = RwSignal::new(SettingsSection::General);

    let nav_item = move |label: &'static str, target: SettingsSection| {
        let cls = move || {
            if section.get() == target {
                format!("{} {}", style::side_row, style::side_row_active)
            } else {
                style::side_row.to_owned()
            }
        };
        view! {
            <a class=cls on:click=move |_| section.set(target)>
                {label}
            </a>
        }
    };

    view! {
        <div class=style::shell>
            <aside class=style::sidebar>
                <div class=style::side_label>"Settings"</div>
                {nav_item("General", SettingsSection::General)}
                {nav_item("Currencies", SettingsSection::Currencies)}
            </aside>
            <main class=style::main>
                {move || match section.get() {
                    SettingsSection::General => {
                        view! {
                            <div class=format!(
                                "page {}",
                                style::page_settings,
                            )>
                                {move || match settings.get() {
                                    None => view! { <SettingsSkeleton /> }.into_any(),
                                    Some(Err(e)) => {
                                        view! {
                                            <ErrorBanner message=format!(
                                                "Failed to load settings: {e}",
                                            ) />
                                        }
                                            .into_any()
                                    }
                                    Some(Ok(s)) => view! { <SettingsPanel info=s /> }.into_any(),
                                }}
                            </div>
                        }
                            .into_any()
                    }
                    SettingsSection::Currencies => view! { <CurrenciesPanel /> }.into_any(),
                }}
            </main>
        </div>
    }
}
