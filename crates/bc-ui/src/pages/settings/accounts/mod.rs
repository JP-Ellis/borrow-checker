//! Account editor for Settings — every account in tree order, each row
//! editable, new accounts added as draft rows, all saved through a
//! dirty-gated save bar.

mod draft_row;
mod editors;
#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
/// Ordering, display text and the edit draft for the account rows.
pub(crate) mod rows;

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountType;
use bc_ipc::BcError;
use bc_ipc::TagInfo;
use draft_row::draft_row;
use editors::Target;
use editors::acquisition_editor;
use editors::common_cells;
use editors::parse_date;
use editors::reset_input;
use leptos::prelude::*;
use rows::Drafts;
use rows::Row;
use rows::RowDraft;
use stylance::import_style;

use crate::components::account_lifecycle::ArchiveAccount;
use crate::components::account_lifecycle::CloseAccount;
use crate::components::error_banner::ErrorBanner;

import_style!(style, "accounts.module.scss");

/// The Settings accounts section: loads every account and the tag list, then
/// hands them to [`AccountsTable`].
#[component]
pub fn AccountsPanel() -> impl IntoView {
    let resource =
        LocalResource::new(move || async move { bc_ipc::client::list_account_details().await });
    let all_tags = RwSignal::new(Vec::<TagInfo>::new());
    let _tags = LocalResource::new(move || async move {
        if let Ok(list) = bc_ipc::client::list_tags().await {
            all_tags.set(list);
        }
    });

    view! {
        <div class=style::panel data-testid="accounts-panel">
            <h1 class=style::title>"Accounts"</h1>
            <p class=style::sub>
                "Every account in tree order, archived ones dimmed. Edits are staged until you save."
            </p>
            {move || match resource.get() {
                None => view! { <p class=style::empty>"// loading accounts…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("Failed to load accounts: {e}") /> }
                        .into_any()
                }
                Some(Ok(details)) => view! { <AccountsTable details all_tags /> }.into_any(),
            }}
        </div>
    }
}

