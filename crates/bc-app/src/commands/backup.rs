//! Tauri command handlers for backup & restore.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match the bc-ipc contract"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use std::path::Path;
use std::path::PathBuf;

use tauri::State;

use crate::AppState;

/// Returns the path of the pending-restore marker for a given database file.
///
/// The marker sits beside the database and holds the path of a validated backup
/// to swap in on next startup.
#[must_use]
pub(crate) fn restore_marker_path(db_path: &Path) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(".restore-pending");
    db_path.with_file_name(name)
}

/// Snapshots the database to the managed backup directory.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the snapshot fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn backup_database(
    state: State<'_, AppState>,
) -> Result<bc_ipc::BackupInfo, bc_ipc::BcError> {
    let rec = state
        .backup
        .backup(bc_core::BackupKind::Manual, None)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(record_to_info(&rec))
}

/// Lists existing backups, newest-first.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the directory cannot be read.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_backups(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::BackupInfo>, bc_ipc::BcError> {
    let list = state
        .backup
        .list()
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(list.iter().map(record_to_info).collect())
}

/// Validates a backup, snapshots the current DB, writes the restore marker, and
/// relaunches the app so the file is swapped in with no live connection.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the file is not a valid backup, or
/// [`bc_ipc::BcError::Internal`] on any other failure. On success this does not
/// return — the app restarts.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn restore_database(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let candidate = PathBuf::from(&path);
    bc_core::BackupService::validate(&candidate)
        .await
        .map_err(|e| bc_ipc::BcError::Validation(e.to_string()))?;
    state
        .backup
        .backup(bc_core::BackupKind::PreRestore, None)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let marker = restore_marker_path(&state.db_path);
    std::fs::write(&marker, path.as_bytes())
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    // Swap happens in setup() on next launch. `AppHandle::restart` returns `!`,
    // so as the tail expression it satisfies the `Result` return type.
    app.restart()
}

/// Reads the current backup settings from config.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the config cannot be loaded.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_backup_settings() -> Result<bc_ipc::BackupSettings, bc_ipc::BcError> {
    let settings =
        bc_config::Settings::load().map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let b = settings.backup();
    Ok(bc_ipc::BackupSettings::new(
        b.dir().map(|p| p.display().to_string()),
        b.retain_count(),
        b.retain_days(),
        b.auto_pre_migration(),
    ))
}

/// Persists updated backup settings to the config file and hot-reloads the
/// in-memory backup policy so rotation/backup calls use the new values
/// immediately (no restart required).
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the config cannot be written or
/// reloaded.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn update_backup_settings(
    settings: bc_ipc::BackupSettings,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    bc_config::persist_backup_section(
        settings.dir.as_deref(),
        settings.retain_count,
        settings.retain_days,
        settings.auto_pre_migration,
    )
    .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    // Rebuild the runtime policy from the just-saved settings and swap it into
    // the live service, so `backup_database`/rotation stop using the stale
    // startup policy.
    let reloaded =
        bc_config::Settings::load().map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let b = reloaded.backup();
    let policy = bc_core::BackupPolicy::new(
        b.resolved_dir(),
        b.retain_count(),
        b.retain_days(),
        b.auto_pre_migration(),
    );
    state.backup.set_policy(policy);
    Ok(())
}

/// Converts a core [`bc_core::BackupRecord`] into the IPC [`bc_ipc::BackupInfo`].
fn record_to_info(rec: &bc_core::BackupRecord) -> bc_ipc::BackupInfo {
    bc_ipc::BackupInfo::new(
        rec.path.display().to_string(),
        rec.kind.suffix().to_owned(),
        rec.created_at.to_string(),
        rec.size_bytes,
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::restore_marker_path;

    #[test]
    fn marker_path_is_sibling_of_db() {
        let p = restore_marker_path(std::path::Path::new("/data/db.sqlite"));
        assert_eq!(
            p,
            std::path::PathBuf::from("/data/db.sqlite.restore-pending")
        );
    }
}
