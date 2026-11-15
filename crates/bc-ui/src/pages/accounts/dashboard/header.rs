//! The dashboard header: title, management actions, metadata line and the
//! inline lifecycle panels.

use bc_ipc::AccountDetail;
use bc_ipc::AccountNode;
use bc_ipc::BcError;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use super::meta::meta_text;
use super::style;
use crate::components::account_lifecycle::ArchiveAccount;
use crate::components::account_lifecycle::CloseAccount;
use crate::components::error_banner::ErrorBanner;
use crate::components::tag_token::TagToken;
use crate::pages::settings::accounts::links::focus_account_href;
use crate::pages::settings::accounts::links::new_account_href;

/// A lifecycle panel that expands under the header; one is open at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    /// Close the account on a date.
    Close,
    /// Archive the account.
    Archive,
}

/// A lifecycle action that runs at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Restore {
    /// Clear the close date.
    Reopen,
    /// Clear the archive.
    Unarchive,
}

/// The account's title row, metadata line and management actions.
///
/// The actions link into Settings to add a sub-account or edit this one, and
/// expand the close and archive panels inline. A closed account offers
/// `reopen`, an archived one `unarchive`. Until `detail` loads, the closed
/// state comes from `node`. A successful archive opens the account's row in
/// Settings, since the account leaves the sidebar.
///
/// # Arguments
///
/// * `node` - The account shown, for its name, mask, tags and type.
/// * `detail` - Its details: `None` while loading.
/// * `on_change` - Runs after a close, reopen or unarchive lands.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn AccountHeader(
    /// The account shown.
    node: AccountNode,
    /// The account's details; `None` while loading.
    #[prop(into)]
    detail: Signal<Option<Result<AccountDetail, BcError>>>,
    /// Runs after a close, reopen or unarchive lands.
    on_change: Callback<()>,
) -> impl IntoView {
    let id = StoredValue::new(node.id.clone());
    let node_closed = node.closed_on.is_some();
    let loaded = move || detail.get().and_then(Result::ok);
    let closed = Memo::new(move |_| loaded().map_or(node_closed, |d| d.closed_on.is_some()));
    let archived = Memo::new(move |_| loaded().is_some_and(|d| d.archived));
    let meta = move || match detail.get() {
        None => String::new(),
        Some(Ok(d)) => meta_text(&d),
        Some(Err(_)) => "// details unavailable".to_owned(),
    };
    let tags = node
        .tags
        .iter()
        .map(|t| view! { <TagToken label=t.path.clone() /> })
        .collect_view();

    let open = RwSignal::new(None::<Panel>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<BcError>);
    let changed = Callback::new(move |()| {
        open.try_set(None);
        on_change.run(());
    });
    let navigate = use_navigate();
    let archived_done = Callback::new(move |()| {
        let href = id.with_value(|id| focus_account_href(id));
        navigate(&href, NavigateOptions::default());
    });
    let on_busy = Callback::new(move |b| {
        busy.try_set(b);
    });
    let toggle = move |panel: Panel| {
        error.set(None);
        open.update(|o| *o = (*o != Some(panel)).then_some(panel));
    };
    let restore = move |action: Restore| {
        if busy.get_untracked() {
            return;
        }
        let account = id.get_value();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = match action {
                Restore::Reopen => bc_ipc::client::reopen_account(&account).await,
                Restore::Unarchive => bc_ipc::client::unarchive_account(&account).await,
            };
            busy.try_set(false);
            match result {
                Ok(()) => changed.run(()),
                Err(e) => drop(error.try_set(Some(e))),
            }
        });
    };
    let expanded = move |panel: Panel| (open.get() == Some(panel)).to_string();

    let close_action = move || {
        if closed.get() {
            view! {
                <button
                    class=style::action
                    prop:disabled=move || busy.get()
                    on:click=move |_| restore(Restore::Reopen)
                >
                    "reopen"
                </button>
            }
            .into_any()
        } else {
            view! {
                <button
                    class=style::action
                    prop:disabled=move || busy.get()
                    aria-expanded=move || expanded(Panel::Close)
                    on:click=move |_| toggle(Panel::Close)
                >
                    "close…"
                </button>
            }
            .into_any()
        }
    };
    let archive_action = move || {
        if archived.get() {
            view! {
                <button
                    class=style::action
                    prop:disabled=move || busy.get()
                    on:click=move |_| restore(Restore::Unarchive)
                >
                    "unarchive"
                </button>
            }
            .into_any()
        } else {
            view! {
                <button
                    class=style::action
                    prop:disabled=move || busy.get()
                    aria-expanded=move || expanded(Panel::Archive)
                    on:click=move |_| toggle(Panel::Archive)
                >
                    "archive…"
                </button>
            }
            .into_any()
        }
    };
    let panel = move || {
        open.get().map(|panel| {
            let account = id.get_value();
            match panel {
                Panel::Close => {
                    view! { <CloseAccount id=account on_done=changed on_busy /> }.into_any()
                }
                Panel::Archive => {
                    view! { <ArchiveAccount id=account on_done=archived_done on_busy /> }.into_any()
                }
            }
        })
    };

    view! {
        <div class=style::title_row>
            <div class=style::name_group>
                <span class=style::acct_name>{node.name}</span>
                {node.mask.map(|m| view! { <span class=style::mask>"···· "{m}</span> })}
            </div>
            <div class=style::actions>
                <A
                    href=new_account_href(node.account_type, Some(&node.id))
                    attr:class=style::action
                >
                    "+ sub-account"
                </A>
                <A href=focus_account_href(&node.id) attr:class=style::action>
                    "edit"
                </A>
                {close_action}
                {archive_action}
            </div>
        </div>
        <p class=style::meta_line data-testid="dashboard-meta">
            <span>{meta}</span>
            {tags}
        </p>
        {panel}
        {move || error.get().map(|e| view! { <ErrorBanner message=e.to_string() /> })}
    }
}
