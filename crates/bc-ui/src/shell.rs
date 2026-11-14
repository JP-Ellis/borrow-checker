//! Shell components: [`ConsoleShell`] wrapper and [`top_bar::TopBar`].
//! The [`palette`] module contains a [`palette::CommandPalette`] wired into [`ConsoleShell`].

pub mod palette;
#[cfg(target_arch = "wasm32")]
pub mod top_bar;

#[cfg(target_arch = "wasm32")]
use std::sync::atomic::AtomicBool;
#[cfg(target_arch = "wasm32")]
use std::sync::atomic::Ordering;

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
#[cfg(target_arch = "wasm32")]
use bc_ipc::SettingsInfo;
#[cfg(target_arch = "wasm32")]
use leptos::ev;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos_router::NavigateOptions;
#[cfg(target_arch = "wasm32")]
use leptos_router::components::Outlet;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_location;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_navigate;
#[cfg(target_arch = "wasm32")]
pub use top_bar::TopBar;

#[cfg(target_arch = "wasm32")]
use crate::components::new_transaction::NewTransactionOverlay;
#[cfg(target_arch = "wasm32")]
use crate::components::new_transaction::provide_new_tx_opener;
#[cfg(target_arch = "wasm32")]
use crate::components::toast::ToastAction;
#[cfg(target_arch = "wasm32")]
use crate::components::toast::ToastKind;
#[cfg(target_arch = "wasm32")]
use crate::filter_ctx::Edit;
#[cfg(target_arch = "wasm32")]
use crate::period_defaults::fallback_settings;
#[cfg(target_arch = "wasm32")]
use crate::period_defaults::provide_settings;
#[cfg(target_arch = "wasm32")]
use crate::shell::palette::CommandPalette;
#[cfg(target_arch = "wasm32")]
use crate::url_state::History;
#[cfg(target_arch = "wasm32")]
use crate::url_state::LAST_LOCATION_KEY;
#[cfg(target_arch = "wasm32")]
use crate::url_state::TAB_SEEN_KEY;
#[cfg(target_arch = "wasm32")]
use crate::url_state::UrlState;

/// Set once the shell has considered a cold-start replay, so a remount (a
/// trip through a QA route) never replays again.
#[cfg(target_arch = "wasm32")]
static REPLAY_CONSIDERED: AtomicBool = AtomicBool::new(false);

/// Full-app wrapper: loads the settings, then mounts [`ConsoleShellInner`].
///
/// Nothing renders until `get_settings` resolves. The URL canonicaliser and
/// the last-location mirror would otherwise persist a URL snapped against the
/// fallback financial-year start. On a load error the shell runs on
/// [`fallback_settings`] and reports the error as a toast.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn ConsoleShell() -> impl IntoView {
    let settings = LocalResource::new(bc_ipc::client::get_settings);
    move || {
        settings.get().map(|result| match result {
            Ok(info) => view! { <ConsoleShellInner settings=info /> }.into_any(),
            Err(e) => view! {
                <ConsoleShellInner
                    settings=fallback_settings()
                    load_error=format!("Could not load settings; using defaults: {e}")
                />
            }
            .into_any(),
        })
    }
}

