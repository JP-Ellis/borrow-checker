//! [`TopBar`] navigation component (52px fixed height).

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

use crate::components::filter_chips::FilterChips;
use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;

/// Application top bar: logo, wordmark, nav tabs, search, sync pill, avatar.
///
/// Active tab detection matches the current route pathname against each tab's
/// href prefix. The sync status pill is a static placeholder pending Phase 2
/// IPC wiring.
///
/// # Arguments
///
/// * `on_search` - Callback invoked when the user clicks the search button.
#[component]
pub fn TopBar(
    /// Called when the user clicks the search button or triggers ⌘K.
    on_search: Callback<()>,
) -> impl IntoView {
    let location = use_location();

    let is_active = move |href: &'static str| {
        let p = location.pathname.get();
        if href == "/" {
            p == "/"
        } else {
            p.starts_with(href)
        }
    };

    let tabs: &[(&str, &str)] = &[
        ("dashboard", "/"),
        ("accounts", "/accounts"),
        ("budget", "/budget"),
        ("reports", "/reports"),
        ("plugins", "/plugins"),
        ("settings", "/settings"),
    ];

    view! {
        <header class="top-bar">
            <div class="top-bar__logo">
                <span class="top-bar__logo-mark" aria-hidden="true">
                    "$"
                </span>
                <span class="top-bar__wordmark" aria-label="borrow-checker">
                    "borrow"
                    <span class="top-bar__hyphen">"-"</span>
                    "checker"
                </span>
            </div>

            <nav class="top-bar__nav" aria-label="main navigation">
                {tabs
                    .iter()
                    .map(|&(name, href)| {
                        view! {
                            <A
                                href=href
                                attr:class=move || {
                                    if is_active(href) {
                                        "top-bar__tab top-bar__tab--active"
                                    } else {
                                        "top-bar__tab"
                                    }
                                }
                                attr:data-testid=(href == "/accounts").then_some("nav-accounts")
                            >
                                {name}
                            </A>
                        }
                    })
                    .collect::<Vec<_>>()}
            </nav>

            <FilterChips />
            <StrictnessToggle />

            <button
                class="top-bar__search"
                aria-label="open command palette (⌘K)"
                on:click=move |_| on_search.run(())
            >
                <span class="top-bar__search-prompt">
                    "› search payee, account, or run a command…"
                </span>
                <kbd class="top-bar__kbd">"⌘K"</kbd>
            </button>

            <StatusPill label="pending".to_owned() tone=Tone::Warn />

            <div class="top-bar__avatar" aria-label="user: jp">
                "jp"
            </div>
        </header>
    }
}

/// Lenient/strict presentation toggle for the global filter. Visible only while
/// a filter is active (matching the chips' visibility). Writes
/// `FilterStore.strictness`.
#[component]
fn StrictnessToggle() -> impl IntoView {
    use crate::filter_ctx::Strictness;

    let store = crate::filter_ctx::use_filter_store();
    let active = Signal::derive(move || store.filter.with(crate::filter_ctx::filter_is_active));
    let is_strict = Signal::derive(move || store.strictness.get() == Strictness::Strict);

    view! {
        <Show when=move || active.get()>
            <button
                class="top-bar__strictness"
                data-testid="strictness-toggle"
                aria-pressed=move || is_strict.get().to_string()
                on:click=move |_| {
                    store
                        .strictness
                        .update(|s| {
                            *s = if *s == Strictness::Strict {
                                Strictness::Lenient
                            } else {
                                Strictness::Strict
                            };
                        });
                }
            >
                {move || store.strictness.get().label()}
            </button>
        </Show>
    }
}
