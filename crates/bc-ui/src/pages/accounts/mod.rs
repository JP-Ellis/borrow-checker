//! Accounts page — account tree sidebar and transaction register.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

pub(crate) mod components;
#[cfg(target_arch = "wasm32")]
pub(crate) mod dashboard;
pub(crate) mod figure;
#[cfg(target_arch = "wasm32")]
pub(crate) mod period_notify;
pub(crate) mod query;
pub(crate) mod register_pages;
#[cfg(target_arch = "wasm32")]
pub(crate) mod rollup;
pub(crate) mod tree;

#[cfg(target_arch = "wasm32")]
use bc_ipc::NewTransaction;
#[cfg(target_arch = "wasm32")]
use components::add_transaction::AddTransactionForm;
#[cfg(target_arch = "wasm32")]
use components::sidebar::AccountSidebar;
#[cfg(target_arch = "wasm32")]
use components::sticky_bar::StickyAccountBar;
#[cfg(target_arch = "wasm32")]
use components::transaction_register::TransactionRegister;
#[cfg(target_arch = "wasm32")]
use dashboard::AccountDashboard;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_params_map;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::pages::accounts::register_pages::BalanceMode;
#[cfg(target_arch = "wasm32")]
use crate::pages::accounts::register_pages::LoadedRegister;

#[cfg(target_arch = "wasm32")]
import_style!(style, "accounts.module.scss");

