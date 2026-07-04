//! Backup IPC types shared between the Tauri backend and the WASM frontend.

use serde::Deserialize;
use serde::Serialize;

/// Metadata about a single backup file, for display in the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupInfo {
    /// Absolute path to the backup file.
    pub path: String,
    /// `"manual"` or `"automatic"`.
    pub kind: String,
    /// Creation timestamp, `"YYYY-MM-DDTHH:MM:SS"`.
    pub created_at: String,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Editable backup/rotation settings surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupSettings {
    /// Backup directory override, or `None` for the default.
    pub dir: Option<String>,
    /// "Keep N newest" retention limit.
    pub retain_count: Option<u32>,
    /// "Keep newer than N days" retention limit.
    pub retain_days: Option<u32>,
    /// Whether automatic pre-migration snapshots are enabled.
    pub auto_pre_migration: bool,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn backup_settings_roundtrip() {
        let s = BackupSettings {
            dir: Some("/data/bk".to_owned()),
            retain_count: Some(5),
            retain_days: None,
            auto_pre_migration: true,
        };
        let json = serde_json::to_string(&s).expect("ser");
        let s2: BackupSettings = serde_json::from_str(&json).expect("de");
        assert_eq!(s, s2);
    }
}
