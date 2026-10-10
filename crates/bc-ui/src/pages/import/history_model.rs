//! Leptos-free text for the History section.

use bc_ipc::BatchState;
use bc_ipc::ImportBatchInfo;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::components::status_pill::Tone;
use crate::pages::import::model::count_noun;
use crate::pages::import::model::counts_line;

/// The outcome cell of a run that never finished.
pub(crate) const INCOMPLETE_TEXT: &str =
    "Run stopped before finishing; rows already written stay until discarded";

/// A batch's state pill.
///
/// # Arguments
///
/// * `state` - The batch state.
///
/// # Returns
///
/// The label and tone: complete is good, incomplete warns, discarded is muted.
#[must_use]
pub(crate) fn state_pill(state: &BatchState) -> (&'static str, Tone) {
    match state {
        BatchState::Complete => ("complete", Tone::Good),
        BatchState::Incomplete => ("incomplete", Tone::Warn),
        BatchState::Discarded => ("discarded", Tone::Muted),
        _ => ("unknown", Tone::Warn),
    }
}

/// Whether a batch is discarded.
///
/// # Arguments
///
/// * `state` - The batch state.
///
/// # Returns
///
/// `true` for a discarded batch.
#[must_use]
pub(crate) fn is_discarded(state: &BatchState) -> bool {
    matches!(state, BatchState::Discarded)
}

/// The batch's profile name, or its bare importer once the profile is gone.
///
/// # Arguments
///
/// * `info` - The batch.
///
/// # Returns
///
/// The name to show.
#[must_use]
pub(crate) fn batch_name(info: &ImportBatchInfo) -> &str {
    info.profile.as_deref().unwrap_or(&info.importer)
}

/// The History outcome cell.
///
/// # Arguments
///
/// * `info` - The batch.
///
/// # Returns
///
/// [`INCOMPLETE_TEXT`] for an incomplete run, else its counts.
#[must_use]
pub(crate) fn outcome_text(info: &ImportBatchInfo) -> String {
    if matches!(info.state, BatchState::Incomplete) {
        return INCOMPLETE_TEXT.to_owned();
    }
    info.counts.as_ref().map_or_else(
        || "no counts recorded".to_owned(),
        |c| counts_line(c.new_transactions, c.attached_postings, c.skipped_postings),
    )
}

/// An RFC 3339 timestamp as `YYYY-MM-DD HH:MM` in `tz`.
///
/// # Arguments
///
/// * `raw` - The timestamp as the server sent it.
/// * `tz` - The zone to show it in; the page passes the system zone.
///
/// # Returns
///
/// The short form, or `raw` unchanged when it does not parse.
#[must_use]
pub(crate) fn short_timestamp(raw: &str, tz: &TimeZone) -> String {
    raw.parse::<Timestamp>().map_or_else(
        |_| raw.to_owned(),
        |t| {
            t.to_zoned(tz.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

/// The expanded row's lines: start, finish and full counts.
///
/// # Arguments
///
/// * `info` - The batch.
/// * `tz` - The display zone.
///
/// # Returns
///
/// One line per fact.
#[must_use]
pub(crate) fn detail_lines(info: &ImportBatchInfo, tz: &TimeZone) -> Vec<String> {
    let mut lines = vec![format!("Started {}", short_timestamp(&info.started_at, tz))];
    lines.push(info.finished_at.as_deref().map_or_else(
        || "Never finished".to_owned(),
        |f| format!("Finished {}", short_timestamp(f, tz)),
    ));
    match &info.counts {
        Some(c) => {
            lines.push(count_noun(
                &c.new_transactions,
                "new transaction",
                "new transactions",
            ));
            lines.push(count_noun(
                &c.attached_postings,
                "attached posting",
                "attached postings",
            ));
            lines.push(count_noun(
                &c.skipped_postings,
                "skipped posting",
                "skipped postings",
            ));
        }
        None => lines.push("No counts recorded".to_owned()),
    }
    lines
}

/// Whether a `?discard=` value asks to arm the highlighted batch's discard.
///
/// # Arguments
///
/// * `value` - The query value.
///
/// # Returns
///
/// `true` for `"1"`.
#[must_use]
pub(crate) fn arm_requested(value: Option<&str>) -> bool {
    value == Some("1")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::tz::TimeZone;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    use super::*;
    use crate::components::status_pill::Tone;
    use crate::pages::import::fixtures;

    #[rstest]
    #[case("complete", "complete", Tone::Good)]
    #[case("incomplete", "incomplete", Tone::Warn)]
    #[case("discarded", "discarded", Tone::Muted)]
    fn state_pill_cases(#[case] kind: &str, #[case] label: &str, #[case] tone: Tone) {
        let b = fixtures::batch(json!({ "state": { "kind": kind } }));
        assert_eq!(state_pill(&b.state), (label, tone));
        assert_eq!(is_discarded(&b.state), kind == "discarded");
    }

    #[rstest]
    #[case(json!({}), "everyday")]
    #[case(json!({ "profile": null }), "csv")]
    fn batch_name_falls_back_to_the_importer(
        #[case] patch: serde_json::Value,
        #[case] expected: &str,
    ) {
        assert_eq!(batch_name(&fixtures::batch(patch)), expected);
    }

    #[rstest]
    #[case(json!({}), "42 new \u{b7} 3 attached \u{b7} 7 skipped")]
    #[case(json!({ "state": { "kind": "incomplete" }, "counts": null, "finished_at": null }), INCOMPLETE_TEXT)]
    #[case(json!({ "state": { "kind": "discarded" } }), "42 new \u{b7} 3 attached \u{b7} 7 skipped")]
    #[case(json!({ "state": { "kind": "discarded" }, "counts": null }), "no counts recorded")]
    fn outcome_text_cases(#[case] patch: serde_json::Value, #[case] expected: &str) {
        assert_eq!(outcome_text(&fixtures::batch(patch)), expected);
    }

    #[rstest]
    #[case("2026-10-10T01:02:03Z", "2026-10-10 01:02")]
    #[case("not a timestamp", "not a timestamp")]
    fn short_timestamp_cases(#[case] raw: &str, #[case] expected: &str) {
        assert_eq!(short_timestamp(raw, &TimeZone::UTC), expected);
    }

    #[rstest]
    #[case(json!({}), vec![
        "Started 2026-10-10 01:00", "Finished 2026-10-10 01:00",
        "42 new transactions", "3 attached postings", "7 skipped postings",
    ])]
    #[case(json!({ "finished_at": null, "counts": null, "state": { "kind": "incomplete" } }), vec![
        "Started 2026-10-10 01:00", "Never finished", "No counts recorded",
    ])]
    fn detail_lines_cases(#[case] patch: serde_json::Value, #[case] expected: Vec<&str>) {
        assert_eq!(
            detail_lines(&fixtures::batch(patch), &TimeZone::UTC),
            expected
        );
    }

    #[rstest]
    #[case(Some("1"), true)]
    #[case(Some("0"), false)]
    #[case(None, false)]
    fn arm_requested_cases(#[case] value: Option<&str>, #[case] expected: bool) {
        assert_eq!(arm_requested(value), expected);
    }
}
