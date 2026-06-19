//! Pure revision-resolution and period-tiling logic for versioned budgets.

use jiff::civil::Date;

use crate::BudgetRevision;

/// A budget period resolved against a revision timeline.
///
/// `start` is inclusive, `end` exclusive. `is_stub` is `true` when the period
/// was truncated by the next revision's `effective_from` (i.e. shorter than the
/// revision's natural period).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ResolvedPeriod<'a> {
    /// Inclusive start.
    pub start: Date,
    /// Exclusive end.
    pub end: Date,
    /// Revision governing this period.
    pub revision: &'a BudgetRevision,
    /// `true` when truncated by the next revision boundary.
    pub is_stub: bool,
}

/// Returns the revision governing `date` (greatest `effective_from <= date`).
///
/// # Arguments
///
/// * `revisions` - Revisions sorted ascending by `effective_from`.
/// * `date` - The date to resolve.
///
/// # Returns
///
/// The governing revision, or `None` if `date` precedes the earliest revision.
#[must_use]
#[inline]
pub fn governing_revision(revisions: &[BudgetRevision], date: Date) -> Option<&BudgetRevision> {
    revisions
        .iter()
        .rev()
        .find(|r| r.effective_from() <= date)
}

/// Enumerates the revision-defined periods overlapping `[from, to)`.
///
/// Each revision tiles its own grid from its `effective_from`, stepping by the
/// revision's period. The final period of a reign is truncated to a stub at the
/// next revision's `effective_from`. Only periods overlapping `[from, to)` are
/// returned.
///
/// # Arguments
///
/// * `revisions` - Revisions sorted ascending by `effective_from`.
/// * `from` - Inclusive window start.
/// * `to` - Exclusive window end.
///
/// # Returns
///
/// Resolved periods in chronological order. Empty if `from >= to` or no reign
/// overlaps the window.
#[must_use]
#[inline]
#[expect(
    clippy::elidable_lifetime_names,
    reason = "explicit 'a clarifies that ResolvedPeriod borrows from revisions"
)]
pub fn periods_overlapping<'a>(
    revisions: &'a [BudgetRevision],
    from: Date,
    to: Date,
) -> Vec<ResolvedPeriod<'a>> {
    let mut out = Vec::new();
    if from >= to {
        return out;
    }
    for (i, rev) in revisions.iter().enumerate() {
        let reign_start = rev.effective_from();
        let reign_end = revisions.get(i.saturating_add(1)).map(BudgetRevision::effective_from);
        if reign_start >= to {
            break;
        }
        if let Some(re) = reign_end
            && re <= from
        {
            continue;
        }
        let mut cursor = reign_start;
        loop {
            let natural_end = rev.period().advance(cursor);
            let (p_end, is_stub) = match reign_end {
                Some(re) if re < natural_end => (re, true),
                _ => (natural_end, false),
            };
            if cursor < to && p_end > from {
                out.push(ResolvedPeriod { start: cursor, end: p_end, revision: rev, is_stub });
            }
            if p_end >= to {
                break;
            }
            if let Some(re) = reign_end
                && p_end >= re
            {
                break;
            }
            cursor = p_end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::civil::{Date, date};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{BudgetId, BudgetRevision, Period, RolloverPolicy};

    fn rev(eff: Date, period: Period) -> BudgetRevision {
        BudgetRevision::builder()
            .budget_id(BudgetId::new())
            .effective_from(eff)
            .period(period)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "unwrap in tests is acceptable to assert the expected Some state"
    )]
    fn governing_picks_greatest_effective_from_le_date() {
        let revs = vec![rev(date(2026, 1, 1), Period::Weekly), rev(date(2027, 1, 1), Period::Monthly)];
        assert_eq!(governing_revision(&revs, date(2026, 6, 1)).unwrap().period(), &Period::Weekly);
        assert_eq!(governing_revision(&revs, date(2027, 6, 1)).unwrap().period(), &Period::Monthly);
        assert!(governing_revision(&revs, date(2025, 1, 1)).is_none());
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "indices 0 and 2 are valid: we assert ps.len() == 3 immediately before"
    )]
    fn single_revision_tiles_from_effective_from() {
        let revs = vec![rev(date(2026, 1, 1), Period::Weekly)];
        let ps = periods_overlapping(&revs, date(2026, 1, 1), date(2026, 1, 22));
        assert_eq!(ps.len(), 3);
        assert_eq!((ps[0].start, ps[0].end), (date(2026, 1, 1), date(2026, 1, 8)));
        assert_eq!((ps[2].start, ps[2].end), (date(2026, 1, 15), date(2026, 1, 22)));
        assert!(ps.iter().all(|p| !p.is_stub));
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "unwrap in tests is acceptable to assert the expected Some state"
    )]
    fn boundary_truncates_prior_reign_to_stub() {
        // Weekly from Jan 1; switch Jan 10 (mid-week). The week [Jan 8, Jan 15)
        // is cut to a stub [Jan 8, Jan 10); a fresh grid starts at Jan 10.
        let revs = vec![rev(date(2026, 1, 1), Period::Weekly), rev(date(2026, 1, 10), Period::Weekly)];
        let ps = periods_overlapping(&revs, date(2026, 1, 1), date(2026, 1, 17));
        // Reign 1 periods: [1,8) full, [8,10) stub.
        let stub = ps.iter().find(|p| p.start == date(2026, 1, 8)).unwrap();
        assert_eq!(stub.end, date(2026, 1, 10));
        assert!(stub.is_stub);
        // Reign 2 re-anchors at Jan 10: [10,17) full.
        let reanchored = ps.iter().find(|p| p.start == date(2026, 1, 10)).unwrap();
        assert_eq!(reanchored.end, date(2026, 1, 17));
        assert!(!reanchored.is_stub);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "index 0 is valid: we assert ps.len() == 1 immediately before"
    )]
    fn only_periods_overlapping_window_returned() {
        let revs = vec![rev(date(2026, 1, 1), Period::Weekly)];
        let ps = periods_overlapping(&revs, date(2026, 1, 9), date(2026, 1, 13));
        // window overlaps [8,15) only.
        assert_eq!(ps.len(), 1);
        assert_eq!((ps[0].start, ps[0].end), (date(2026, 1, 8), date(2026, 1, 15)));
    }
}