/// The editable account table over an already-loaded list.
///
/// Separate from [`AccountsPanel`] so the QA harness renders it without IPC.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn AccountsTable(
    /// The accounts to list, in any order; the table orders them.
    details: Vec<AccountDetail>,
    /// Every known tag, for the tag picker; a tag created there joins it.
    all_tags: RwSignal<Vec<TagInfo>>,
) -> impl IntoView {
    let rows = RwSignal::new(details.into_iter().map(Row::new).collect::<Vec<_>>());
    let drafts = RwSignal::new(Drafts::default());
    let saving = RwSignal::new(false);
    // Recomputed on every keystroke, but it notifies only when a base changes,
    // so the parent pickers and the row order re-render only after a save.
    let bases =
        Memo::new(move |_| rows.with(|rs| rs.iter().map(|r| r.base.clone()).collect::<Vec<_>>()));
    let draft_keys =
        Memo::new(move |_| drafts.with(|d| d.rows().iter().map(|r| r.key).collect::<Vec<_>>()));
    let dirty_count = Memo::new(move |_| {
        rows.with(|rs| rs.iter().filter(|r| r.is_dirty()).count())
            .saturating_add(draft_keys.with(Vec::len))
    });
    let is_empty = Memo::new(move |_| bases.with(Vec::is_empty) && draft_keys.with(Vec::is_empty));
    let open = RwSignal::new(None::<OpenPanel>);
    let lifecycle_done = Callback::new(move |()| {
        open.set(None);
        leptos::task::spawn_local(refresh(rows));
    });

    let discard = move |_| {
        rows.update(|rs| {
            for r in rs {
                r.draft = RowDraft::from(&r.base);
                r.error = None;
            }
        });
        drafts.update(Drafts::clear);
    };

    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        // A row with unparsed text, or a draft missing a required field,
        // stays dirty and is not sent.
        let creates: Vec<_> = drafts.with_untracked(|d| {
            d.rows()
                .iter()
                .filter(|r| !r.draft.has_unparsed())
                .filter_map(|r| {
                    rows::create_args(&r.draft)
                        .ok()
                        .map(|args| (r.key, r.draft.clone(), args))
                })
                .collect()
        });
        let pending: Vec<_> = rows.with_untracked(|rs| {
            rs.iter()
                .filter(|r| !r.draft.has_unparsed())
                .filter_map(|r| {
                    rows::diff(&r.base, &r.draft).map(|patch| {
                        (
                            r.base.id.clone(),
                            r.base.event_count,
                            r.draft.clone(),
                            patch,
                        )
                    })
                })
                .collect()
        });
        if creates.is_empty() && pending.is_empty() {
            return;
        }
        saving.set(true);
        leptos::task::spawn_local(async move {
            let mut any_saved = false;
            for (key, sent, args) in creates {
                let result = bc_ipc::client::create_account(&args).await;
                any_saved |= result.is_ok();
                match result {
                    Ok(fresh) => {
                        let mut current = None;
                        drafts.update(|d| current = d.remove(key).map(|r| r.draft));
                        // Edits made while the request was in flight stay as
                        // a draft against the created account.
                        let draft = current
                            .filter(|c| *c != sent)
                            .unwrap_or_else(|| RowDraft::from(&fresh));
                        rows.update(|rs| {
                            rs.push(Row {
                                base: fresh,
                                draft,
                                error: None,
                            });
                        });
                    }
                    Err(e) => drafts.update(|d| {
                        if let Some(r) = d.get_mut(key) {
                            r.error = Some(e);
                        }
                    }),
                }
            }
            for (id, expected_events, sent, patch) in pending {
                let result = bc_ipc::client::update_account(&id, expected_events, &patch).await;
                any_saved |= result.is_ok();
                rows.update(|rs| {
                    let Some(r) = rs.iter_mut().find(|r| r.base.id == id) else {
                        return;
                    };
                    match result {
                        Ok(fresh) => {
                            // Edits made while the request was in flight stay
                            // as a draft against the fresh base.
                            if r.draft == sent {
                                r.draft = RowDraft::from(&fresh);
                            }
                            r.base = fresh;
                            r.error = None;
                        }
                        Err(e) => r.error = Some(e),
                    }
                });
            }
            if any_saved {
                refresh(rows).await;
            }
            saving.set(false);
        });
    };

    let ordered = move || {
        bases.with(|b| {
            rows::tree_order(b)
                .into_iter()
                .map(|(depth, d)| RowKey {
                    depth,
                    id: d.id.clone(),
                    archived: d.archived,
                    closed: d.closed_on.is_some(),
                })
                .collect::<Vec<_>>()
        })
    };

    view! {
        <div
            class=move || {
                if dirty_count.get() > 0 {
                    format!("{} {}", style::savebar, style::savebar_show)
                } else {
                    style::savebar.to_owned()
                }
            }
            data-testid="accounts-savebar"
        >
            <span class=style::spacer />
            <button class=style::abtn on:click=discard prop:disabled=move || saving.get()>
                "discard"
            </button>
            <button
                class=format!("{} {}", style::abtn, style::abtn_primary)
                prop:disabled=move || saving.get()
                on:click=save
            >
                {move || match dirty_count.get() {
                    1 => "save 1 change".to_owned(),
                    n => format!("save {n} changes"),
                }}
            </button>
        </div>
        <Show
            when=move || !is_empty.get()
            fallback=|| {
                view! {
                    <p class=style::empty data-testid="accounts-empty">
                        "// no accounts yet"
                    </p>
                }
            }
        >
            <div class=style::table_scroll>
                <table class=style::table data-testid="accounts-table">
                    <thead>
                        <tr>
                            <th>"Account"</th>
                            <th>"Parent"</th>
                            <th>"Kind"</th>
                            <th>"Description"</th>
                            <th>"Tags"</th>
                            <th>"Commodities"</th>
                            <th>"Opened"</th>
                            <th>"Closed"</th>
                            <th>"Acquisition"</th>
                            <th>"State"</th>
                            <th aria-label="actions" />
                        </tr>
                    </thead>
                    <tbody>
                        <For each=ordered key=Clone::clone let:row_key>
                            {account_row(rows, bases, all_tags, row_key, open, lifecycle_done)}
                        </For>
                        <For each=move || draft_keys.get() key=|k| *k let:draft_key>
                            {draft_row(drafts, bases, all_tags, draft_key)}
                        </For>
                    </tbody>
                </table>
            </div>
        </Show>
        <button
            class=format!("{} {}", style::abtn, style::add)
            prop:disabled=move || saving.get()
            on:click=move |_| add_draft(drafts, bases, None, None)
        >
            "+ add account"
        </button>
    }
}

