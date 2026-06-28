// Pure de-duplication of the transaction audit trail for display.
//
// `audit_rows` collapses the timestamp of consecutive entries that share the
// same instant so a run of changes made together renders under one time label.
// Comparison is on the underlying [`jiff::Timestamp`], not its formatted
// `HH:MM` label, so two distinct instants that format identically each keep
// their label. No Leptos here — this is host-tested via the `include!` shim in
// `main.rs`.

use bc_ipc::AuditEntry;

/// One rendered audit row.
///
/// `time` carries the formatted `HH:MM` label only for the first entry of a
/// same-instant run; subsequent rows in the run carry `None` so the gutter is
/// left blank.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "AuditRow is the canonical interface name specified by the task SDD"
)]
pub struct AuditRow {
    /// Time label for the first row of an instant-group, else `None`.
    pub time: Option<String>,
    /// Event kind, e.g. `"import"`, `"user"`.
    pub kind: String,
    /// Human-readable message.
    pub message: String,
}

/// De-duplicates timestamps across a chronological audit trail.
///
/// # Arguments
///
/// * `entries` - Audit entries in chronological order.
///
/// # Returns
///
/// One [`AuditRow`] per entry, in input order, with the time label blanked on
/// consecutive entries that share the same instant.
#[must_use]
#[expect(
    clippy::module_name_repetitions,
    reason = "audit_rows is the canonical interface name specified by the task SDD"
)]
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "wasm consumer arrives in Task 4; remove then")
)]
pub fn audit_rows(entries: &[AuditEntry]) -> Vec<AuditRow> {
    let mut rows = Vec::with_capacity(entries.len());
    let mut prev: Option<jiff::Timestamp> = None;
    for e in entries {
        let time = if prev == Some(e.time) {
            None
        } else {
            Some(e.time_label())
        };
        prev = Some(e.time);
        rows.push(AuditRow {
            time,
            kind: e.kind.clone(),
            message: e.message.clone(),
        });
    }
    rows
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "test code: index bounds are asserted by the test setup"
)]
mod tests {
    use bc_ipc::AuditEntry;
    use pretty_assertions::assert_eq;

    use super::audit_rows;

    fn ts(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(secs).expect("valid timestamp")
    }

    #[test]
    fn same_instant_run_blanks_repeat_labels() {
        let t = ts(1_700_000_000);
        let entries = vec![
            AuditEntry::new(t, "user", "edited description"),
            AuditEntry::new(t, "user", "recategorised"),
        ];
        let rows = audit_rows(&entries);
        assert!(rows[0].time.is_some());
        assert_eq!(rows[1].time, None);
    }

    #[test]
    fn distinct_instants_keep_labels() {
        let entries = vec![
            AuditEntry::new(ts(1_700_000_000), "user", "a"),
            AuditEntry::new(ts(1_700_000_600), "user", "b"),
        ];
        let rows = audit_rows(&entries);
        assert!(rows[0].time.is_some());
        assert!(rows[1].time.is_some());
    }

    #[test]
    fn same_minute_distinct_seconds_both_show() {
        // Two instants one second apart, mid-minute: identical HH:MM label but
        // distinct instants, so neither is blanked.
        let entries = vec![
            AuditEntry::new(ts(1_700_000_100), "user", "a"),
            AuditEntry::new(ts(1_700_000_101), "user", "b"),
        ];
        let rows = audit_rows(&entries);
        assert!(rows[0].time.is_some());
        assert!(rows[1].time.is_some());
    }

    #[test]
    fn carries_kind_and_message() {
        let rows = audit_rows(&[AuditEntry::new(ts(0), "import", "commbank")]);
        assert_eq!(rows[0].kind, "import");
        assert_eq!(rows[0].message, "commbank");
    }
}
