// Editable backup-settings panel for Settings.
//
// This file is also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim so `settings_dirty` can be host-tested, which is why the
// module doc here uses `//` rather than `//!` (an inner doc comment is only
// valid as the first item when the file is compiled as a standalone module).

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::error_banner::ErrorBanner;

#[cfg(target_arch = "wasm32")]
import_style!(style, "backup.module.scss");

/// Whether the two settings snapshots differ (drives the dirty save bar).
///
/// # Arguments
///
/// * `a` - The pristine settings.
/// * `b` - The current draft settings.
///
/// # Returns
///
/// `true` when the draft differs from the pristine snapshot.
#[must_use]
pub fn settings_dirty(a: &bc_ipc::BackupSettings, b: &bc_ipc::BackupSettings) -> bool {
    a != b
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::settings_dirty;

    fn base() -> bc_ipc::BackupSettings {
        bc_ipc::BackupSettings::new(None, Some(5), None, true)
    }

    #[test]
    fn identical_settings_are_not_dirty() {
        assert_eq!(settings_dirty(&base(), &base()), false);
    }

    #[test]
    fn changed_retain_count_is_dirty() {
        let mut b = base();
        b.retain_count = Some(3);
        assert_eq!(settings_dirty(&base(), &b), true);
    }
}

/// Editable backup settings + backup/restore actions.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn BackupPanel() -> impl IntoView {
    let pristine = RwSignal::new(Option::<bc_ipc::BackupSettings>::None);
    let draft = RwSignal::new(Option::<bc_ipc::BackupSettings>::None);
    let banner = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);
    let backups = RwSignal::new(Vec::<bc_ipc::BackupInfo>::new());

    // Seed settings + backup list once.
    Effect::new(move |_| {
        if pristine.get_untracked().is_some() {
            return;
        }
        leptos::task::spawn_local(async move {
            match bc_ipc::client::get_backup_settings().await {
                Ok(s) => {
                    pristine.set(Some(s.clone()));
                    draft.set(Some(s));
                }
                Err(e) => banner.set(Some(e.to_string())),
            }
            if let Ok(list) = bc_ipc::client::list_backups().await {
                backups.set(list);
            }
        });
    });

    let dirty = move || match (draft.get(), pristine.get()) {
        (Some(d), Some(p)) => settings_dirty(&p, &d),
        _ => false,
    };

    let discard = move |_| {
        draft.set(pristine.get());
        banner.set(None);
    };

    let save = move |_| {
        let Some(d) = draft.get() else { return };
        if saving.get() {
            return;
        }
        saving.set(true);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::update_backup_settings(&d).await {
                Ok(()) => {
                    pristine.set(Some(d));
                    banner.set(None);
                }
                Err(e) => banner.set(Some(e.to_string())),
            }
            saving.set(false);
        });
    };

    let create_backup = move |_| {
        leptos::task::spawn_local(async move {
            match bc_ipc::client::backup_database().await {
                Ok(_) => {
                    if let Ok(list) = bc_ipc::client::list_backups().await {
                        backups.set(list);
                    }
                }
                Err(e) => banner.set(Some(e.to_string())),
            }
        });
    };

    // Field editors mutate the Option<BackupSettings> draft in place.
    let set_retain_count = move |ev: leptos::ev::Event| {
        let raw = event_target_value(&ev);
        draft.update(|d| {
            if let Some(s) = d.as_mut() {
                s.retain_count = raw.trim().parse::<u32>().ok();
            }
        });
    };
    let set_retain_days = move |ev: leptos::ev::Event| {
        let raw = event_target_value(&ev);
        draft.update(|d| {
            if let Some(s) = d.as_mut() {
                s.retain_days = raw.trim().parse::<u32>().ok();
            }
        });
    };
    let set_dir = move |ev: leptos::ev::Event| {
        let raw = event_target_value(&ev);
        draft.update(|d| {
            if let Some(s) = d.as_mut() {
                s.dir = if raw.trim().is_empty() {
                    None
                } else {
                    Some(raw)
                };
            }
        });
    };
    let toggle_auto = move |ev: leptos::ev::Event| {
        let checked = event_target_checked(&ev);
        draft.update(|d| {
            if let Some(s) = d.as_mut() {
                s.auto_pre_migration = checked;
            }
        });
    };

    view! {
        <div>
            <div
                class=move || {
                    if dirty() {
                        format!("{} {}", style::savebar, style::savebar_show)
                    } else {
                        style::savebar.to_owned()
                    }
                }
                data-testid="backup-savebar"
            >
                <span class=style::spacer />
                <button class=style::abtn data-testid="backup-discard" on:click=discard>
                    "discard"
                </button>
                <button
                    class=format!("{} {}", style::abtn, style::abtn_primary)
                    data-testid="backup-save"
                    prop:disabled=move || saving.get()
                    on:click=save
                >
                    "save"
                </button>
            </div>

            <div class=style::panel>
                <h1 class=style::title>"Backup"</h1>
                <p class=style::sub>
                    "Automatic pre-migration snapshots and manual backups. Changes are staged until you save."
                </p>

                {move || { banner.get().map(|msg| view! { <ErrorBanner message=msg /> }) }}

                {move || {
                    draft
                        .get()
                        .map(|s| {
                            let dir_val = s.dir.clone().unwrap_or_default();
                            let count_val = s
                                .retain_count
                                .map(|n| n.to_string())
                                .unwrap_or_default();
                            let days_val = s.retain_days.map(|n| n.to_string()).unwrap_or_default();
                            view! {
                                <dl class=style::fields>
                                    <div class=style::row>
                                        <label class=style::label>"Backup directory"</label>
                                        <input
                                            class=style::input
                                            data-testid="backup-dir"
                                            prop:value=dir_val
                                            placeholder="(default)"
                                            on:input=set_dir
                                        />
                                    </div>
                                    <div class=style::row>
                                        <label class=style::label>"Keep newest (count)"</label>
                                        <input
                                            class=style::input
                                            type="number"
                                            data-testid="backup-retain-count"
                                            prop:value=count_val
                                            on:input=set_retain_count
                                        />
                                    </div>
                                    <div class=style::row>
                                        <label class=style::label>"Keep for (days)"</label>
                                        <input
                                            class=style::input
                                            type="number"
                                            data-testid="backup-retain-days"
                                            prop:value=days_val
                                            on:input=set_retain_days
                                        />
                                    </div>
                                    <div class=style::row>
                                        <label class=style::label>
                                            "Auto pre-migration backup"
                                        </label>
                                        <input
                                            type="checkbox"
                                            data-testid="backup-auto"
                                            prop:checked=s.auto_pre_migration
                                            on:change=toggle_auto
                                        />
                                    </div>
                                </dl>
                            }
                        })
                }}

                <button class=style::addbtn data-testid="backup-create" on:click=create_backup>
                    "Create backup now"
                </button>

                <h2 class=style::subtitle>"Existing backups"</h2>
                <ul class=style::list data-testid="backup-list">
                    <For each=move || backups.get() key=|b| b.path.clone() let:b>
                        {backup_row(b)}
                    </For>
                </ul>
            </div>
        </div>
    }
}

/// Renders one backup row with a restore button.
#[cfg(target_arch = "wasm32")]
fn backup_row(b: bc_ipc::BackupInfo) -> impl IntoView {
    let path = b.path.clone();
    let restore = move |_| {
        let path = path.clone();
        leptos::task::spawn_local(async move {
            // On success the backend relaunches the app; only the error arm
            // is actionable here.
            if let Err(e) = bc_ipc::client::restore_database(&path).await {
                leptos::logging::error!("restore_database failed: {e}");
            }
        });
    };
    view! {
        <li class=style::list_row>
            <span class=style::list_kind>{b.kind}</span>
            <span class=style::list_when>{b.created_at}</span>
            <span class=style::spacer />
            <button class=style::abtn data-testid="backup-restore" on:click=restore>
                "restore"
            </button>
        </li>
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
