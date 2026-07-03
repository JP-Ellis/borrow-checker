//! IPC helpers local to `bc-app` that orchestrate cross-item window math.
//!
//! The `bc_models`/`bc_ipc` type conversions now live with their source crates
//! as `From`/`TryFrom` impls (see `bc-ipc`, `bc-core`, `bc-config`, and
//! `bc-plugins`). What remains here is [`window_overlap`], which computes the
//! intersection of a budget revision's reign with a display window — logic that
//! belongs to the app's presentation layer rather than any single domain type.

// MARK: Budget revision view

/// Computes the overlap of a revision's reign `[reign_start, reign_end)` with the
/// display window `[win_start, win_end)`.
///
/// A `reign_end` of `None` represents the latest revision (open-ended reign).
///
/// # Arguments
///
/// * `reign_start` - Inclusive reign start (`effective_from`).
/// * `reign_end` - Exclusive reign end (next revision's `effective_from`), or `None`.
/// * `win_start` - Inclusive window start.
/// * `win_end` - Exclusive window end.
///
/// # Returns
///
/// `Some(WindowOverlap)` when the reign intersects the window, else `None`.
#[must_use]
pub(crate) fn window_overlap(
    reign_start: jiff::civil::Date,
    reign_end: Option<jiff::civil::Date>,
    win_start: jiff::civil::Date,
    win_end: jiff::civil::Date,
) -> Option<bc_ipc::WindowOverlap> {
    let start = reign_start.max(win_start);
    let end = reign_end.map_or(win_end, |re| re.min(win_end));
    if start >= end {
        return None;
    }
    let covers_full_window = start == win_start && end == win_end;
    Some(bc_ipc::WindowOverlap::new(start, end, covers_full_window))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::window_overlap;

    #[test]
    fn window_overlap_full_cover_when_reign_spans_window() {
        use jiff::civil::date;
        let o = window_overlap(
            date(2025, 1, 1),
            Some(date(2028, 1, 1)),
            date(2026, 7, 1),
            date(2026, 7, 8),
        );
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 7, 1),
                date(2026, 7, 8),
                true
            ))
        );
    }

    #[test]
    fn window_overlap_partial_from_left() {
        use jiff::civil::date;
        // reign ends inside the window -> partial, range [win_start, reign_end).
        let o = window_overlap(
            date(2025, 1, 1),
            Some(date(2026, 9, 1)),
            date(2026, 7, 1),
            date(2027, 7, 1),
        );
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 7, 1),
                date(2026, 9, 1),
                false
            ))
        );
    }

    #[test]
    fn window_overlap_open_reign_extends_to_window_end() {
        use jiff::civil::date;
        // reign_end None (latest revision) starting inside the window -> partial to win_end.
        let o = window_overlap(date(2026, 10, 1), None, date(2026, 7, 1), date(2027, 7, 1));
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 10, 1),
                date(2027, 7, 1),
                false
            ))
        );
    }

    #[test]
    fn window_overlap_none_when_disjoint() {
        use jiff::civil::date;
        let o = window_overlap(date(2030, 1, 1), None, date(2026, 7, 1), date(2027, 7, 1));
        assert_eq!(o, None);
    }
}
