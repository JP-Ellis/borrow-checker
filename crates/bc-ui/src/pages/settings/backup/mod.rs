// Editable backup-settings panel for Settings.
//
// This file is also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim so `settings_dirty` can be host-tested, which is why the
// module doc here uses `//` rather than `//!` (an inner doc comment is only
// valid as the first item when the file is compiled as a standalone module).

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_query_map;
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

/// Formats a byte count as a human-readable size string.
///
/// Uses binary (1024-based) units: `B`, `KB`, `MB`, `GB`, ... Values below
/// 1 KB are shown as a whole number of bytes; larger values are shown with
/// one decimal place.
///
/// # Arguments
///
/// * `bytes` - The size in bytes.
///
/// # Returns
///
/// A human-readable string such as `"512 B"` or `"1.2 MB"`.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "display-only unit-scaling math on a bounded byte count; precision loss is \
              irrelevant at these magnitudes and cannot overflow or panic"
)]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    const STEP: f64 = 1024.0;

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0_usize;
    while value >= STEP && unit_index + 1 < UNITS.len() {
        value /= STEP;
        unit_index += 1;
    }

    let unit = UNITS.get(unit_index).unwrap_or(&"PB");
    format!("{value:.1} {unit}")
}

/// Whether a listed backup is the one a `?backup=` link names: the same path,
/// or the same file name when the directory is spelt differently.
///
/// # Arguments
///
/// * `path` - The listed backup's absolute path.
/// * `file_name` - The listed backup's file name.
/// * `wanted` - The `?backup=` value.
///
/// # Returns
///
/// `true` for the backup to highlight.
#[must_use]
pub fn is_highlighted(path: &str, file_name: &str, wanted: &str) -> bool {
    !wanted.is_empty() && (path == wanted || wanted.rsplit(['/', '\\']).next() == Some(file_name))
}

/// Whether a `?backup=` link names a backup the list no longer holds, as
/// after retention prunes a pre-discard snapshot.
///
/// # Arguments
///
/// * `wanted` - The `?backup=` value, if any.
/// * `listed` - The loaded backup list.
///
/// # Returns
///
/// `true` when `wanted` is non-empty and [`is_highlighted`] matches no listed
/// backup.
#[must_use]
pub fn wanted_is_missing(wanted: Option<&str>, listed: &[bc_ipc::BackupInfo]) -> bool {
    wanted.is_some_and(|w| {
        !w.is_empty()
            && !listed
                .iter()
                .any(|b| is_highlighted(&b.path, &b.file_name, w))
    })
}