/// Re-reads every account and merges the result into `rows`.
///
/// A failed read leaves the rows in place; only paths and states derived
/// from other accounts stay stale.
async fn refresh(rows: RwSignal<Vec<Row>>) {
    if let Ok(fresh) = bc_ipc::client::list_account_details().await {
        rows.update(|rs| rows::merge_fresh(rs, fresh));
    }
}

/// Appends a new-account draft row to the table.
///
/// # Arguments
///
/// * `drafts` - The table's drafts.
/// * `bases` - Every saved account, for the parent's type.
/// * `account_type` - The type to start with; a known parent overrides it.
/// * `parent_id` - The parent to start under; an unknown ID is dropped.
pub(crate) fn add_draft(
    drafts: RwSignal<Drafts>,
    bases: Memo<Vec<AccountDetail>>,
    account_type: Option<AccountType>,
    parent_id: Option<String>,
) {
    bases.with_untracked(|b| {
        drafts.update(|d| {
            d.add(b, account_type, parent_id);
        });
    });
}

// MARK: Row

/// What renders a row once: a change to any field re-renders it, since the
/// keyed `<For>` keeps a row whose key is unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RowKey {
    /// Tree depth, for the indent.
    depth: usize,
    /// Account ID.
    id: String,
    /// Whether the account is archived.
    archived: bool,
    /// Whether the account is closed.
    closed: bool,
}

/// The lifecycle panel open under one account's row; one is open at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenPanel {
    /// The account whose row it sits under.
    id: String,
    /// Which panel it is.
    kind: Panel,
}

/// A lifecycle action that previews before it confirms.
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

/// Clears the error on the row `id`, so the post-action refresh replaces it.
fn clear_error(rows: RwSignal<Vec<Row>>, id: &str) {
    let has_error =
        rows.with_untracked(|rs| rs.iter().any(|r| r.base.id == id && r.error.is_some()));
    if has_error {
        rows.update(|rs| {
            if let Some(r) = rs.iter_mut().find(|r| r.base.id == id) {
                r.error = None;
            }
        });
    }
}

/// A value read from the row `id`, or its default when the row is gone.
///
/// Memoised, so an edit to one row does not notify the others.
fn row_value<T, F>(rows: RwSignal<Vec<Row>>, id: StoredValue<String>, read: F) -> Signal<T>
where
    T: Default + Clone + PartialEq + Send + Sync + 'static,
    F: Fn(&Row) -> T + Send + Sync + 'static,
{
    Memo::new(move |_| {
        id.with_value(|id| {
            rows.with(|rs| rs.iter().find(|r| r.base.id == *id).map_or_default(&read))
        })
    })
    .into()
}

