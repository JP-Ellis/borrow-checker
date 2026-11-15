// The confirm gate and warning copy for the lifecycle panels. No Leptos or
// WASM here, so it is host-tested natively.

use jiff::civil::Date;

/// Whether a lifecycle confirm may proceed.
///
/// A non-empty cascade needs its "these too" box ticked; nothing else blocks.
///
/// # Arguments
///
/// * `cascade_len` - How many descendants the action would carry with it.
/// * `ticked` - Whether the "these too" box is ticked.
///
/// # Returns
///
/// `true` when the confirm button is enabled.
#[must_use]
pub const fn can_confirm(cascade_len: usize, ticked: bool) -> bool {
    cascade_len == 0 || ticked
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

    #[rstest]
    #[case::no_cascade(0, false, true)]
    #[case::no_cascade_ticked(0, true, true)]
    #[case::cascade_unticked(2, false, false)]
    #[case::cascade_ticked(2, true, true)]
    fn confirm_gate(#[case] cascade_len: usize, #[case] ticked: bool, #[case] expected: bool) {
        assert_eq!(can_confirm(cascade_len, ticked), expected);
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
