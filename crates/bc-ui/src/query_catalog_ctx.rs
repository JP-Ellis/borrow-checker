//! App-level query catalog: the ledger facts the palette and chips resolve
//! query text against, fetched whole from the server.
//!
//! It is fetched at start-up, again whenever the window regains focus, and on
//! [`QueryCatalogStore::refresh`]. The palette refreshes it on every open, so
//! an account renamed elsewhere resolves as the server will see it. A failed
//! fetch logs a warning to the console and keeps the previous snapshot.

use bc_query::catalog::Snapshot;
use leptos::ev;
use leptos::prelude::*;

use crate::shell::palette::model::Load;

/// Reactive handle to the query catalog, provided once at the shell root.
#[derive(Clone, Copy)]
#[expect(
    clippy::partial_pub_fields,
    reason = "readers watch the snapshot; only refresh bumps the version"
)]
pub struct QueryCatalogStore {
    /// The latest catalog; `None` until the first load lands.
    pub snapshot: RwSignal<Option<Snapshot>>,
    /// Whether the latest fetch failed. It tells "still loading" from "could
    /// not load" while `snapshot` is `None`.
    pub failed: RwSignal<bool>,
    /// Bumped to fetch the catalog again.
    version: RwSignal<u32>,
}

impl QueryCatalogStore {
    /// Whether the catalog is loaded, still loading, or failed to load.
    #[must_use]
    pub fn load(&self) -> Load {
        Load::from_store(self.snapshot.with(Option::is_some), self.failed.get())
    }

    /// [`Self::load`], without subscribing the caller.
    #[must_use]
    pub fn load_untracked(&self) -> Load {
        Load::from_store(
            self.snapshot.with_untracked(Option::is_some),
            self.failed.get_untracked(),
        )
    }

    /// Fetches the catalog again.
    pub fn refresh(&self) {
        self.version.update(|v| *v = v.wrapping_add(1));
    }
}

/// Provides the [`QueryCatalogStore`] into context and loads it, again on
/// every window focus and refresh. Call once, at the shell root.
///
/// # Returns
///
/// The provided store.
#[must_use]
pub fn provide_query_catalog() -> QueryCatalogStore {
    let store = QueryCatalogStore {
        snapshot: RwSignal::new(None),
        failed: RwSignal::new(false),
        version: RwSignal::new(0),
    };
    provide_context(store);
    let _resource = LocalResource::new(move || {
        store.version.track();
        async move {
            match bc_ipc::client::query_catalog().await {
                Ok(catalog) => {
                    store.snapshot.set(Some(Snapshot::from(catalog)));
                    store.failed.set(false);
                }
                Err(e) => {
                    leptos::logging::warn!("query catalog fetch failed: {e:?}");
                    store.failed.set(true);
                }
            }
        }
    });
    let handle = window_event_listener(ev::focus, move |_| store.refresh());
    on_cleanup(move || handle.remove());
    store
}

/// Provides a fixed catalog that is never fetched, for QA routes.
///
/// # Arguments
///
/// * `snapshot` - The catalog to serve.
///
/// # Returns
///
/// The provided store.
#[cfg(debug_assertions)]
#[must_use]
pub fn provide_fixed_query_catalog(snapshot: Snapshot) -> QueryCatalogStore {
    let store = QueryCatalogStore {
        snapshot: RwSignal::new(Some(snapshot)),
        failed: RwSignal::new(false),
        version: RwSignal::new(0),
    };
    provide_context(store);
    store
}

/// Reads the [`QueryCatalogStore`] from context, or an empty, never-loaded one.
///
/// # Returns
///
/// The store from context, or a detached empty one.
#[must_use]
pub fn use_query_catalog() -> QueryCatalogStore {
    use_context::<QueryCatalogStore>().unwrap_or_else(|| QueryCatalogStore {
        snapshot: RwSignal::new(None),
        failed: RwSignal::new(false),
        version: RwSignal::new(0),
    })
}
