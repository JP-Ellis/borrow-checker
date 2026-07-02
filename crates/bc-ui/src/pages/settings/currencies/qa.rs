//! QA showcase for [`super::CurrenciesPanel`] — mock currency set, no IPC.

use bc_ipc::CommodityInfo;
use leptos::prelude::*;

use super::CurrenciesPanel;
use crate::currency_ctx::CurrencyStore;

/// Mock commodity set: USD, AUD, EUR + BTC (non-ISO, symbol-after).
fn mock_currencies() -> Vec<CommodityInfo> {
    vec![
        CommodityInfo::new(
            "commodity-usd",
            "USD",
            Some("$".to_owned()),
            vec!["US$".to_owned()],
            2,
            true,
            false,
        ),
        CommodityInfo::new(
            "commodity-aud",
            "AUD",
            Some("A$".to_owned()),
            vec!["AU$".to_owned()],
            2,
            true,
            false,
        ),
        CommodityInfo::new(
            "commodity-eur",
            "EUR",
            Some("€".to_owned()),
            vec![],
            2,
            true,
            false,
        ),
        CommodityInfo::new(
            "commodity-btc",
            "BTC",
            Some("₿".to_owned()),
            vec!["XBT".to_owned()],
            8,
            false,
            true,
        ),
    ]
}

/// Renders [`CurrenciesPanel`] against a mock [`CurrencyStore`] seeded in
/// context, so the panel renders without going through Tauri IPC.
#[component]
pub fn CurrenciesPanelQa() -> impl IntoView {
    let signal = RwSignal::new(mock_currencies());
    provide_context(CurrencyStore(signal));

    view! { <CurrenciesPanel /> }
}
