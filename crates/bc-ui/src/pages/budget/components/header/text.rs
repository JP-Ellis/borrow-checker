// Leptos-free text helpers for the budget header.
//
// Kept separate from `mod.rs` so the pure formatting logic runs under a
// native `cargo nextest` (`mod.rs` sits under a wasm-only module tree and
// never runs there); see `components_tests` in `main.rs`.

/// One line summarising the page's verdicts, skipping empty groups.
#[must_use]
pub(crate) fn verdict_line(red: u32, warn: u32, green: u32) -> String {
    let parts: Vec<String> = [(red, "red"), (warn, "warn"), (green, "ok")]
        .into_iter()
        .filter(|&(n, _)| n > 0)
        .map(|(n, word)| format!("{n} {word}"))
        .collect();
    if parts.is_empty() {
        "no verdicts".to_owned()
    } else {
        parts.join(" \u{00b7} ")
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(3, 2, 14, "3 red \u{00b7} 2 warn \u{00b7} 14 ok")]
    #[case(0, 0, 5, "5 ok")]
    #[case(1, 0, 0, "1 red")]
    #[case(0, 0, 0, "no verdicts")]
    fn verdict_line_cases(#[case] r: u32, #[case] w: u32, #[case] g: u32, #[case] expected: &str) {
        assert_eq!(verdict_line(r, w, g), expected);
    }
}
