//! Database backup: `VACUUM INTO` snapshots with conservative rotation.

use std::path::PathBuf;

/// The origin of a backup, encoded in its filename suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupKind is the clearest public name for this type; the module is private to the crate root re-export"
)]
pub enum BackupKind {
    /// Created explicitly by the user (`.manual`).
    Manual,
    /// Created automatically (pre-migration or pre-restore) (`.automatic`).
    Automatic,
}

impl BackupKind {
    /// Returns the filename suffix for this kind (without dots).
    #[inline]
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
        }
    }

    /// Parses a kind from a filename suffix, if recognised.
    #[inline]
    #[must_use]
    pub fn from_suffix(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Self::Manual),
            "automatic" => Some(Self::Automatic),
            _ => None,
        }
    }
}

/// Metadata about a single backup file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupRecord is the clearest public name for this type; re-exported at the crate root"
)]
pub struct BackupRecord {
    /// Absolute path to the backup file.
    pub path: PathBuf,
    /// Whether the backup was created manually or automatically.
    pub kind: BackupKind,
    /// The creation timestamp parsed from the filename (local civil time).
    pub created_at: jiff::civil::DateTime,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Runtime backup and rotation policy (translated from `bc_config::BackupSection`).
#[derive(Debug, Clone)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupPolicy is the clearest public name for this type; re-exported at the crate root"
)]
pub struct BackupPolicy {
    /// Directory backups are written to and rotated within.
    pub dir: PathBuf,
    /// "Keep N newest" retention limit.
    pub retain_count: Option<u32>,
    /// "Keep newer than N days" retention limit.
    pub retain_days: Option<u32>,
    /// Whether to snapshot automatically before migrations.
    pub auto_pre_migration: bool,
}

impl BackupPolicy {
    /// Creates a new backup policy.
    ///
    /// # Arguments
    ///
    /// * `dir` - Directory backups live in.
    /// * `retain_count` - "Keep N newest" limit, or `None`.
    /// * `retain_days` - "Keep newer than N days" limit, or `None`.
    /// * `auto_pre_migration` - Whether pre-migration snapshots are enabled.
    #[inline]
    #[must_use]
    pub fn new(
        dir: PathBuf,
        retain_count: Option<u32>,
        retain_days: Option<u32>,
        auto_pre_migration: bool,
    ) -> Self {
        Self {
            dir,
            retain_count,
            retain_days,
            auto_pre_migration,
        }
    }
}

/// Decides which backups to prune under the conservative union policy.
///
/// `ages_days` MUST be sorted newest-first (ascending age). A backup is kept if
/// it is among the `retain_count` newest **or** newer than `retain_days`; it is
/// pruned only if it satisfies neither. When both limits are `None`, nothing is
/// pruned.
///
/// # Arguments
///
/// * `ages_days` - Age of each backup in whole days, newest-first.
/// * `retain_count` - "Keep N newest" limit.
/// * `retain_days` - "Keep newer than N days" limit.
///
/// # Returns
///
/// The indices (into `ages_days`) of backups to delete.
#[must_use]
pub fn prune_indices(
    ages_days: &[i64],
    retain_count: Option<u32>,
    retain_days: Option<u32>,
) -> Vec<usize> {
    if retain_count.is_none() && retain_days.is_none() {
        return Vec::new();
    }
    ages_days
        .iter()
        .enumerate()
        .filter_map(|(i, &age)| {
            #[expect(
                clippy::as_conversions,
                reason = "index i is a small slice position; u64::try_from would be fallible for no practical benefit here"
            )]
            let within_count = retain_count.is_some_and(|n| (i as u64) < u64::from(n));
            let within_age = retain_days.is_some_and(|d| age < i64::from(d));
            (!(within_count || within_age)).then_some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::prune_indices;

    #[test]
    fn prune_disabled_when_both_limits_unset() {
        // 4 backups aged 0,10,100,400 days; no limits ⇒ keep all.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], None, None),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn prune_count_only_keeps_newest_n() {
        // Keep 2 newest ⇒ delete indices 2 and 3 (ages ignored).
        assert_eq!(prune_indices(&[0, 1, 500, 9000], Some(2), None), vec![2, 3]);
    }

    #[test]
    fn prune_age_only_keeps_recent() {
        // Keep < 90 days ⇒ delete the 100- and 400-day-old ones.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], None, Some(90)),
            vec![2, 3]
        );
    }

    #[test]
    fn prune_union_deletes_only_when_beyond_both() {
        // count=2, days=90. Index 2 (100d) is beyond count AND age ⇒ delete.
        // Index 1 (10d) beyond count? no (i<2). Kept. Index 3 (400d) delete.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], Some(2), Some(90)),
            vec![2, 3]
        );
    }

    #[test]
    fn prune_union_age_rescues_old_beyond_count() {
        // count=1, days=90: index1 (10d) is beyond count but within age ⇒ kept.
        assert_eq!(prune_indices(&[0, 10, 100], Some(1), Some(90)), vec![2]);
    }
}
