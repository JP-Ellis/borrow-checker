//! Settings page — read-only view of application configuration.

use bc_ipc::SettingsInfo;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;

import_style!(style, "settings.module.scss");

/// Returns the English name for a 1-based month number (1 = January).
#[inline]
fn month_name(month: u8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

/// Returns the English ordinal suffix for a day number.
///
/// Returns `"st"` for 1/21, `"nd"` for 2/22, `"rd"` for 3/23,
/// and `"th"` for all others in the range 1–28.
#[inline]
fn ordinal_suffix(day: u8) -> &'static str {
    match day {
        1 | 21 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    }
}

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

/// Settings page — displays current application configuration in a read-only
/// info panel, grouped into logical sections.
///
/// Settings are fetched from the native backend via the `get_settings` Tauri
/// command. The page is intentionally read-only; to change settings the user
/// must edit the TOML config file directly.
///
/// Settings are immutable at runtime; no `data_version` subscription is needed
/// because no mutation command can invalidate this resource.
// TODO: add a manual refresh button once settings become editable
#[component]
pub fn Settings() -> impl IntoView {
    let settings = LocalResource::new(move || async move { bc_ipc::client::get_settings().await });

    view! {
        <div class=format!(
            "page {}",
            style::page_settings,
        )>
            {move || match settings.get() {
                None => view! { <SettingsSkeleton /> }.into_any(),
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("Failed to load settings: {e}") /> }
                        .into_any()
                }
                Some(Ok(s)) => view! { <SettingsPanel info=s /> }.into_any(),
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(1, "st")]
    #[case(2, "nd")]
    #[case(3, "rd")]
    #[case(4, "th")]
    #[case(11, "th")]
    #[case(12, "th")]
    #[case(13, "th")]
    #[case(21, "st")]
    #[case(22, "nd")]
    #[case(23, "rd")]
    #[case(28, "th")]
    fn ordinal_suffix_cases(#[case] day: u8, #[case] expected: &str) {
        assert_eq!(ordinal_suffix(day), expected);
    }

    #[rstest]
    #[case(1, "January")]
    #[case(12, "December")]
    #[case(0, "Unknown")]
    #[case(13, "Unknown")]
    fn month_name_cases(#[case] month: u8, #[case] expected: &str) {
        assert_eq!(month_name(month), expected);
    }
}
