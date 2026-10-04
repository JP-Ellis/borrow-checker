//! The one mapping from core's verdict to budget-page colours.

use bc_ipc::Verdict;

/// A verdict's colour family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerdictTone {
    /// On track.
    Good,
    /// Approaching or at the limit.
    Warn,
    /// Over the limit, or short of a goal.
    Bad,
    /// No verdict to show.
    Mute,
}

/// The tone for `verdict`; `None` is muted.
pub(crate) fn verdict_tone(verdict: Option<Verdict>) -> VerdictTone {
    match verdict {
        Some(Verdict::Good) => VerdictTone::Good,
        Some(Verdict::Warn) => VerdictTone::Warn,
        Some(Verdict::Bad) => VerdictTone::Bad,
        _ => VerdictTone::Mute,
    }
}

#[cfg(target_arch = "wasm32")]
/// CSS colour value for a verdict, for bar segments' `--seg` variable.
pub(crate) fn verdict_color(verdict: Option<Verdict>) -> &'static str {
    match verdict_tone(verdict) {
        VerdictTone::Good => "var(--bc-good)",
        VerdictTone::Warn => "var(--bc-warn)",
        VerdictTone::Bad => "var(--bc-bad)",
        VerdictTone::Mute => "var(--bc-ink-mute)",
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Verdict;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::VerdictTone;
    use super::verdict_tone;

    #[rstest]
    #[case(Some(Verdict::Good), VerdictTone::Good)]
    #[case(Some(Verdict::Warn), VerdictTone::Warn)]
    #[case(Some(Verdict::Bad), VerdictTone::Bad)]
    #[case(None, VerdictTone::Mute)]
    fn tone(#[case] v: Option<Verdict>, #[case] expected: VerdictTone) {
        assert_eq!(verdict_tone(v), expected);
    }
}
