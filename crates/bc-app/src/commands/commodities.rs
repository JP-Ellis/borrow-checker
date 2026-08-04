//! Tauri command handlers for commodity/currency queries.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

use crate::AppState;

/// Lists all active commodities/currencies for the UI.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the database query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_currencies(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::CommodityInfo>, bc_ipc::BcError> {
    let list = state
        .commodities
        .list_all()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(list.iter().map(bc_ipc::CommodityInfo::from).collect())
}

/// Creates a new commodity/currency.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id or a marker is invalid,
/// or [`bc_ipc::BcError::Internal`] if the database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_currency(
    info: bc_ipc::CommodityInfo,
    state: State<'_, AppState>,
) -> Result<bc_ipc::CommodityInfo, bc_ipc::BcError> {
    let c = bc_models::Commodity::try_from(info)?;
    let stored = state.commodities.create(&c).await?;
    Ok(bc_ipc::CommodityInfo::from(&stored))
}

/// Carries forward fields that [`bc_ipc::CommodityInfo`] cannot express.
///
/// `CommodityInfo` has no `name`, `exchange`, `description`, `active_from`, or
/// `active_until`, so a DTO-derived [`bc_models::Commodity`] always has those
/// five fields set to `None`. Persisting it as-is would null them out on every
/// GUI edit. This merges `stored`'s values for those five fields into
/// `incoming`, while `incoming` supplies everything else (`symbol`, `aliases`,
/// `decimals`, `is_iso`, `symbol_after`).
///
/// # Arguments
///
/// * `incoming` - The commodity built from the incoming DTO.
/// * `stored` - The currently persisted commodity with the same id.
///
/// # Returns
///
/// The merged commodity to persist.
fn carry_forward_unmapped_fields(
    incoming: &bc_models::Commodity,
    stored: &bc_models::Commodity,
) -> bc_models::Commodity {
    bc_models::Commodity::builder()
        .id(incoming.id().clone())
        .code(incoming.code().to_owned())
        .maybe_name(stored.name())
        .maybe_exchange(stored.exchange())
        .maybe_description(stored.description())
        .aliases(incoming.aliases().to_vec())
        .decimals(incoming.decimals())
        .is_iso(incoming.is_iso())
        .symbol_after(incoming.symbol_after())
        .maybe_symbol(incoming.symbol())
        .maybe_active_from(stored.active_from())
        .maybe_active_until(stored.active_until())
        .build()
}

/// Updates an existing commodity/currency (its code is immutable).
///
/// Since [`bc_ipc::CommodityInfo`] cannot express `name`, `exchange`,
/// `description`, `active_from`, or `active_until`, the stored commodity is
/// looked up and those five fields are carried forward so this GUI-driven
/// update cannot null them out.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id or a marker is invalid,
/// or if the id does not match any stored commodity, or
/// [`bc_ipc::BcError::Internal`] if the database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn update_currency(
    info: bc_ipc::CommodityInfo,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let incoming = bc_models::Commodity::try_from(info)?;
    let all = state
        .commodities
        .list_all()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let stored = all
        .iter()
        .find(|c| c.id() == incoming.id())
        .ok_or_else(|| {
            bc_ipc::BcError::Validation(format!(
                "no commodity with id '{}' to update",
                incoming.id()
            ))
        })?;
    let merged = carry_forward_unmapped_fields(&incoming, stored);
    Ok(state.commodities.update(&merged).await?)
}

/// Deletes a commodity/currency, refusing if it is still referenced.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the id is invalid or the
/// commodity is still referenced, or [`bc_ipc::BcError::Internal`] if the
/// database write fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_currency(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let cid = id
        .parse::<bc_models::CommodityId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid commodity id '{id}': {e}")))?;
    Ok(state.commodities.delete(&cid).await?)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::carry_forward_unmapped_fields;

    #[test]
    fn merge_preserves_stored_unmapped_fields_and_takes_incoming_rest() {
        let id = bc_models::CommodityId::new();
        let stored = bc_models::Commodity::builder()
            .id(id.clone())
            .code("AUD")
            .name("Australian Dollar")
            .exchange("ASX")
            .description("Official currency of Australia")
            .symbol("A$")
            .aliases(vec!["AU$".to_owned()])
            .decimals(2)
            .is_iso(true)
            .symbol_after(false)
            .active_from(jiff::civil::date(2000, 1, 1))
            .active_until(jiff::civil::date(2099, 12, 31))
            .build();

        let incoming = bc_models::Commodity::builder()
            .id(id.clone())
            .code("AUD")
            .symbol("AU$")
            .aliases(vec!["AUS$".to_owned()])
            .decimals(0)
            .is_iso(false)
            .symbol_after(true)
            .build();

        let merged = carry_forward_unmapped_fields(&incoming, &stored);

        assert_eq!(merged.id(), &id);
        assert_eq!(merged.code(), "AUD");
        assert_eq!(merged.name(), Some("Australian Dollar"));
        assert_eq!(merged.exchange(), Some("ASX"));
        assert_eq!(merged.description(), Some("Official currency of Australia"));
        assert_eq!(merged.active_from(), Some(jiff::civil::date(2000, 1, 1)));
        assert_eq!(merged.active_until(), Some(jiff::civil::date(2099, 12, 31)));

        assert_eq!(merged.symbol(), Some("AU$"));
        assert_eq!(merged.aliases(), &["AUS$".to_owned()]);
        assert_eq!(merged.decimals(), 0);
        assert!(!merged.is_iso());
        assert!(merged.symbol_after());
    }
}
