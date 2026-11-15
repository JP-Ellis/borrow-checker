//! Settings page — the read-only configuration summary and the editable
//! currency, account, backup and transfer panels, one at a time by
//! `?section=`.

/// Editable account table — see [`accounts::AccountsPanel`].
pub(crate) mod accounts;
/// Editable backup-settings panel — see [`backup::BackupPanel`].
pub(crate) mod backup;
/// Editable currency-registry panel — see [`currencies::CurrenciesPanel`].
pub(crate) mod currencies;
/// The `?section=` mapping — see [`section::SettingsSection`].
pub(crate) mod section;
/// Transfer-suggestion review panel — see [`transfers::TransfersPanel`].
pub(crate) mod transfers;

use accounts::AccountsPanel;
use backup::BackupPanel;
use bc_ipc::SettingsInfo;
use currencies::CurrenciesPanel;
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use section::SettingsSection;
use stylance::import_style;
use transfers::TransfersPanel;

use crate::components::error_banner::ErrorBanner;
use crate::format::month_name;
use crate::format::ordinal_suffix;
use crate::period_defaults::use_settings;

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

/// Settings page — sidebar shell with sections for read-only configuration
/// and (in future) editable settings such as the currency registry.
///
/// The `General` section shows the settings the shell loaded at startup. It is
/// intentionally read-only; to change those settings the user must edit the
/// TOML config file directly.
// TODO: add a manual refresh button once general settings become editable
#[component]
pub fn Settings() -> impl IntoView {
    let settings = use_settings();
    let initial = use_query_map()
        .with_untracked(|q| q.get("section"))
        .as_deref()
        .and_then(SettingsSection::from_query)
        .unwrap_or(SettingsSection::General);
    let section = RwSignal::new(initial);

    let nav_item = move |label: &'static str, target: SettingsSection| {
        let cls = move || {
            if section.get() == target {
                format!("{} {}", style::side_row, style::side_row_active)
            } else {
                style::side_row.to_owned()
            }
        };
        let aria_current = move || section.get() == target;
        let testid = match target {
            SettingsSection::General => None,
            SettingsSection::Currencies => Some("settings-nav-currencies"),
            SettingsSection::Accounts => Some("settings-nav-accounts"),
            SettingsSection::Backup => Some("settings-nav-backup"),
            SettingsSection::Transfers => Some("settings-nav-transfers"),
        };
        view! {
            <button
                type="button"
                class=cls
                aria-current=move || aria_current().then_some("page")
                data-testid=testid
                on:click=move |_| section.set(target)
            >
                {label}
            </button>
        }
    };

    view! {
        <div class=style::shell>
            <aside class=style::sidebar>
                <div class=style::side_label>"Settings"</div>
                {nav_item("General", SettingsSection::General)}
                {nav_item("Currencies", SettingsSection::Currencies)}
                {nav_item("Accounts", SettingsSection::Accounts)}
                {nav_item("Backup", SettingsSection::Backup)}
                {nav_item("Transfers", SettingsSection::Transfers)}
            </aside>
            <main class=style::main>
                {move || match section.get() {
                    SettingsSection::General => {
                        view! {
                            <div class=format!(
                                "page {}",
                                style::page_settings,
                            )>
                                {match settings.clone() {
                                    None => {
                                        view! {
                                            <ErrorBanner message="Settings are unavailable outside the app shell." />
                                        }
                                            .into_any()
                                    }
                                    Some(s) => view! { <SettingsPanel info=s /> }.into_any(),
                                }}
                            </div>
                        }
                            .into_any()
                    }
                    SettingsSection::Currencies => view! { <CurrenciesPanel /> }.into_any(),
                    SettingsSection::Accounts => view! { <AccountsPanel /> }.into_any(),
                    SettingsSection::Backup => view! { <BackupPanel /> }.into_any(),
                    SettingsSection::Transfers => view! { <TransfersPanel /> }.into_any(),
                }}
            </main>
        </div>
    }
}
