// Leptos-free text helpers for the budget header.
//
// Kept separate from `mod.rs` so the pure formatting logic runs under a
// native `cargo nextest` (`mod.rs` sits under a wasm-only module tree and
// never runs there); see `components_tests` in `main.rs`.

/// Which verdict bucket a segment counts, used by the header to pick a
/// colour without duplicating the word/order/filter logic below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerdictKind {
    /// Rows with a red verdict.
    Red,
    /// Rows with a warn verdict.
    Warn,
    /// Rows with a green verdict.
    Ok,
}

/// Builds the ordered, non-empty `(count, word, kind)` segments underlying
/// [`verdict_line`]; both the tested string form and the header's
/// colour-coded rendering read this single list.
#[must_use]
pub(crate) fn verdict_parts(
    red: u32,
    warn: u32,
    green: u32,
) -> Vec<(u32, &'static str, VerdictKind)> {
    [
        (red, "red", VerdictKind::Red),
        (warn, "warn", VerdictKind::Warn),
        (green, "ok", VerdictKind::Ok),
    ]
    .into_iter()
    .filter(|&(n, _, _)| n > 0)
    .collect()
}

/// One line summarising the page's verdicts, skipping empty groups.
#[must_use]
pub(crate) fn verdict_line(red: u32, warn: u32, green: u32) -> String {
    let parts = verdict_parts(red, warn, green);
    if parts.is_empty() {
        "no verdicts".to_owned()
    } else {
        parts
            .into_iter()
            .map(|(n, word, _)| format!("{n} {word}"))
            .collect::<Vec<_>>()
            .join(" \u{00b7} ")
    }
}

/// Groups the header's unbudgeted entries by root, keeping their order.
///
/// The summary lists one entry per root and currency, one root's entries
/// together, so each root's currencies gather under one label.
#[must_use]
pub(crate) fn unbudgeted_by_root(
    entries: Vec<(String, bc_ipc::Amount)>,
) -> Vec<(String, Vec<bc_ipc::Amount>)> {
    let mut out: Vec<(String, Vec<bc_ipc::Amount>)> = Vec::new();
    for (root, amount) in entries {
        match out.last_mut() {
            Some((last, amounts)) if *last == root => amounts.push(amount),
            _ => out.push((root, vec![amount])),
        }
    }
    out
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Amount;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;

    #[test]
    fn unbudgeted_entries_gather_per_root() {
        let aud = Amount::new(Decimal::new(30_000, 2), "AUD");
        let usd = Amount::new(Decimal::new(12_300, 2), "USD");
        let income = Amount::new(Decimal::new(-5_000, 2), "AUD");
        let grouped = unbudgeted_by_root(vec![
            ("Expenses".to_owned(), aud.clone()),
            ("Expenses".to_owned(), usd.clone()),
            ("Income".to_owned(), income.clone()),
        ]);
        assert_eq!(
            grouped,
            vec![
                ("Expenses".to_owned(), vec![aud, usd]),
                ("Income".to_owned(), vec![income]),
            ]
        );
    }

    #[rstest]
    #[case(3, 2, 14, "3 red \u{00b7} 2 warn \u{00b7} 14 ok")]
    #[case(0, 0, 5, "5 ok")]
    #[case(1, 0, 0, "1 red")]
    #[case(0, 0, 0, "no verdicts")]
    fn verdict_line_cases(#[case] r: u32, #[case] w: u32, #[case] g: u32, #[case] expected: &str) {
        assert_eq!(verdict_line(r, w, g), expected);
    }

    #[rstest]
    #[case(
        3,
        2,
        14,
        vec![
            (3, "red", VerdictKind::Red),
            (2, "warn", VerdictKind::Warn),
            (14, "ok", VerdictKind::Ok),
        ]
    )]
    #[case(0, 0, 5, vec![(5, "ok", VerdictKind::Ok)])]
    #[case(1, 0, 0, vec![(1, "red", VerdictKind::Red)])]
    #[case(0, 0, 0, vec![])]
    fn verdict_parts_cases(
        #[case] r: u32,
        #[case] w: u32,
        #[case] g: u32,
        #[case] expected: Vec<(u32, &str, VerdictKind)>,
    ) {
        assert_eq!(verdict_parts(r, w, g), expected);
    }
}
