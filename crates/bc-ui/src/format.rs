//! Formatting helpers shared across `bc-ui` pages and components.

/// Returns the English name for a 1-based month number (1 = January).
#[inline]
pub(crate) fn month_name(month: u8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

/// Returns the English ordinal suffix for a day number.
///
/// Returns `"st"` for 1/21, `"nd"` for 2/22, `"rd"` for 3/23,
/// and `"th"` for all others in the range 1–28.
#[inline]
pub(crate) fn ordinal_suffix(day: u8) -> &'static str {
    match day {
        1 | 21 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(1, "st")]
    #[case(2, "nd")]
    #[case(3, "rd")]
    #[case(4, "th")]
    #[case(11, "th")]
    #[case(12, "th")]
    #[case(13, "th")]
    #[case(21, "st")]
    #[case(22, "nd")]
    #[case(23, "rd")]
    #[case(28, "th")]
    fn ordinal_suffix_cases(#[case] day: u8, #[case] expected: &str) {
        assert_eq!(ordinal_suffix(day), expected);
    }

    #[rstest]
    #[case(1, "January")]
    #[case(12, "December")]
    #[case(0, "Unknown")]
    #[case(13, "Unknown")]
    fn month_name_cases(#[case] month: u8, #[case] expected: &str) {
        assert_eq!(month_name(month), expected);
    }
}
