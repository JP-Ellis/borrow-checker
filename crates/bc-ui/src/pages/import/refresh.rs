//! The app-wide stores a commit or discard can make stale.

use bc_ipc::CommodityInfo;
use bc_ipc::MetaKeyDefDto;
use leptos::prelude::*;

use crate::query_catalog_ctx::QueryCatalogStore;

/// Handles to the shell's shared stores, captured while context is in reach.
#[derive(Clone, Copy)]
pub(crate) struct SharedStores {
    /// The palette's query catalog: accounts, tags, payees.
    catalog: QueryCatalogStore,
    /// The served commodity set.
    currencies: RwSignal<Vec<CommodityInfo>>,
    /// The metadata key registry.
    meta_keys: RwSignal<Vec<MetaKeyDefDto>>,
}

impl SharedStores {
    /// Reads the stores from context. Call during component setup.
    #[must_use]
    pub(crate) fn from_context() -> Self {
        Self {
            catalog: crate::query_catalog_ctx::use_query_catalog(),
            currencies: crate::currency_ctx::use_currency_store(),
            meta_keys: crate::meta_keys_ctx::use_meta_key_store(),
        }
    }

    /// Refetches every store. A failed fetch keeps the store's last value.
    pub(crate) fn refresh(self) {
        self.catalog.refresh();
        leptos::task::spawn_local(async move {
            if let Ok(list) = bc_ipc::client::list_currencies().await {
                self.currencies.set(list);
            }
            if let Ok(list) = bc_ipc::client::list_metadata_keys().await {
                self.meta_keys.set(list);
            }
        });
    }
}
