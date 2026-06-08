//! Settings page — read-only view of application configuration.

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "settings.module.scss");

/// Returns the English name for a 1-based month number.
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

/// Returns an ordinal suffix for a day number.
///
/// Returns `"st"` for 1/21, `"nd"` for 2/22, `"rd"` for 3/23,
/// and `"th"` for all others in the range 1–28.
fn ordinal_suffix(day: u8) -> &'static str {
    match day {
        1 | 21 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    }
}

/// Settings page — displays current application configuration in a read-only
/// info panel, grouped into logical sections.
///
/// Settings are fetched from the native backend via the `get_settings` Tauri
/// command. The page is intentionally read-only; to change settings the user
/// must edit the TOML config file directly.
#[component]
pub fn Settings() -> impl IntoView {
    let settings = LocalResource::new(move || async move { bc_ipc::client::get_settings().await });

    view! {
        <div class="page page-settings">
            {move || match settings.get() {
                None => view! { <p class=style::loading>"Loading settings…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p class=style::error>{format!("Failed to load settings: {e}")}</p> }
                        .into_any()
                }
                Some(Ok(s)) => {
                    let day = s.financial_year_start_day;
                    let month = s.financial_year_start_month;
                    let fy_start = format!("{}{} {}", day, ordinal_suffix(day), month_name(month));
                    let anchor = s.fortnightly_anchor.unwrap_or_else(|| "not set".to_owned());
                    let db_filename = s
                        .db_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(s.db_path.as_str())
                        .to_owned();
                    let plugin_paths = s.plugin_paths.clone();

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
                                        <dd class=style::field_value>{s.display_commodity}</dd>
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
                                                            .iter()
                                                            .map(|p| {
                                                                view! { <li>{p.clone()}</li> }
                                                            })
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
                                "To change settings, edit "
                                <code class=style::code>
                                    "~/.config/borrow-checker/config.toml"
                                </code>
                            </p>
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}
