//! Inline close and archive panels for one account.
//!
//! Each panel previews what the action would do, warns about what it finds,
//! and confirms through the lifecycle command. Confirm waits while the
//! preview loads, and a non-empty cascade waits until its "these too" box is
//! ticked. A failed preview does not block: the write itself refuses a
//! blocking descendant. The close panel also needs a date.

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
/// balance as of the date, and the postings dated after it. An empty date
/// disables confirm. A failed preview warns and leaves confirm enabled,
/// sending no cascade. A failed close shows inline and leaves the panel open.
///
/// # Arguments
///
/// * `id` - The account to close.
/// * `on_done` - Runs once the account has closed.
/// * `on_busy` - Told when a confirm starts and ends.
#[component]
pub fn CloseAccount(
    /// The account to close.
    #[prop(into)]
    id: String,
    /// Runs once the account has closed.
    on_done: Callback<()>,
    /// Told `true` when a confirm starts and `false` when it ends.
    #[prop(optional)]
    on_busy: Option<Callback<bool>>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let on = RwSignal::new(Some(jiff::Zoned::now().date()));
    let preview = RwSignal::new(None::<(Date, Result<ClosePreviewDto, BcError>)>);
    let ticked = RwSignal::new(false);
    let shown_cascade = StoredValue::new(None::<Vec<AccountRef>>);
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
        let Some(date) = date else {
            return;
        };
        let account = id.get_value();
        spawn_local(async move {
            let result = bc_ipc::client::preview_close_account(&account, date).await;
            if latest.try_get_value() != Some(request) {
                return;
            }
            if let Ok(p) = &result {
                untick_on_change(shown_cascade, ticked, &p.cascade);
            }
            preview.try_set(Some((date, result)));
        });
    });

    let state = move || {
        preview.with(|p| {
            preview_state(
                p.as_ref()
                    .map(|(_, r)| r.as_ref().map(|dto| dto.cascade.len())),
            )
        })
    };
    let enabled = move || !busy.get() && rules::can_close(on.get(), state(), ticked.get());
    let confirm = move |_| {
        let Some(date) = on.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return;
        }
        let account = id.get_value();
        let cascade = rules::sends_cascade(untrack(state), ticked.get_untracked());
        set_busy(busy, on_busy, true);
        error.set(None);
        spawn_local(async move {
            let result = bc_ipc::client::close_account(&account, date, cascade).await;
            set_busy(busy, on_busy, false);
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
                    prop:value=move || on.get().map_or_default(|d| d.to_string())
                    on:change=move |ev| on.set(event_target_value(&ev).parse::<Date>().ok())
                />
            </label>
            {move || match (on.get(), preview.get()) {
                (None, _) => view! { <p class=style::warn>"enter a close date"</p> }.into_any(),
                (Some(_), None) => view! { <p class=style::muted>"// previewing…"</p> }.into_any(),
                (Some(_), Some((_, Err(e)))) => unavailable(&e).into_any(),
                (Some(_), Some((date, Ok(p)))) => close_details(&p, date, ticked).into_any(),
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
/// nudges when the account has no close date. A failed preview warns and
/// leaves confirm enabled, sending no cascade. A failed archive shows inline
/// and leaves the panel open.
///
/// # Arguments
///
/// * `id` - The account to archive.
/// * `on_done` - Runs once the account is archived.
/// * `on_busy` - Told when a confirm starts and ends.
#[component]
pub fn ArchiveAccount(
    /// The account to archive.
    #[prop(into)]
    id: String,
    /// Runs once the account is archived.
    on_done: Callback<()>,
    /// Told `true` when a confirm starts and `false` when it ends.
    #[prop(optional)]
    on_busy: Option<Callback<bool>>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let preview = LocalResource::new(move || {
        let account = id.get_value();
        async move { bc_ipc::client::preview_archive_account(&account).await }
    });
    let ticked = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<BcError>);

    let state = move || {
        preview.with(|p| preview_state(p.as_ref().map(|r| r.as_ref().map(|dto| dto.cascade.len()))))
    };
    let enabled = move || !busy.get() && rules::can_confirm(state(), ticked.get());
    let confirm = move |_| {
        if busy.get_untracked() {
            return;
        }
        let account = id.get_value();
        let cascade = rules::sends_cascade(untrack(state), ticked.get_untracked());
        set_busy(busy, on_busy, true);
        error.set(None);
        spawn_local(async move {
            let result = bc_ipc::client::archive_account(&account, cascade).await;
            set_busy(busy, on_busy, false);
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
                Some(Err(e)) => unavailable(&e).into_any(),
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

/// The gate's view of a preview, from its cascade size.
///
/// # Arguments
///
/// * `cascade` - `None` while the preview loads, else its cascade size or
///   its failure.
fn preview_state(cascade: Option<Result<usize, &BcError>>) -> rules::Preview {
    match cascade {
        None => rules::Preview::Loading,
        Some(Err(_)) => rules::Preview::Failed,
        Some(Ok(cascade)) => rules::Preview::Ready { cascade },
    }
}

/// Unticks the "these too" box when a preview's cascade differs from the
/// last one shown, so a tick never carries over to accounts it did not see.
fn untick_on_change(
    shown: StoredValue<Option<Vec<AccountRef>>>,
    ticked: RwSignal<bool>,
    cascade: &[AccountRef],
) {
    let changed = shown
        .try_with_value(|s| rules::cascade_changed(s.as_deref(), cascade))
        .unwrap_or(false);
    if changed {
        shown.try_set_value(Some(cascade.to_vec()));
        ticked.try_set(false);
    }
}

/// Sets the panel's busy flag and tells the caller.
fn set_busy(busy: RwSignal<bool>, on_busy: Option<Callback<bool>>, value: bool) {
    busy.try_set(value);
    if let Some(callback) = on_busy {
        callback.run(value);
    }
}

/// The warning for a failed preview; confirm stays enabled.
fn unavailable(error: &BcError) -> impl IntoView + use<> {
    view! {
        <p class=style::warn role="status">
            {format!("preview unavailable — {error}")}
        </p>
    }
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
