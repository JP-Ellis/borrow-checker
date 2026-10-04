//! [`TopBar`] navigation component (52px fixed height).

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

use crate::components::filter_chips::FilterChips;

/// Application top bar: logo, wordmark, nav tabs, filter chips, search,
/// avatar.
///
/// Active tab detection matches the current route pathname against each tab's
/// href prefix.
///
/// # Arguments
///
/// * `on_search` - Callback invoked when the user clicks the search button.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
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

    let primary: &[(&str, &str)] = &[
        ("dashboard", "/"),
        ("accounts", "/accounts"),
        ("budget", "/budget"),
    ];
    let overflow: &[(&str, &str)] = &[
        ("reports", "/reports"),
        ("plugins", "/plugins"),
        ("settings", "/settings"),
    ];

    // `testid` prefixes the name, so the menu's copies of the overflow tabs
    // stay distinct from the inline ones.
    let tab =
        move |name: &'static str, href: &'static str, extra: &'static str, testid: &'static str| {
            view! {
                <A
                    href=href
                    attr:class=move || {
                        let active = if is_active(href) { " top-bar__tab--active" } else { "" };
                        format!("top-bar__tab{extra}{active}")
                    }
                    attr:data-testid=format!("{testid}{name}")
                >
                    {name}
                </A>
            }
        };

    // The overflow menu closes on any route change and on any click inside
    // it; a link click does not light-dismiss a popover, and choosing the
    // current route changes no path.
    let more_ref = NodeRef::<leptos::html::Div>::new();
    let close_more = move || {
        if let Some(el) = more_ref.get_untracked() {
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "hide_popover() returns Result<(), JsValue>; errors are benign"
            )]
            let _ = el.hide_popover();
        }
    };
    Effect::new(move |_| {
        location.pathname.track();
        close_more();
    });
    let overflow_active = move || overflow.iter().any(|&(_, href)| is_active(href));

    let store = crate::filter_ctx::use_filter_store();
    let filter_count = Signal::derive(move || {
        crate::filter_ctx::chips_from_filter(&store.filter.get(), &store.labels.get()).len()
    });

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
                {primary.iter().map(|&(n, h)| tab(n, h, "", "nav-")).collect::<Vec<_>>()}
                {overflow
                    .iter()
                    .map(|&(n, h)| tab(n, h, " top-bar__tab--overflow", "nav-"))
                    .collect::<Vec<_>>()}
                <button
                    class=move || {
                        if overflow_active() {
                            "top-bar__tab top-bar__more top-bar__tab--active"
                        } else {
                            "top-bar__tab top-bar__more"
                        }
                    }
                    popovertarget="bc-nav-more"
                    aria-label="more pages"
                    data-testid="nav-more"
                >
                    "⋯"
                </button>
                <div
                    id="bc-nav-more"
                    class="top-bar__menu"
                    popover="auto"
                    node_ref=more_ref
                    on:click=move |_| close_more()
                >
                    {overflow.iter().map(|&(n, h)| tab(n, h, "", "nav-more-")).collect::<Vec<_>>()}
                </div>
            </nav>

            <div class="top-bar__filters">
                <FilterChips />
            </div>
            <Show when=move || { filter_count.get() > 0 }>
                <button
                    class="top-bar__filter-more"
                    popovertarget="bc-filter-more"
                    data-testid="filter-more"
                    aria-label=move || match filter_count.get() {
                        1 => "1 filter".to_owned(),
                        n => format!("{n} filters"),
                    }
                >
                    {move || format!("{} ▾", filter_count.get())}
                </button>
                <div id="bc-filter-more" class="top-bar__menu" popover="auto">
                    <FilterChips testid="filter-more-chips" />
                </div>
            </Show>

            <button
                class="top-bar__search"
                aria-label="open command palette (⌘K)"
                on:click=move |_| on_search.run(())
            >
                <span class="top-bar__search-icon" aria-hidden="true">
                    "⌕"
                </span>
                <span class="top-bar__search-prompt">
                    "› search payee, account, or run a command…"
                </span>
                <kbd class="top-bar__kbd">"⌘K"</kbd>
            </button>

            <div class="top-bar__avatar" aria-label="user: jp">
                "jp"
            </div>
        </header>
    }
}