/// The notice shown when a `?backup=` link names a backup no longer listed.
#[cfg(target_arch = "wasm32")]
pub const MISSING_TEXT: &str =
    "That snapshot is no longer in the backup pool; retention removed it.";

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::format_size;
    use super::is_highlighted;
    use super::settings_dirty;
    use super::wanted_is_missing;

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

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1_258_291), "1.2 MB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }

    #[rstest]
    #[case(
        "/b/x/1.pre-discard.sqlite",
        "1.pre-discard.sqlite",
        "/b/x/1.pre-discard.sqlite",
        true
    )]
    #[case(
        "/b/x/1.pre-discard.sqlite",
        "1.pre-discard.sqlite",
        "/home/u/b/x/1.pre-discard.sqlite",
        true
    )]
    #[case(
        "/b/x/1.pre-discard.sqlite",
        "1.pre-discard.sqlite",
        "/b/x/2.pre-discard.sqlite",
        false
    )]
    #[case("/b/x/1.manual.sqlite", "1.manual.sqlite", "", false)]
    fn is_highlighted_matches_path_or_file_name(
        #[case] path: &str,
        #[case] file_name: &str,
        #[case] wanted: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_highlighted(path, file_name, wanted), expected);
    }

    fn listed() -> Vec<bc_ipc::BackupInfo> {
        vec![bc_ipc::BackupInfo::new(
            "1.pre-discard.sqlite".to_owned(),
            "/b/x/1.pre-discard.sqlite".to_owned(),
            "pre-discard".to_owned(),
            "2026-07-01T02:00:00".to_owned(),
            4096,
        )]
    }

    #[rstest]
    #[case(None, false)]
    #[case(Some(""), false)]
    #[case(Some("/b/x/1.pre-discard.sqlite"), false)]
    #[case(Some("/home/u/b/x/1.pre-discard.sqlite"), false)]
    #[case(Some("/b/x/2.pre-discard.sqlite"), true)]
    fn backup_missing_when_no_listed_backup_matches(
        #[case] wanted: Option<&str>,
        #[case] expected: bool,
    ) {
        assert_eq!(wanted_is_missing(wanted, &listed()), expected);
    }

    #[test]
    fn any_wanted_backup_is_missing_from_an_empty_list() {
        assert_eq!(wanted_is_missing(Some("1.pre-discard.sqlite"), &[]), true);
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
    // Whether the list has loaded once, so the missing-backup notice waits for it.
    let listed = RwSignal::new(false);
    let wanted = StoredValue::new(use_query_map().with_untracked(|q| q.get("backup")));

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
                listed.set(true);
            }
        });
    });
    let missing = move || {
        listed.get() && wanted.with_value(|w| backups.with(|l| wanted_is_missing(w.as_deref(), l)))
    };

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
                                        <span class=style::hint data-testid="backup-retain-hint">
                                            "Per automatic kind. Manual backups are kept until you delete them."
                                        </span>
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
                <Show when=missing>
                    <p class=style::missing role="status" data-testid="backup-missing">
                        {MISSING_TEXT}
                    </p>
                </Show>
                <ul class=style::list data-testid="backup-list">
                    <For each=move || backups.get() key=|b| b.file_name.clone() let:b>
                        {
                            let highlighted = wanted
                                .with_value(|w| {
                                    w.as_deref()
                                        .is_some_and(|w| is_highlighted(&b.path, &b.file_name, w))
                                });
                            backup_row(b, backups, banner, highlighted)
                        }
                    </For>
                </ul>
            </div>
        </div>
    }
}

/// Renders one backup row with two-step confirm gates for restore and delete.
///
/// # Arguments
///
/// * `b` - The backup metadata to render.
/// * `backups` - The panel's backup list, refreshed after a delete.
/// * `banner` - Shared banner signal used to surface restore and delete failures.
/// * `highlighted` - Whether a `?backup=` link names this backup.
#[cfg(target_arch = "wasm32")]
fn backup_row(
    b: bc_ipc::BackupInfo,
    backups: RwSignal<Vec<bc_ipc::BackupInfo>>,
    banner: RwSignal<Option<String>>,
    highlighted: bool,
) -> impl IntoView {
    let path = b.path.clone();
    let file_name = b.file_name.clone();
    let armed = RwSignal::new(false);

    let arm = move |_| armed.set(true);
    let disarm = move |_| armed.set(false);

    let row_class = if highlighted {
        format!("{} {}", style::list_row, style::list_row_highlight)
    } else {
        style::list_row.to_owned()
    };
    let row_ref = NodeRef::<leptos::html::Li>::new();
    Effect::new(move |_| {
        if highlighted && let Some(el) = row_ref.get() {
            el.scroll_into_view();
        }
    });

    view! {
        <li
            class=row_class
            data-highlighted=highlighted.to_string()
            aria-current=highlighted.then_some("true")
            node_ref=row_ref
        >
            <span class=style::list_kind>{b.kind}</span>
            <span class=style::list_when>{b.created_at}</span>
            <span class=style::list_size>{format_size(b.size_bytes)}</span>
            <span class=style::spacer />
            {move || {
                if armed.get() {
                    let path = path.clone();
                    let confirm_restore = move |_| {
                        let path = path.clone();
                        armed.set(false);
                        leptos::task::spawn_local(async move {
                            match bc_ipc::client::restore_database(&path).await {
                                Ok(()) => {
                                    banner
                                        .set(Some("Restoring… the page will reload".to_owned()));
                                    reload_when_back(banner, RESTORE_POLLS);
                                }
                                Err(e) => {
                                    leptos::logging::error!("restore_database failed: {e}");
                                    banner.set(Some(e.to_string()));
                                }
                            }
                        });
                    };
                    // The desktop app relaunches before the restore call
                    // returns; the server's reply starts the reload poll.
                    view! {
                        <button
                            class=style::abtn
                            data-testid="backup-restore-confirm"
                            on:click=confirm_restore
                        >
                            "confirm — BorrowChecker will restart"
                        </button>
                        <button
                            class=style::abtn
                            data-testid="backup-restore-cancel"
                            on:click=disarm
                        >
                            "cancel"
                        </button>
                    }
                        .into_any()
                } else {
                    view! {
                        <button class=style::abtn data-testid="backup-restore" on:click=arm>
                            "restore"
                        </button>
                    }
                        .into_any()
                }
            }}
            {delete_gate(file_name, backups, banner)}
        </li>
    }
}

