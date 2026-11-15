//! Inline close and archive panels for one account.
//!
//! Each panel previews what the action would do, warns about what it finds,
//! and confirms through the lifecycle command. A non-empty cascade is the one
//! gate: confirm waits until its "these too" box is ticked.

/// The confirm gate and warning copy.
mod rules;

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::ArchivePreviewDto;
use bc_ipc::BcError;
use bc_ipc::ClosePreviewDto;
use jiff::civil::Date;
use leptos::prelude::*;
use leptos::task::spawn_local;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;
use crate::components::num::Num;

import_style!(style, "account_lifecycle.module.scss");

// MARK: Close

/// Previews and confirms closing an account on a chosen date.
///
/// The date defaults to today, and each change re-runs the preview. The
/// panel lists the open descendants under `close these too`, every non-zero
/// balance as of the date, and the postings dated after it. A failed close
/// shows inline and leaves the panel open.
///
/// # Arguments
///
/// * `id` - The account to close.
/// * `on_done` - Runs once the account has closed.
#[component]
pub fn CloseAccount(
    /// The account to close.
    #[prop(into)]
    id: String,
    /// Runs once the account has closed.
    on_done: Callback<()>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let on = RwSignal::new(jiff::Zoned::now().date());
    let preview = RwSignal::new(None::<(Date, Result<ClosePreviewDto, BcError>)>);
    let ticked = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<BcError>);
    // Each preview request takes a number; only the latest one's response
    // lands, so a slow preview for an earlier date never overwrites a newer.
    let latest = StoredValue::new(0_u64);

    Effect::new(move || {
        let date = on.get();
        let request = latest.get_value().wrapping_add(1);
        latest.set_value(request);
        preview.set(None);
        let account = id.get_value();
        spawn_local(async move {
            let result = bc_ipc::client::preview_close_account(&account, date).await;
            if latest.try_get_value() == Some(request) {
                preview.try_set(Some((date, result)));
            }
        });
    });

    let enabled = move || {
        !busy.get()
            && preview.with(|p| match p {
                Some((_, Ok(p))) => rules::can_confirm(p.cascade.len(), ticked.get()),
                _ => false,
            })
    };
    let confirm = move |_| {
        if busy.get_untracked() {
            return;
        }
        let account = id.get_value();
        let date = on.get_untracked();
        let cascade = ticked.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = bc_ipc::client::close_account(&account, date, cascade).await;
            busy.try_set(false);
            match result {
                Ok(()) => on_done.run(()),
                Err(e) => drop(error.try_set(Some(e))),
            }
        });
    };

    view! {
        <section class=style::panel aria-label="close account">
            <label class=style::field>
                "close on"
                <input
                    class=style::date
                    r#type="date"
                    prop:value=move || on.get().to_string()
                    on:change=move |ev| {
                        let text = event_target_value(&ev);
                        if let Ok(date) = text.parse::<Date>() {
                            on.set(date);
                        }
                    }
                />
            </label>
            {move || match preview.get() {
                None => view! { <p class=style::muted>"// previewing…"</p> }.into_any(),
                Some((_, Err(e))) => {
                    view! { <ErrorBanner message=format!("preview failed: {e}") /> }.into_any()
                }
                Some((date, Ok(p))) => close_details(&p, date, ticked).into_any(),
            }}
            {failure(error)}
            <div class=style::actions>
                <button
                    class=format!("{} {}", style::btn, style::btn_primary)
                    prop:disabled=move || !enabled()
                    on:click=confirm
                >
                    "close"
                </button>
            </div>
        </section>
    }
}

/// The close preview's cascade, balances and late postings.
///
/// # Arguments
///
/// * `preview` - What the close would do.
/// * `on` - The date the preview ran for.
/// * `ticked` - The `close these too` box.
fn close_details(preview: &ClosePreviewDto, on: Date, ticked: RwSignal<bool>) -> impl IntoView {
    let balances = (!preview.balances.is_empty()).then(|| {
        let items = preview
            .balances
            .iter()
            .map(|(account, amounts)| balance_item(account, amounts))
            .collect_view();
        view! {
            <p class=style::warn>{format!("non-zero balances on {on}")}</p>
            <ul class=style::list aria-label="balances">
                {items}
            </ul>
        }
    });
    let late = rules::postings_after(preview.postings_after, on)
        .map(|text| view! { <p class=style::warn>{text}</p> });
    view! {
        {cascade(&preview.cascade, "close these too", ticked)}
        {balances}
        {late}
    }
}

