// The confirm gate and warning copy for the lifecycle panels. No Leptos or
// WASM here, so it is host-tested natively.

use jiff::civil::Date;

/// What a panel's preview says, as far as the confirm gate cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    /// The preview has not answered yet.
    Loading,
    /// The preview failed; the panel warns and lets the write decide.
    Failed,
    /// The preview answered with this many cascade descendants.
    Ready {
        /// How many descendants the action would carry with it.
        cascade: usize,
    },
}

/// Whether a lifecycle confirm may proceed.
///
/// A loading preview blocks. A failed one does not: the write itself refuses
/// a blocking descendant. A non-empty cascade needs its "these too" box
/// ticked.
///
/// # Arguments
///
/// * `preview` - What the preview says.
/// * `ticked` - Whether the "these too" box is ticked.
///
/// # Returns
///
/// `true` when the confirm button is enabled.
#[must_use]
pub const fn can_confirm(preview: Preview, ticked: bool) -> bool {
    match preview {
        Preview::Loading => false,
        Preview::Failed => true,
        Preview::Ready { cascade } => cascade == 0 || ticked,
    }
}

/// Whether a close confirm may proceed: [`can_confirm`], and a date is set.
///
/// # Arguments
///
/// * `on` - The close date, or `None` while the input is empty or invalid.
/// * `preview` - What the preview says.
/// * `ticked` - Whether the "close these too" box is ticked.
///
/// # Returns
///
/// `true` when the confirm button is enabled.
#[must_use]
pub const fn can_close(on: Option<Date>, preview: Preview, ticked: bool) -> bool {
    on.is_some() && can_confirm(preview, ticked)
}

/// The `cascade` flag a confirm sends.
///
/// The box counts only while the preview shows a cascade, so a failed or
/// empty preview sends `false`.
///
/// # Arguments
///
/// * `preview` - What the preview says.
/// * `ticked` - Whether the "these too" box is ticked.
///
/// # Returns
///
/// `true` when the action should carry its descendants.
#[must_use]
pub const fn sends_cascade(preview: Preview, ticked: bool) -> bool {
    matches!(preview, Preview::Ready { cascade } if cascade > 0) && ticked
}

/// The warning for postings dated after a close date.
///
/// # Arguments
///
/// * `count` - Postings after `on`, across the account and its cascade.
/// * `on` - The close date.
///
/// # Returns
///
/// `None` when no posting falls after `on`.
#[must_use]
pub fn postings_after(count: u64, on: Date) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("1 posting falls after {on}")),
        n => Some(format!("{n} postings fall after {on}")),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    const NONE: Preview = Preview::Ready { cascade: 0 };
    const TWO: Preview = Preview::Ready { cascade: 2 };

    #[rstest]
    #[case::loading(Preview::Loading, true, false)]
    #[case::failed(Preview::Failed, false, true)]
    #[case::no_cascade(NONE, false, true)]
    #[case::no_cascade_ticked(NONE, true, true)]
    #[case::cascade_unticked(TWO, false, false)]
    #[case::cascade_ticked(TWO, true, true)]
    fn confirm_gate(#[case] preview: Preview, #[case] ticked: bool, #[case] expected: bool) {
        assert_eq!(can_confirm(preview, ticked), expected);
    }

    #[rstest]
    #[case::dated(Some(date(2026, 10, 31)), true)]
    #[case::cleared(None, false)]
    fn close_needs_a_date(#[case] on: Option<Date>, #[case] expected: bool) {
        assert_eq!(can_close(on, NONE, false), expected);
    }

    #[rstest]
    #[case::failed_ticked(Preview::Failed, true, false)]
    #[case::empty_ticked(NONE, true, false)]
    #[case::cascade_unticked(TWO, false, false)]
    #[case::cascade_ticked(TWO, true, true)]
    fn cascade_flag(#[case] preview: Preview, #[case] ticked: bool, #[case] expected: bool) {
        assert_eq!(sends_cascade(preview, ticked), expected);
    }

    #[rstest]
    #[case::none(0, None)]
    #[case::one(1, Some("1 posting falls after 2026-10-31"))]
    #[case::many(3, Some("3 postings fall after 2026-10-31"))]
    fn postings_after_copy(#[case] count: u64, #[case] expected: Option<&str>) {
        assert_eq!(
            postings_after(count, date(2026, 10, 31)),
            expected.map(str::to_owned)
        );
    }
}