/// Accounts page — sidebar + scrollable account dashboard and register.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn Accounts() -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let filter_store = crate::filter_ctx::use_filter_store();
    let params = use_params_map();
    let selected_id = Signal::derive(move || params.with(|p| p.get("id")));

    let include_descendants = rollup::create_include_descendants();
    provide_context(include_descendants);
    let include_descendants = include_descendants.0;

    // Expanded sidebar ids: persisted, seeded with the roots so the first
    // level is visible, and grown by the selected account's ancestors. An
    // absent key is the only "never seeded" state: a persisted empty set is a
    // deliberate collapse-all and stays that way.
    let stored_expanded = components::sidebar::load_expanded();
    let seed_pending = StoredValue::new(stored_expanded.is_none());
    let expanded = RwSignal::new(stored_expanded.unwrap_or_default());
    Effect::new(move |_| {
        expanded.with(|e| {
            // Nothing is written until the roots have been seeded: an empty
            // set persisted before the account list resolves would read as a
            // collapse-all on the next mount.
            if !seed_pending.get_value() {
                components::sidebar::save_expanded(e);
            }
        });
    });

    // Initialise collapsed on narrow viewports (≤ 480px, matching $bp-sm).
    let sidebar_collapsed = {
        let narrow = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .is_some_and(|w| w <= 480.0_f64);
        RwSignal::new(narrow)
    };
    let toggle_sidebar = move |_: web_sys::MouseEvent| {
        sidebar_collapsed.update(|c| *c = !*c);
    };

    let main_ref = NodeRef::<leptos::html::Div>::new();
    let dashboard_scrolled = RwSignal::new(false);

    // MARK: Live data

    // Monotonic counter bumped after any mutation that changes account or
    // transaction state (create, amend, void, import, …).  All resources that
    // show derived data subscribe to this signal so they re-fetch automatically
    // when anything changes — new actions only need to bump the counter.
    let data_version = RwSignal::new(0_u32);

    // LocalResource is required because bc_ipc::client futures are not Send
    // (they use js_sys::futures::JsFuture internally).
    let accounts_resource = LocalResource::new(move || {
        data_version.get(); // re-fetch whenever any mutation lands
        bc_ipc::client::list_accounts()
    });

    // Roots open one level on first load; ancestors of the selection always open.
    Effect::new(move |_| {
        let Some(Ok(nodes)) = accounts_resource.get() else {
            return;
        };
        let roots: Vec<String> = tree::ordered_roots(&nodes)
            .into_iter()
            .map(|n| n.id)
            .collect();
        let ancestors = selected_id
            .get()
            .map(|id| tree::ancestors_of(&nodes, &id))
            .unwrap_or_default();
        let seed_roots = seed_pending.get_value();
        seed_pending.set_value(false);
        let to_add: Vec<String> = expanded.with_untracked(|e| {
            seed_roots
                .then(|| roots.iter().cloned())
                .into_iter()
                .flatten()
                .chain(ancestors)
                .filter(|id| !e.contains(id))
                .collect()
        });
        if !to_add.is_empty() {
            expanded.update(|e| e.extend(to_add));
        }
    });

    // Page-level display window, shared with TransactionRegister and
    // AccountDashboard. Opens on the whole ledger; nothing picks a period.
    let window = RwSignal::new(crate::components::period_nav::DisplayWindow::AllTime);

    // MARK: Register paging

    let register = RwSignal::new(LoadedRegister::default());

    // Everything a page request needs except cursor and limit. `None` while no
    // account is selected. Tracks every reset trigger.
    let request_base = Signal::derive(move || {
        data_version.get();
        let id = selected_id.get()?;
        let win = window.get();
        let eff = filter_store
            .filter
            .with(|f| crate::pages::accounts::query::effective_filter(f, &win));
        Some((eff, id, include_descendants.get()))
    });

    // Reset: replace what is loaded, asking for at least as many rows as are on screen.
    Effect::new(move |_| {
        let Some((filter, id, rollup)) = request_base.get() else {
            register.try_update(LoadedRegister::clear);
            return;
        };
        let Some((generation, limit)) = register.try_update(LoadedRegister::begin_reset) else {
            return;
        };
        let request = bc_ipc::RegisterRequest::new(filter, id, rollup, None, limit);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::register_page(&request).await {
                Ok(page) => {
                    register.try_update(|r| r.apply_reset(generation, page));
                }
                Err(e) => {
                    leptos::logging::warn!("register page failed: {e:?}");
                    register.try_update(|r| r.fail(generation));
                }
            }
        });
    });

    // Extend: append the next page. A no-op while loading or at the end.
    let load_more = Callback::new(move |()| {
        let Some((filter, id, rollup)) = request_base.get_untracked() else {
            return;
        };
        let Some((generation, cursor)) =
            register.try_update(LoadedRegister::begin_extend).flatten()
        else {
            return;
        };
        let request = bc_ipc::RegisterRequest::new(
            filter,
            id,
            rollup,
            Some(cursor),
            crate::pages::accounts::register_pages::PAGE_SIZE,
        );
        leptos::task::spawn_local(async move {
            match bc_ipc::client::register_page(&request).await {
                Ok(page) => {
                    register.try_update(|r| r.apply_extend(generation, page));
                }
                Err(e) => {
                    leptos::logging::warn!("register page failed: {e:?}");
                    register.try_update(|r| r.fail(generation));
                }
            }
        });
    });

    let on_scroll = move |_: web_sys::Event| {
        if let Some(el) = main_ref.get() {
            dashboard_scrolled.set(el.scroll_top() > 180_i32);
            // Ask for the next page two viewports before the bottom, so it
            // lands before the user reaches the sentinel.
            let remaining = el
                .scroll_height()
                .saturating_sub(el.scroll_top())
                .saturating_sub(el.client_height());
            if remaining < el.client_height().saturating_mul(2_i32) {
                load_more.run(());
            }
        }
    };

    // Balance column mode, remembered per browser.
    let balance_mode = RwSignal::new(
        crate::storage::get(crate::pages::accounts::register_pages::BALANCE_MODE_KEY)
            .map_or(BalanceMode::Real, |s| BalanceMode::parse(&s)),
    );
    Effect::new(move |_| {
        crate::storage::set(
            crate::pages::accounts::register_pages::BALANCE_MODE_KEY,
            balance_mode.get().as_str(),
        );
    });

    // Resolved account statistics for the selected account, recomputed against
    // the effective filter (register-style: filter dates win, else the
    // page-level display window). Shared by the sticky bar and the dashboard
    // so both headlines stay in lockstep.
    //
    // The resource returns the window it answered for. A `LocalResource`
    // keeps serving its previous value while a refetch is in flight, so the
    // window tag is the only way to tell "fresh" from "stale" data;
    // `stats_busy` below compares it with the current window.
    let stats_resource = LocalResource::new(move || async move {
        data_version.get();
        let win = window.get();
        let rollup_on = include_descendants.get();
        let Some(id) = selected_id.get() else {
            return Ok::<_, bc_ipc::BcError>((win, None));
        };
        let (from, until, filter) = filter_store.filter.with_untracked(|f| {
            let eff = crate::pages::accounts::query::effective_filter(f, &win);
            let active =
                crate::pages::accounts::query::filter_has_non_date_dim(f).then(|| eff.clone());
            (
                eff.date_from.unwrap_or(jiff::civil::Date::MIN),
                eff.date_until.unwrap_or(jiff::civil::Date::MAX),
                active,
            )
        });
        // Re-subscribe to the filter signal so edits re-run the resource.
        filter_store.filter.track();
        let commodity = accounts_resource
            .get()
            .and_then(Result::ok)
            .and_then(|nodes| nodes.into_iter().find(|n| n.id == id))
            .and_then(|n| {
                if rollup_on {
                    n.rollup.first().map(|a| a.currency_code.clone())
                } else {
                    n.balance.map(|b| b.currency_code)
                }
            });
        let stats = bc_ipc::client::get_account_stats(
            &id,
            commodity.as_deref(),
            rollup_on,
            from,
            until,
            filter.as_ref(),
        )
        .await?;
        Ok((win, Some(stats)))
    });

    let stats_signal = Signal::derive(move || {
        let (_, stats) = stats_resource.get().and_then(Result::ok)?;
        stats
    });
    let stats_busy = Signal::derive(move || {
        let current = window.get();
        stats_resource
            .with(|r| crate::pages::accounts::query::awaiting_window(r.as_ref(), &current))
    });

    // The store sets `loading` on every reset and drops responses from an
    // older generation, so while it is set the rows on screen belong to a
    // previous request.
    let register_busy = Signal::derive(move || register.with(|r| r.loading));

    // Derive selected node as a Signal so StickyAccountBar can receive it.
    let selected_node = Signal::derive(move || {
        let id = selected_id.get()?;
        let accounts = accounts_resource.get()?.ok()?;
        accounts.into_iter().find(|a| a.id == id)
    });

    let create_tx = Action::new_unsync(|tx: &NewTransaction| {
        let tx = tx.clone();
        async move { bc_ipc::client::create_transaction(&tx).await }
    });

    let toasts = crate::components::toast::use_toasts();
    let pending_new_date = RwSignal::new(None::<jiff::civil::Date>);

    // Controls whether the add-transaction form is shown.
    let show_add_tx = RwSignal::new(false);

    // Centralised open/close so all writers go through one place.
    let open_add_tx = move || show_add_tx.set(true);
    let close_add_tx = move || show_add_tx.set(false);

    // Reactive IPC error signal passed down to AddTransactionForm.
    let submit_error = Signal::derive(move || {
        create_tx.value().with(|v| {
            let r = v.as_ref()?;
            r.as_ref().err().map(ToString::to_string)
        })
    });

    // After any successful mutation, bump data_version — all subscribed
    // resources react automatically.  Future actions (amend, void, import)
    // only need to add one line here.
    Effect::new(move |_| {
        if create_tx.value().with(|v| matches!(v, Some(Ok(_)))) {
            data_version.update(|v| *v = v.wrapping_add(1));
            close_add_tx();
            if let Some(date) = pending_new_date.get_untracked() {
                pending_new_date.set(None);
                period_notify::notify_if_out_of_period(toasts, window, date);
            }
        }
    });

    // MARK: Keyboard shortcuts

    // ↵ (Enter) opens the add-transaction form when an account is selected
    // and the form is not already visible.  Ignored when an interactive
    // element (input, select, textarea, button) has focus.
    window_event_listener_untyped("keydown", move |e| {
        use wasm_bindgen::JsCast as _;
        let Ok(ke) = e.dyn_into::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ke.key() != "Enter" || show_add_tx.get() || selected_id.get().is_none() {
            return;
        }
        if let Some(target) = ke.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            let tag = el.tag_name().to_ascii_lowercase();
            if matches!(tag.as_str(), "input" | "select" | "textarea" | "button") {
                return;
            }
        }
        open_add_tx();
    });

    view! {
        <div class=style::shell>
            // Sidebar
            <div class=move || {
                if sidebar_collapsed.get() {
                    format!("{} {}", style::sidebar, style::sidebar_collapsed)
                } else {
                    style::sidebar.to_owned()
                }
            }>
                // Inner scroll wrapper — keeps overflow-y: auto off the outer
                // sidebar so the absolutely-positioned toggle button can hang
                // outside the right edge without triggering a scrollbar.
                <div class=style::sidebar_content>
                    {move || match accounts_resource.get() {
                        None => {
                            // TODO: replace with AccountSidebarSkeleton (layout shift)
                            view! { <div class=style::empty_state>"Loading accounts…"</div> }
                                .into_any()
                        }
                        Some(Err(e)) => {
                            view! { <div class=style::empty_state>{format!("Error: {e}")}</div> }
                                .into_any()
                        }
                        Some(Ok(accounts)) => {
                            view! {
                                <AccountSidebar
                                    nodes=accounts
                                    selected_id=selected_id
                                    collapsed=sidebar_collapsed.read_only()
                                    expanded=expanded
                                />
                            }
                                .into_any()
                        }
                    }}
                </div>
                <button
                    class=style::sidebar_toggle
                    on:click=toggle_sidebar
                    aria-label="toggle sidebar"
                >
                    {move || if sidebar_collapsed.get() { "›" } else { "‹" }}
                </button>
            </div>

            // Main scrollable column
            <div
                class=style::main
                node_ref=main_ref
                on:scroll=on_scroll
                data-testid="accounts-main-scroll"
            >
                <StickyAccountBar
                    node=selected_node
                    stats=stats_signal
                    visible=dashboard_scrolled.read_only()
                />

                {move || match selected_node.get() {
                    None => {
                        view! {
                            <div class=style::empty_state>
                                "// select an account from the sidebar"
                            </div>
                        }
                            .into_any()
                    }
                    Some(node) => {
                        let node_id = node.id.clone();
                        let node_id_register = node.id.clone();
                        let account_nodes = accounts_resource
                            .get()
                            .and_then(Result::ok)
                            .unwrap_or_default();
                        let account_refs = crate::components::account_picker::account_paths(
                            &account_nodes,
                        );

                        view! {
                            <AccountDashboard
                                node=node.clone()
                                stats=stats_signal
                                data_version=data_version.read_only()
                                on_add_tx=Callback::new(move |()| open_add_tx())
                                window=window.read_only().into()
                                busy=stats_busy
                            />

                            {move || {
                                let all_accounts = accounts_resource.get().and_then(Result::ok)?;
                                if !show_add_tx.get() {
                                    return None;
                                }
                                let currency_code = node
                                    .balance
                                    .as_ref()
                                    .map_or_else(String::new, |b| b.currency_code.clone());
                                let scale = crate::components::num::meta::display_meta_for(
                                        &currency_code,
                                        &currencies.get(),
                                    )
                                    .decimals;
                                Some(
                                    // Gate on accounts being loaded — prevents an empty offset
                                    // dropdown from showing before the resource resolves.
                                    view! {
                                        <AddTransactionForm
                                            accounts=all_accounts
                                            current_account_id=node_id.clone()
                                            currency_code=currency_code
                                            scale=scale
                                            on_submit=Callback::new(move |tx: NewTransaction| {
                                                pending_new_date.set(Some(tx.date));
                                                create_tx.dispatch(tx);
                                            })
                                            on_cancel=Callback::new(move |()| close_add_tx())
                                            submit_error=submit_error
                                        />
                                    },
                                )
                            }}

                            <TransactionRegister
                                register=register.read_only().into()
                                on_load_more=load_more
                                balance_mode=balance_mode
                                viewing_account_id=node_id_register
                                accounts=account_refs
                                window=window
                                busy=register_busy
                                on_change=Callback::new(move |()| {
                                    data_version.update(|v| *v = v.wrapping_add(1));
                                })
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}