/// One account's non-zero balance, every commodity it holds.
fn balance_item(account: &AccountRef, amounts: &[Amount]) -> impl IntoView {
    let amounts = amounts
        .iter()
        .map(|a| view! { <Num money=a.clone() /> })
        .collect_view();
    view! {
        <li class=style::balance>
            <span class=style::path>{account.name.clone()}</span>
            <span class=style::amounts>{amounts}</span>
        </li>
    }
}

// MARK: Archive

/// Previews and confirms archiving an account.
///
/// The panel lists the active descendants under `archive these too`, and
/// nudges when the account has no close date. A failed archive shows inline
/// and leaves the panel open.
///
/// # Arguments
///
/// * `id` - The account to archive.
/// * `on_done` - Runs once the account is archived.
#[component]
pub fn ArchiveAccount(
    /// The account to archive.
    #[prop(into)]
    id: String,
    /// Runs once the account is archived.
    on_done: Callback<()>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let preview = LocalResource::new(move || {
        let account = id.get_value();
        async move { bc_ipc::client::preview_archive_account(&account).await }
    });
    let ticked = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<BcError>);

    let enabled = move || {
        !busy.get()
            && preview.with(|p| match p {
                Some(Ok(p)) => rules::can_confirm(p.cascade.len(), ticked.get()),
                _ => false,
            })
    };
    let confirm = move |_| {
        if busy.get_untracked() {
            return;
        }
        let account = id.get_value();
        let cascade = ticked.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = bc_ipc::client::archive_account(&account, cascade).await;
            busy.try_set(false);
            match result {
                Ok(()) => on_done.run(()),
                Err(e) => drop(error.try_set(Some(e))),
            }
        });
    };

    view! {
        <section class=style::panel aria-label="archive account">
            {move || match preview.get() {
                None => view! { <p class=style::muted>"// previewing…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("preview failed: {e}") /> }.into_any()
                }
                Some(Ok(p)) => archive_details(&p, ticked).into_any(),
            }}
            {failure(error)}
            <div class=style::actions>
                <button
                    class=format!("{} {}", style::btn, style::btn_primary)
                    prop:disabled=move || !enabled()
                    on:click=confirm
                >
                    "archive"
                </button>
            </div>
        </section>
    }
}

/// The archive preview's cascade and close-date nudge.
///
/// # Arguments
///
/// * `preview` - What the archive would do.
/// * `ticked` - The `archive these too` box.
fn archive_details(preview: &ArchivePreviewDto, ticked: RwSignal<bool>) -> impl IntoView {
    let nudge = preview
        .no_close_date
        .then(|| view! { <p class=style::warn>"this account has no close date"</p> });
    view! {
        {cascade(&preview.cascade, "archive these too", ticked)}
        {nudge}
    }
}

// MARK: Shared

/// The descendants an action carries with it, under the box that allows it.
///
/// Renders nothing for an empty cascade.
///
/// # Arguments
///
/// * `accounts` - The descendants, by full path.
/// * `label` - The checkbox label.
/// * `ticked` - The checkbox state.
fn cascade(
    accounts: &[AccountRef],
    label: &'static str,
    ticked: RwSignal<bool>,
) -> Option<impl IntoView + use<>> {
    (!accounts.is_empty()).then(|| {
        let items = accounts
            .iter()
            .map(|a| view! { <li class=style::path>{a.name.clone()}</li> })
            .collect_view();
        view! {
            <label class=style::check>
                <input
                    r#type="checkbox"
                    prop:checked=move || ticked.get()
                    on:change=move |ev| ticked.set(event_target_checked(&ev))
                />
                {label}
            </label>
            <ul class=style::list aria-label="cascade">
                {items}
            </ul>
        }
    })
}

/// The last confirm's error, if it failed.
fn failure(error: RwSignal<Option<BcError>>) -> impl IntoView {
    move || {
        error
            .get()
            .map(|e| view! { <ErrorBanner message=e.to_string() /> })
    }
}

#[cfg(debug_assertions)]
pub mod qa;
