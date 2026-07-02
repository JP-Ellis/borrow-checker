//! App-level currency store: the single served commodity set, shared by amount
//! display (`Num`) and the transaction editor (marker parsing).

#[cfg(target_arch = "wasm32")]
use bc_ipc::CommodityInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

/// Reactive handle to the served currency set, provided once at the shell root.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct CurrencyStore(pub RwSignal<Vec<CommodityInfo>>);

/// Provides an empty [`CurrencyStore`] into context and kicks off a one-shot
/// `list_currencies` load to populate it. Call once, at the shell root.
///
/// # Returns
///
/// The provided [`CurrencyStore`] handle.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn provide_currency_store() -> CurrencyStore {
    let signal = RwSignal::new(Vec::<CommodityInfo>::new());
    let store = CurrencyStore(signal);
    provide_context(store);
    let _resource = LocalResource::new(move || async move {
        if let Ok(list) = bc_ipc::client::list_currencies().await {
            signal.set(list);
        }
    });
    store
}

/// Reads the shared currency set from context, or an empty vec if unavailable.
///
/// # Returns
///
/// A reactive [`RwSignal`] of the served set.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn use_currency_store() -> RwSignal<Vec<CommodityInfo>> {
    use_context::<CurrencyStore>().map_or_else(|| RwSignal::new(Vec::new()), |s| s.0)
}

/// Resolves the raw display symbol + placement for a currency code from the served set.
/// Returns `(None, false)` when the code is unknown or has no symbol, so
/// [`bc_ipc::Amount::format_short`] falls back to its code-with-space form.
///
/// # Arguments
///
/// * `code` - The currency code to resolve.
/// * `currencies` - The served commodity set to search.
///
/// # Returns
///
/// The resolved symbol and whether it should be placed after the amount.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn short_symbol(code: &str, currencies: &[CommodityInfo]) -> (Option<String>, bool) {
    currencies
        .iter()
        .find(|c| c.code.eq_ignore_ascii_case(code))
        .map_or((None, false), |c| (c.symbol.clone(), c.symbol_after))
}