/// The shell body: renders [`TopBar`] above the routed content area.
///
/// Provides the settings context first, so every store below reads the
/// configured period defaults. Owns the palette's open and edit signals,
/// provides the palette opener, and wires the global ⌘K / Ctrl+K shortcut.
/// Owns the new-transaction overlay and provides its opener.
/// Child routes render via [`Outlet`].
///
/// # Arguments
///
/// * `settings` - The loaded settings, or the fallback on a load error.
/// * `load_error` - The toast message for a settings load error.
#[cfg(target_arch = "wasm32")]
#[component]
fn ConsoleShellInner(
    /// The loaded settings, or the fallback on a load error.
    settings: SettingsInfo,
    /// The toast message for a settings load error.
    #[prop(optional, into)]
    load_error: Option<String>,
) -> impl IntoView {
    let defaults = provide_settings(settings);
    let _currency_store = crate::currency_ctx::provide_currency_store();
    let _meta_key_store = crate::meta_keys_ctx::provide_meta_key_store();
    let filter_store = crate::filter_ctx::provide_filter_store();
    let toasts = crate::components::toast::provide_toast_store();
    let _query_catalog = crate::query_catalog_ctx::provide_query_catalog();
    let palette_open = RwSignal::new(false);
    let palette_edit = RwSignal::new(Edit::append());
    let open_palette = Callback::new(move |edit: Edit| {
        palette_edit.set(edit);
        palette_open.set(true);
    });
    crate::filter_ctx::provide_palette_opener(open_palette);
    if let Some(message) = load_error {
        // An effect runs after mount, so the toast host is already listening.
        Effect::new(move |_| {
            untrack(|| toasts.push(ToastKind::Error, message.clone(), None));
        });
    }
    let new_tx_open = RwSignal::new(None::<Option<AccountRef>>);
    let open_new_tx = Callback::new(move |seed: Option<AccountRef>| {
        if new_tx_open.with_untracked(Option::is_none) {
            new_tx_open.set(Some(seed));
        }
    });
    provide_new_tx_opener(open_new_tx);

    // MARK: Cold start

    // A restart or a new tab opens bare `/`; replay the last mirrored
    // location. The saved value is read now, before the mirror's first run
    // overwrites it. The per-tab mark keeps a reload from replaying another
    // tab's location.
    let location = use_location();
    let replay = if REPLAY_CONSIDERED.swap(true, Ordering::Relaxed) {
        None
    } else {
        let saved = crate::storage::get(LAST_LOCATION_KEY);
        let tab_seen = crate::storage::session_get(TAB_SEEN_KEY).is_some();
        crate::storage::session_set(TAB_SEEN_KEY, "1");
        location.pathname.with_untracked(|path| {
            location.search.with_untracked(|search| {
                crate::url_state::should_replay(path, search, saved.as_deref(), tab_seen)
            })
        })
    };
    Effect::new(move |_| {
        let entry = location.pathname.with(|path| {
            location
                .search
                .with(|search| crate::url_state::mirror_entry(path, search))
        });
        if let Some(entry) = entry {
            crate::storage::set(LAST_LOCATION_KEY, &entry);
        }
    });
    if let Some(target) = replay {
        let navigate = use_navigate();
        Effect::new(move |_| {
            untrack(|| {
                navigate(
                    &target,
                    NavigateOptions {
                        replace: true,
                        scroll: false,
                        ..NavigateOptions::default()
                    },
                );
                let (restored, _) =
                    UrlState::from_search(crate::url_state::search_of(&target), defaults);
                if let Some(message) = crate::url_state::restore_message(&restored) {
                    toasts.push(
                        ToastKind::Info,
                        message,
                        Some(ToastAction {
                            label: "Clear".to_owned(),
                            on_activate: Callback::new(move |()| {
                                filter_store.set(UrlState::default(), History::Push);
                            }),
                        }),
                    );
                }
            });
        });
    }

    /* Global ⌘K / Ctrl+K shortcut: closes an open palette, else opens a blank one. */
    let handle = window_event_listener(ev::keydown, move |ke| {
        if ke.key() == "k" && (ke.meta_key() || ke.ctrl_key()) {
            if palette_open.get_untracked() {
                palette_open.set(false);
            } else {
                open_palette.run(Edit::append());
            }
            ke.prevent_default();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="console-shell">
            <TopBar
                on_search=Callback::new(move |()| open_palette.run(Edit::append()))
                on_new_transaction=Callback::new(move |()| open_new_tx.run(None))
            />
            <main class="console-main">
                <Outlet />
            </main>
            <NewTransactionOverlay open=new_tx_open />
            <CommandPalette
                open=palette_open.read_only()
                on_close=Callback::new(move |()| palette_open.set(false))
                edit=palette_edit.read_only()
            />
            <crate::components::toast::ToastHost />
        </div>
    }
}
