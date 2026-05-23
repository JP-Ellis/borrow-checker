//! QA shell — replaces [`crate::shell::ConsoleShell`] for all `/__test/*` routes.
//!
//! Provides breadcrumb navigation and a light/dark toggle (`data-theme` on `<html>`).
//! The content area fills the full viewport; use your browser's responsive design mode
//! (`DevTools`) to test at specific widths.

use leptos::prelude::*;
use leptos::web_sys;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_location;
use stylance::import_style;

import_style!(style, "shell.module.scss");

/// QA shell layout component.
#[component]
pub fn QaShell() -> impl IntoView {
    // MARK: Color scheme toggle
    let (dark, set_dark) = signal(false);

    let toggle_theme = move |_: web_sys::MouseEvent| {
        let next = !dark.get_untracked();
        set_dark.set(next);
        if let Some(html) = document().document_element() {
            let attr = if next { "dark" } else { "light" };
            drop(html.set_attribute("data-theme", attr));
        }
    };

    // MARK: Breadcrumbs
    let location = use_location();
    let breadcrumbs = Signal::derive(move || {
        location
            .pathname
            .get()
            .split('/')
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    });

    view! {
        <div class=style::shell>
            <div class=style::topbar>
                <nav class=style::breadcrumbs aria-label="QA navigation">
                    {move || {
                        let crumbs = breadcrumbs.get();
                        let last_idx = crumbs.len().saturating_sub(1);
                        crumbs
                            .into_iter()
                            .enumerate()
                            .flat_map(|(i, segment)| {
                                let path = format!(
                                    "/{}",
                                    breadcrumbs
                                        .get()
                                        .iter()
                                        .take(i.saturating_add(1))
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join("/"),
                                );
                                let is_last = i == last_idx;
                                let sep = (i > 0)
                                    .then(|| {
                                        view! { <span class=style::breadcrumb_sep>" / "</span> }
                                            .into_any()
                                    });
                                let node = if is_last {
                                    view! { <span class=style::breadcrumb_current>{segment}</span> }
                                        .into_any()
                                } else {
                                    view! {
                                        <a class=style::breadcrumb_link href=path>
                                            {segment}
                                        </a>
                                    }
                                        .into_any()
                                };
                                sep.into_iter().chain(core::iter::once(node))
                            })
                            .collect::<Vec<_>>()
                    }}
                </nav>

                <div class=style::controls>
                    <button class=style::scheme_btn on:click=toggle_theme title="toggle light/dark">
                        {move || if dark.get() { "light" } else { "dark" }}
                    </button>
                </div>
            </div>

            <main class=style::main>
                <Outlet />
            </main>
        </div>
    }
}
