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
