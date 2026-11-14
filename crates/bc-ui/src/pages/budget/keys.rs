//! Dropdown keys shared by the budget forms, and the intent a new budget
//! defaults to.

use bc_ipc::AccountType;
use bc_ipc::BudgetIntent;
use bc_ipc::RolloverPolicy;

/// Maps a rollover dropdown key to its [`RolloverPolicy`].
#[must_use]
pub(crate) fn rollover_from_key(key: &str) -> RolloverPolicy {
    match key {
        "carry_forward" => RolloverPolicy::CarryForward,
        "cap_at_target" => RolloverPolicy::CapAtTarget,
        _ => RolloverPolicy::ResetToZero,
    }
}

/// Maps a [`RolloverPolicy`] to its dropdown key.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RolloverPolicy is #[non_exhaustive]; the wildcard catches any future variants as reset_to_zero"
)]
pub(crate) fn rollover_key(policy: RolloverPolicy) -> &'static str {
    match policy {
        RolloverPolicy::CarryForward => "carry_forward",
        RolloverPolicy::CapAtTarget => "cap_at_target",
        _ => "reset_to_zero",
    }
}

/// Maps an intent dropdown key to its [`BudgetIntent`].
#[must_use]
pub(crate) fn intent_from_key(key: &str) -> BudgetIntent {
    match key {
        "estimate" => BudgetIntent::Estimate,
        "goal" => BudgetIntent::Goal,
        _ => BudgetIntent::Limit,
    }
}

/// Maps a [`BudgetIntent`] to its dropdown key.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "BudgetIntent is #[non_exhaustive]; the wildcard catches any future variants as limit"
)]
pub(crate) fn intent_key(intent: BudgetIntent) -> &'static str {
    match intent {
        BudgetIntent::Goal => "goal",
        BudgetIntent::Estimate => "estimate",
        _ => "limit",
    }
}

/// Default intent for a newly picked account: `Limit` for `Expense`, `Goal`
/// otherwise.
///
/// Mirrors `bc_models::BudgetIntent::default_for`, which the WASM bundle
/// cannot link; a native test pins the two together.
#[must_use]
pub(crate) fn default_intent_for(account_type: AccountType) -> BudgetIntent {
    if account_type == AccountType::Expense {
        BudgetIntent::Limit
    } else {
        BudgetIntent::Goal
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::BudgetIntent;
    use bc_ipc::RolloverPolicy;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::intent_from_key;
    use super::intent_key;
    use super::rollover_from_key;
    use super::rollover_key;

    #[rstest]
    #[case(RolloverPolicy::CarryForward)]
    #[case(RolloverPolicy::ResetToZero)]
    #[case(RolloverPolicy::CapAtTarget)]
    fn rollover_keys_round_trip(#[case] policy: RolloverPolicy) {
        assert_eq!(rollover_from_key(rollover_key(policy)), policy);
    }

    #[rstest]
    #[case(BudgetIntent::Limit)]
    #[case(BudgetIntent::Goal)]
    #[case(BudgetIntent::Estimate)]
    fn intent_keys_round_trip(#[case] intent: BudgetIntent) {
        assert_eq!(intent_from_key(intent_key(intent)), intent);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[rstest]
    #[case(bc_models::AccountType::Asset)]
    #[case(bc_models::AccountType::Liability)]
    #[case(bc_models::AccountType::Equity)]
    #[case(bc_models::AccountType::Income)]
    #[case(bc_models::AccountType::Expense)]
    fn default_intent_matches_the_model(#[case] account_type: bc_models::AccountType) {
        let model = BudgetIntent::from(bc_models::BudgetIntent::default_for(account_type));
        assert_eq!(super::default_intent_for(account_type.into()), model);
    }
}