/// Renders one editable account row.
///
/// Every editable value derives from `rows` by ID, because the keyed `<For>`
/// re-renders a row only when its [`RowKey`] changes. The lifecycle actions
/// stay disabled while the row is dirty, and the open panel hides with them.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
fn account_row(
    rows: RwSignal<Vec<Row>>,
    bases: Memo<Vec<AccountDetail>>,
    all_tags: RwSignal<Vec<TagInfo>>,
    row_key: RowKey,
    open: RwSignal<Option<OpenPanel>>,
    on_done: Callback<()>,
) -> impl IntoView {
    let RowKey {
        depth,
        id: account_id,
        archived,
        closed,
    } = row_key;
    let key = StoredValue::new(account_id);
    let target = Target::Saved(rows, key);
    let base = row_value(rows, key, |r| Some(r.base.clone()));
    let draft = row_value(rows, key, |r| r.draft.clone());
    let error = row_value(rows, key, |r| r.error.clone());
    let dirty = row_value(rows, key, Row::is_dirty);
    // No command changes an account's kind.
    let kind = base
        .get_untracked()
        .map_or(AccountKindDto::Group, |b| b.kind);

    let row_class = move || {
        let mut class = String::new();
        if archived {
            class.push_str(style::row_archived);
        }
        if dirty.get() {
            class.push(' ');
            class.push_str(style::row_dirty);
        }
        class
    };
    let indent = format!("padding-left: calc({depth} * var(--bc-space-5))");
    let state = base
        .get_untracked()
        .as_ref()
        .map_or("active", rows::state_label);

    // The option list follows the bases alone; each option tracks the
    // draft's parent for its own selected state, so an edit re-renders no
    // options.
    let parent = Memo::new(move |_| draft.with(|d| d.parent_id.clone()));
    let parent_options = Memo::new(move |_| {
        bases.with(|b| {
            key.with_value(|id| rows::parent_choices(b, id, !closed, !archived))
                .into_iter()
                .map(|p| (p.id.clone(), p.path.clone()))
                .collect::<Vec<_>>()
        })
    });
    let parent_view = move || {
        let rest = parent_options
            .get()
            .into_iter()
            .map(|(pid, path)| {
                view! {
                    <option
                        value=pid.clone()
                        prop:selected=move || parent.with(|p| p.as_deref() == Some(pid.as_str()))
                    >
                        {path}
                    </option>
                }
            })
            .collect_view();
        view! {
            <option value="" prop:selected=move || parent.with(Option::is_none)>
                "(top level)"
            </option>
            {rest}
        }
    };

    let busy = RwSignal::new(false);
    let locked = move || dirty.get() || busy.get();
    let shown = Memo::new(move |_| {
        if dirty.get() {
            return None;
        }
        open.with(|o| {
            o.as_ref()
                .filter(|p| key.with_value(|id| p.id == *id))
                .map(|p| p.kind)
        })
    });
    let toggle = move |panel: Panel| {
        let id = key.get_value();
        clear_error(rows, &id);
        open.update(|o| {
            let is_open = o.as_ref().is_some_and(|p| p.id == id && p.kind == panel);
            *o = (!is_open).then_some(OpenPanel { id, kind: panel });
        });
    };
    let restore = move |action: Restore| {
        if busy.get_untracked() {
            return;
        }
        let id = key.get_value();
        clear_error(rows, &id);
        busy.set(true);
        leptos::task::spawn_local(async move {
            let result = match action {
                Restore::Reopen => bc_ipc::client::reopen_account(&id).await,
                Restore::Unarchive => bc_ipc::client::unarchive_account(&id).await,
            };
            busy.try_set(false);
            match result {
                Ok(()) => on_done.run(()),
                Err(e) => rows.update(|rs| {
                    if let Some(r) = rs.iter_mut().find(|r| r.base.id == id) {
                        r.error = Some(e);
                    }
                }),
            }
        });
    };
    let close_action = if closed {
        view! {
            <button
                class=style::abtn
                prop:disabled=locked
                on:click=move |_| restore(Restore::Reopen)
            >
                "reopen"
            </button>
        }
        .into_any()
    } else {
        view! {
            <button
                class=style::abtn
                prop:disabled=locked
                aria-expanded=move || (shown.get() == Some(Panel::Close)).to_string()
                on:click=move |_| toggle(Panel::Close)
            >
                "close…"
            </button>
        }
        .into_any()
    };
    let archive_action = if archived {
        view! {
            <button
                class=style::abtn
                prop:disabled=locked
                on:click=move |_| restore(Restore::Unarchive)
            >
                "unarchive"
            </button>
        }
        .into_any()
    } else {
        view! {
            <button
                class=style::abtn
                prop:disabled=locked
                aria-expanded=move || (shown.get() == Some(Panel::Archive)).to_string()
                on:click=move |_| toggle(Panel::Archive)
            >
                "archive…"
            </button>
        }
        .into_any()
    };
    let panel = move || {
        shown.get().map(|panel| {
            let id = key.get_value();
            let body = match panel {
                Panel::Close => view! { <CloseAccount id on_done /> }.into_any(),
                Panel::Archive => view! { <ArchiveAccount id on_done /> }.into_any(),
            };
            view! {
                <tr class=style::panel_row>
                    <td colspan="11">{body}</td>
                </tr>
            }
        })
    };

    let reload = move |_| {
        let id = key.get_value();
        leptos::task::spawn_local(async move {
            let result = bc_ipc::client::get_account_detail(&id).await;
            rows.update(|rs| {
                if let Some(r) = rs.iter_mut().find(|r| r.base.id == id) {
                    match result {
                        Ok(fresh) => *r = Row::new(fresh),
                        Err(e) => r.error = Some(e),
                    }
                }
            });
        });
    };

    view! {
        <tr
            class=row_class
            data-testid="account-row"
            data-account-id=key.get_value()
            aria-label=move || base.get().map_or_default(|b| b.name)
        >
            <td class=style::cell_path>
                <div style=indent>
                    <input
                        class=format!("{} {}", style::fld, style::fld_name)
                        aria-label="name"
                        prop:value=move || draft.with(|d| d.name.clone())
                        on:input=move |ev| {
                            let v = event_target_value(&ev);
                            target.edit(|d| d.name = v);
                        }
                    />
                    {move || {
                        error
                            .get()
                            .map(|e| {
                                let is_conflict = matches!(e, BcError::Conflict(_));
                                view! {
                                    <p class=style::row_err role="alert">
                                        {e.to_string()}
                                        {is_conflict
                                            .then(|| {
                                                view! {
                                                    <button class=style::abtn on:click=reload>
                                                        "reload"
                                                    </button>
                                                }
                                            })}
                                    </p>
                                }
                            })
                    }}
                </div>
            </td>
            <td>
                <select
                    class=style::fld
                    aria-label="parent"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        target.edit(|d| d.parent_id = (!v.is_empty()).then_some(v));
                    }
                >
                    {parent_view}
                </select>
            </td>
            <td>{rows::kind_label(kind)}</td>
            {common_cells(target, draft, all_tags)}
            <td>
                {closed
                    .then(|| {
                        view! {
                            <input
                                class=format!("{} {}", style::fld, style::fld_date)
                                r#type="date"
                                aria-label="closed"
                                prop:value=move || rows::date_text(draft.with(|d| d.closed_on))
                                on:change=move |ev| {
                                    if let Some(v) = parse_date(&event_target_value(&ev)) {
                                        target.edit(|d| d.closed_on = Some(v));
                                    } else {
                                        reset_input(
                                            &ev,
                                            &rows::date_text(draft.with_untracked(|d| d.closed_on)),
                                        );
                                    }
                                }
                            />
                        }
                    })}
            </td>
            <td>
                {(kind == AccountKindDto::ManualAsset).then(|| acquisition_editor(target, draft))}
            </td>
            <td class=format!("{} {}", style::state, state_class(state))>{state}</td>
            <td class=style::actions>{close_action}{archive_action}</td>
        </tr>
        {panel}
    }
}

/// The colour class for a state label from [`rows::state_label`].
fn state_class(state: &str) -> &'static str {
    match state {
        "archived" => style::state_archived,
        "closed" => style::state_closed,
        _ => style::state_active,
    }
}
