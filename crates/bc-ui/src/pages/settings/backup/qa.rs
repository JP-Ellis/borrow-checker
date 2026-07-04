//! QA showcase for [`super::BackupPanel`].
//!
//! Renders four fixed, deterministic states (clean / dirty / error /
//! empty-list) directly from in-memory data, reusing the panel's own
//! `style::` classes and `backup_row` renderer. This avoids the live IPC
//! calls that [`super::BackupPanel`] performs on mount, which would not
//! resolve outside a Tauri host.

use leptos::prelude::*;

use super::backup_row;
use super::style;
use crate::components::error_banner::ErrorBanner;

/// Builds a couple of fixed [`bc_ipc::BackupInfo`] rows for the showcase.
fn sample_backups() -> Vec<bc_ipc::BackupInfo> {
    vec![
        bc_ipc::BackupInfo::new(
            "/data/backups/2026-07-01T02-00-00.db".to_owned(),
            "pre-migration".to_owned(),
            "2026-07-01T02:00:00".to_owned(),
            1_258_291,
        ),
        bc_ipc::BackupInfo::new(
            "/data/backups/2026-07-03T09-15-00.db".to_owned(),
            "manual".to_owned(),
            "2026-07-03T09:15:00".to_owned(),
            512,
        ),
    ]
}

/// Fixed [`bc_ipc::BackupSettings`] used across the showcase states.
fn sample_settings() -> bc_ipc::BackupSettings {
    bc_ipc::BackupSettings::new(None, Some(5), Some(30), true)
}

/// Renders a single named showcase section.
///
/// # Arguments
///
/// * `label` - Heading identifying the state being shown.
/// * `show_savebar` - Whether the "dirty" save bar should render as visible.
/// * `banner` - Optional error banner message to display.
/// * `backups` - Backup rows to render in the list.
fn section(
    label: &'static str,
    show_savebar: bool,
    banner: Option<&'static str>,
    backups: Vec<bc_ipc::BackupInfo>,
) -> impl IntoView {
    let banner_signal = RwSignal::new(Option::<String>::None);
    let settings = sample_settings();
    let dir_val = settings.dir.clone().unwrap_or_default();
    let count_val = settings
        .retain_count
        .map(|n| n.to_string())
        .unwrap_or_default();
    let days_val = settings
        .retain_days
        .map(|n| n.to_string())
        .unwrap_or_default();

    view! {
        <section>
            <h2>{label}</h2>
            <div
                class=if show_savebar {
                    format!("{} {}", style::savebar, style::savebar_show)
                } else {
                    style::savebar.to_owned()
                }
                data-testid="backup-savebar"
            >
                <span class=style::spacer />
                <button class=style::abtn data-testid="backup-discard">
                    "discard"
                </button>
                <button
                    class=format!("{} {}", style::abtn, style::abtn_primary)
                    data-testid="backup-save"
                >
                    "save"
                </button>
            </div>

            <div class=style::panel>
                <h1 class=style::title>"Backup"</h1>
                <p class=style::sub>
                    "Automatic pre-migration snapshots and manual backups. Changes are staged until you save."
                </p>

                {banner.map(|msg| view! { <ErrorBanner message=msg /> })}

                <dl class=style::fields>
                    <div class=style::row>
                        <label class=style::label>"Backup directory"</label>
                        <input
                            class=style::input
                            data-testid="backup-dir"
                            prop:value=dir_val
                            placeholder="(default)"
                        />
                    </div>
                    <div class=style::row>
                        <label class=style::label>"Keep newest (count)"</label>
                        <input
                            class=style::input
                            type="number"
                            data-testid="backup-retain-count"
                            prop:value=count_val
                        />
                    </div>
                    <div class=style::row>
                        <label class=style::label>"Keep for (days)"</label>
                        <input
                            class=style::input
                            type="number"
                            data-testid="backup-retain-days"
                            prop:value=days_val
                        />
                    </div>
                    <div class=style::row>
                        <label class=style::label>"Auto pre-migration backup"</label>
                        <input
                            type="checkbox"
                            data-testid="backup-auto"
                            prop:checked=settings.auto_pre_migration
                        />
                    </div>
                </dl>

                <button class=style::addbtn data-testid="backup-create">
                    "Create backup now"
                </button>

                <h2 class=style::subtitle>"Existing backups"</h2>
                <ul class=style::list data-testid="backup-list">
                    {backups.into_iter().map(|b| backup_row(b, banner_signal)).collect_view()}
                </ul>
            </div>
        </section>
    }
}

/// Showcases the backup panel's clean, dirty, error, and empty-list states.
#[component]
pub fn BackupPanelQa() -> impl IntoView {
    view! {
        <div>
            {section("Clean", false, None, sample_backups())}
            {section("Dirty (save bar visible)", true, None, sample_backups())}
            {section(
                "Error",
                false,
                Some("Restore failed: destination is not writable"),
                sample_backups(),
            )} {section("Empty list", false, None, Vec::new())}
        </div>
    }
}
