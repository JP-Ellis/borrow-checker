// Target field logic for the new-budget and revision forms. Kept separate
// from `mod.rs` so this pure logic runs under a native `cargo nextest`
// (`mod.rs` sits under a wasm-only module tree and never runs there); see
// `components_tests` in `main.rs`.

use bc_ipc::BudgetRevisionView;
use rust_decimal::Decimal;

/// Text the target field starts with: the expression if any, else the stored value.
#[must_use]
pub(crate) fn initial_target_text(rev: Option<&BudgetRevisionView>) -> String {
    rev.and_then(|r| {
        r.target_expr
            .clone()
            .or_else(|| r.target.as_ref().map(|t| t.value.to_string()))
    })
    .unwrap_or_default()
}

/// The evaluated value to preview beside the field, if the text is an expression.
///
/// # Errors
///
/// Returns the evaluator's message when the text does not evaluate.
pub(crate) fn preview(text: &str) -> Result<Option<Decimal>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let (value, expr) = bc_expr::split(trimmed).map_err(|e| e.message().to_owned())?;
    Ok(expr.map(|_| value))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn view(target_expr: Option<&str>) -> BudgetRevisionView {
        BudgetRevisionView::builder()
            .id("r")
            .effective_from(jiff::civil::Date::constant(2026, 1, 1))
            .period(bc_ipc::Period::Monthly)
            .period_label("monthly")
            .rollover(bc_ipc::RolloverPolicy::ResetToZero)
            .intent(bc_ipc::BudgetIntent::Goal)
            .target(bc_ipc::Amount::new(dec!(-500), "AUD"))
            .maybe_target_expr(target_expr.map(str::to_owned))
            .build()
    }

    #[rstest]
    #[case("", Ok(None))]
    #[case("250.00", Ok(None))]
    #[case("(30.00 / 4)", Ok(Some(dec!(7.5))))]
    fn preview_cases(#[case] text: &str, #[case] expected: Result<Option<Decimal>, String>) {
        assert_eq!(preview(text), expected);
    }

    #[test]
    fn preview_reports_division_by_zero() {
        assert_eq!(preview("1 / 0"), Err("division by zero".to_owned()));
    }

    #[test]
    fn initial_text_keeps_sign_and_prefers_expression() {
        assert_eq!(initial_target_text(Some(&view(None))), "-500");
        assert_eq!(
            initial_target_text(Some(&view(Some("(-6000 / 12)")))),
            "(-6000 / 12)"
        );
        assert_eq!(initial_target_text(None), "");
    }
}