/// Renders the two-step delete gate for one backup row.
///
/// # Arguments
///
/// * `file_name` - The backup's file name within the open ledger's directory.
/// * `backups` - The panel's backup list, refreshed after a delete.
/// * `banner` - Shared banner signal used to surface delete failures.
#[cfg(target_arch = "wasm32")]
fn delete_gate(
    file_name: String,
    backups: RwSignal<Vec<bc_ipc::BackupInfo>>,
    banner: RwSignal<Option<String>>,
) -> impl IntoView {
    let deleting = RwSignal::new(false);
    move || {
        if deleting.get() {
            let file_name = file_name.clone();
            let confirm_delete = move |_| {
                let file_name = file_name.clone();
                deleting.set(false);
                leptos::task::spawn_local(async move {
                    match bc_ipc::client::delete_backup(&file_name).await {
                        Ok(()) => match bc_ipc::client::list_backups().await {
                            Ok(list) => backups.set(list),
                            Err(e) => {
                                leptos::logging::error!("list_backups failed: {e}");
                                banner.set(Some(format!(
                                    "backup deleted, but the list could not be refreshed: {e}"
                                )));
                            }
                        },
                        Err(e) => {
                            leptos::logging::error!("delete_backup failed: {e}");
                            banner.set(Some(e.to_string()));
                        }
                    }
                });
            };
            view! {
                <button
                    class=style::abtn
                    data-testid="backup-delete-confirm"
                    on:click=confirm_delete
                >
                    "confirm delete"
                </button>
                <button
                    class=style::abtn
                    data-testid="backup-delete-cancel"
                    on:click=move |_| deleting.set(false)
                >
                    "cancel"
                </button>
            }
            .into_any()
        } else {
            view! {
                <button
                    class=style::abtn
                    data-testid="backup-delete"
                    on:click=move |_| deleting.set(true)
                >
                    "delete"
                </button>
            }
            .into_any()
        }
    }
}

/// How many times [`reload_when_back`] asks, one [`RESTORE_POLL`] apart.
#[cfg(target_arch = "wasm32")]
const RESTORE_POLLS: u32 = 60;

/// The wait before each [`reload_when_back`] attempt.
#[cfg(target_arch = "wasm32")]
const RESTORE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// Reloads the page once the backend answers again after a restore.
///
/// The server exits to swap the restored database in, and its supervisor
/// starts it again. Each attempt waits [`RESTORE_POLL`], then calls
/// `get_settings`; the first success reloads. After `attempts` failures the
/// banner says the server has not come back.
///
/// # Arguments
///
/// * `banner` - Shared banner signal, for the give-up message.
/// * `attempts` - Attempts left, this one included.
#[cfg(target_arch = "wasm32")]
fn reload_when_back(banner: RwSignal<Option<String>>, attempts: u32) {
    set_timeout(
        move || {
            leptos::task::spawn_local(async move {
                if bc_ipc::client::get_settings().await.is_ok() {
                    if let Some(w) = web_sys::window()
                        && let Err(e) = w.location().reload()
                    {
                        leptos::logging::warn!("reload after restore failed: {e:?}");
                    }
                } else if let Some(left) = attempts.checked_sub(1).filter(|&n| n > 0) {
                    reload_when_back(banner, left);
                } else {
                    // `try_set` hands the message back once the page is left.
                    drop(
                        banner.try_set(Some(
                            "The server has not come back. Start it again, then reload this page."
                                .to_owned(),
                        )),
                    );
                }
            });
        },
        RESTORE_POLL,
    );
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
