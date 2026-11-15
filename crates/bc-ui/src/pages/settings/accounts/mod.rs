//! Account editor for Settings — every account in tree order, each row
//! editable, saved through a dirty-gated save bar.

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
/// Ordering, display text and the edit draft for the account rows.
pub(crate) mod rows;

use std::str::FromStr as _;

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AcquisitionDto;
use bc_ipc::BcError;
use bc_ipc::DepreciationPolicyDto;
use bc_ipc::TagInfo;
use jiff::civil::Date;
use leptos::prelude::*;
use rows::Row;
use rows::RowDraft;
use rust_decimal::Decimal;
use stylance::import_style;

use crate::components::ChipVariant;
use crate::components::chip::Chip;
use crate::components::chip::ChipRow;
use crate::components::error_banner::ErrorBanner;
use crate::components::tag_picker::TagPicker;

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
    if details.is_empty() {
        return view! {
            <p class=style::empty data-testid="accounts-empty">
                "// no accounts yet"
            </p>
        }
        .into_any();
    }

    let rows = RwSignal::new(details.into_iter().map(Row::new).collect::<Vec<_>>());
    let saving = RwSignal::new(false);
    // Recomputed on every keystroke, but it notifies only when a base changes,
    // so the parent pickers and the row order re-render only after a save.
    let bases =
        Memo::new(move |_| rows.with(|rs| rs.iter().map(|r| r.base.clone()).collect::<Vec<_>>()));
    let dirty_count =
        Memo::new(move |_| rows.with(|rs| rs.iter().filter(|r| r.is_dirty()).count()));

    let discard = move |_| {
        rows.update(|rs| {
            for r in rs {
                r.draft = RowDraft::from(&r.base);
                r.error = None;
            }
        });
    };

    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        // A row with unparsed text stays dirty and is not sent.
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
        if pending.is_empty() {
            return;
        }
        saving.set(true);
        leptos::task::spawn_local(async move {
            let mut any_saved = false;
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
                // A failed refresh leaves the saved bases in place; only the
                // derived paths of other rows stay stale.
                if let Ok(fresh) = bc_ipc::client::list_account_details().await {
                    rows.update(|rs| rows::merge_fresh(rs, fresh));
                }
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
                    </tr>
                </thead>
                <tbody>
                    <For each=ordered key=Clone::clone let:row_key>
                        {account_row(rows, bases, all_tags, row_key)}
                    </For>
                </tbody>
            </table>
        </div>
    }
    .into_any()
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

/// Resets an input to `value` after a change the draft ignored, so the
/// field never shows a value the draft does not hold.
fn reset_input(ev: &leptos::ev::Event, value: &str) {
    event_target::<leptos::web_sys::HtmlInputElement>(ev).set_value(value);
}

/// Applies `change` to the draft of row `id`.
fn edit_draft(
    rows: RwSignal<Vec<Row>>,
    id: StoredValue<String>,
    change: impl FnOnce(&mut RowDraft),
) {
    id.with_value(|id| {
        rows.update(|rs| {
            if let Some(r) = rs.iter_mut().find(|r| r.base.id == *id) {
                change(&mut r.draft);
            }
        });
    });
}

/// A date input's value, or `None` when it is empty or malformed.
fn parse_date(text: &str) -> Option<Date> {
    Date::from_str(text).ok()
}

/// Renders one editable account row.
///
/// Every editable value derives from `rows` by ID, because the keyed `<For>`
/// re-renders a row only when its [`RowKey`] changes.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
fn account_row(
    rows: RwSignal<Vec<Row>>,
    bases: Memo<Vec<AccountDetail>>,
    all_tags: RwSignal<Vec<TagInfo>>,
    row_key: RowKey,
) -> impl IntoView {
    let RowKey {
        depth,
        id: account_id,
        archived,
        closed,
    } = row_key;
    let key = StoredValue::new(account_id);
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
                            edit_draft(rows, key, |d| d.name = v);
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
                        edit_draft(rows, key, |d| d.parent_id = (!v.is_empty()).then_some(v));
                    }
                >
                    {parent_view}
                </select>
            </td>
            <td>{rows::kind_label(kind)}</td>
            <td>
                <input
                    class=format!("{} {}", style::fld, style::fld_desc)
                    aria-label="description"
                    prop:value=move || draft.with(|d| d.description.clone())
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        edit_draft(rows, key, |d| d.description = v);
                    }
                />
            </td>
            <td>
                <TagPicker
                    tags=Signal::derive(move || draft.with(|d| d.tags.clone()))
                    all_tags=all_tags.into()
                    on_add=Callback::new(move |tag: TagInfo| {
                        edit_draft(
                            rows,
                            key,
                            |d| {
                                if !d.tags.iter().any(|t| t.id == tag.id) {
                                    d.tags.push(tag);
                                }
                            },
                        );
                    })
                    on_remove=Callback::new(move |tag: TagInfo| {
                        edit_draft(rows, key, |d| d.tags.retain(|t| t.id != tag.id));
                    })
                    on_created=Callback::new(move |tag: TagInfo| all_tags.update(|v| v.push(tag)))
                    compact=true
                />
            </td>
            <td>{commodity_editor(rows, draft, key)}</td>
            <td>
                <input
                    class=format!("{} {}", style::fld, style::fld_date)
                    r#type="date"
                    aria-label="opened"
                    prop:value=move || rows::date_text(draft.with(|d| d.opened_on))
                    on:change=move |ev| {
                        let v = parse_date(&event_target_value(&ev));
                        edit_draft(rows, key, |d| d.opened_on = v);
                    }
                />
            </td>
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
                                        edit_draft(rows, key, |d| d.closed_on = Some(v));
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
                {(kind == AccountKindDto::ManualAsset)
                    .then(|| acquisition_editor(rows, draft, key))}
            </td>
            <td class=format!("{} {}", style::state, state_class(state))>{state}</td>
        </tr>
    }
}

/// The ordered commodity chips of one row: the first is the default,
/// clicking another makes it the default, and a select adds a known code.
fn commodity_editor(
    rows: RwSignal<Vec<Row>>,
    draft: Signal<RowDraft>,
    key: StoredValue<String>,
) -> impl IntoView {
    let store = crate::currency_ctx::use_currency_store();
    let codes = Memo::new(move |_| draft.with(|d| d.commodity_codes.clone()));
    let chips = move || {
        codes
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, code)| {
                let (first_code, remove_code) = (code.clone(), code.clone());
                let variant = if i == 0 {
                    ChipVariant::Accent
                } else {
                    ChipVariant::Filled
                };
                let remove = Callback::new(move |()| {
                    let c = remove_code.clone();
                    edit_draft(rows, key, |d| d.commodity_codes.retain(|x| *x != c));
                });
                view! {
                    <Chip variant on_remove=remove remove_label=format!("Remove {code}")>
                        <button
                            class=style::chip_btn
                            title=if i == 0 { "default commodity" } else { "make default" }
                            prop:disabled=i == 0
                            on:click=move |_| {
                                let c = first_code.clone();
                                edit_draft(
                                    rows,
                                    key,
                                    |d| {
                                        d.commodity_codes.retain(|x| *x != c);
                                        d.commodity_codes.insert(0, c);
                                    },
                                );
                            }
                        >
                            {code.clone()}
                        </button>
                    </Chip>
                }
            })
            .collect_view()
    };
    let addable = move || {
        let held = codes.get();
        store
            .get()
            .into_iter()
            .filter(|c| !held.contains(&c.code))
            .map(|c| view! { <option value=c.code.clone()>{c.code.clone()}</option> })
            .collect_view()
    };

    view! {
        <ChipRow>
            {chips}
            <select
                class=style::fld
                aria-label="add commodity"
                on:change=move |ev| {
                    let code = event_target_value(&ev);
                    if !code.is_empty() {
                        edit_draft(rows, key, |d| d.commodity_codes.push(code));
                    }
                }
            >
                // Re-rendered with the draft, which resets the select to "+".
                {move || {
                    view! {
                        <option value="" prop:selected=true>
                            "+"
                        </option>
                        {addable()}
                    }
                }}
            </select>
        </ChipRow>
    }
}

/// The policy select's value for a depreciation policy.
fn policy_key(policy: Option<&DepreciationPolicyDto>) -> &'static str {
    match policy {
        None => "",
        Some(DepreciationPolicyDto::StraightLine { .. }) => "straight_line",
        Some(DepreciationPolicyDto::DecliningBalance { .. }) => "declining_balance",
        Some(_) => "none",
    }
}

/// The annual rate of a depreciating policy, if it has one.
fn policy_rate(policy: Option<&DepreciationPolicyDto>) -> Option<Decimal> {
    match policy {
        Some(
            DepreciationPolicyDto::StraightLine { annual_rate }
            | DepreciationPolicyDto::DecliningBalance { annual_rate },
        ) => Some(*annual_rate),
        _ => None,
    }
}

/// A manual asset's acquisition fields: date, cost, policy and rate.
///
/// Each edit changes one field of the draft's whole record, so a save sends
/// the untouched fields at their current values.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
fn acquisition_editor(
    rows: RwSignal<Vec<Row>>,
    draft: Signal<RowDraft>,
    key: StoredValue<String>,
) -> impl IntoView {
    let acquisition_memo =
        Memo::new(move |_| draft.with(|d| d.acquisition.clone().unwrap_or_default()));
    let acquisition = move || acquisition_memo.get();
    let edit = move |change: Box<dyn FnOnce(&mut AcquisitionDto)>| {
        edit_draft(rows, key, |d| {
            change(d.acquisition.get_or_insert_with(Default::default));
        });
    };

    view! {
        <div class=style::acq>
            <input
                class=format!("{} {}", style::fld, style::fld_date)
                r#type="date"
                aria-label="acquired"
                prop:value=move || rows::date_text(acquisition().date)
                on:change=move |ev| {
                    let v = parse_date(&event_target_value(&ev));
                    edit(Box::new(move |a| a.date = v));
                }
            />
            <input
                class=format!("{} {}", style::fld, style::fld_num)
                inputmode="decimal"
                aria-label="cost"
                placeholder="cost"
                aria-invalid=move || draft.with(|d| d.unparsed_cost.is_some()).to_string()
                prop:value=move || {
                    draft
                        .with(|d| d.unparsed_cost.clone())
                        .unwrap_or_else(|| acquisition().cost.map_or_default(|c| c.to_string()))
                }
                on:change=move |ev| {
                    let text = event_target_value(&ev);
                    edit_draft(
                        rows,
                        key,
                        |d| match rows::parse_decimal(&text) {
                            Ok(cost) => {
                                d.unparsed_cost = None;
                                d.acquisition.get_or_insert_with(Default::default).cost = cost;
                            }
                            Err(_) => d.unparsed_cost = Some(text),
                        },
                    );
                }
            />
            {move || field_error(draft.with(|d| d.unparsed_cost.clone()))}
            <select
                class=style::fld
                aria-label="depreciation"
                on:change=move |ev| {
                    let choice = event_target_value(&ev);
                    edit(
                        Box::new(move |a| {
                            let rate = policy_rate(a.depreciation_policy.as_ref())
                                .unwrap_or_default();
                            a.depreciation_policy = match choice.as_str() {
                                "none" => Some(DepreciationPolicyDto::None),
                                "straight_line" => {
                                    Some(DepreciationPolicyDto::StraightLine {
                                        annual_rate: rate,
                                    })
                                }
                                "declining_balance" => {
                                    Some(DepreciationPolicyDto::DecliningBalance {
                                        annual_rate: rate,
                                    })
                                }
                                _ => None,
                            };
                        }),
                    );
                    if policy_rate(
                            draft
                                .with_untracked(|d| d.acquisition.clone())
                                .and_then(|a| a.depreciation_policy)
                                .as_ref(),
                        )
                        .is_none()
                    {
                        edit_draft(rows, key, |d| d.unparsed_rate = None);
                    }
                }
            >
                {move || {
                    let current = policy_key(acquisition().depreciation_policy.as_ref());
                    [
                        ("", "(no policy)"),
                        ("none", "none"),
                        ("straight_line", "straight line"),
                        ("declining_balance", "declining balance"),
                    ]
                        .into_iter()
                        .map(|(value, label)| {
                            view! {
                                <option value=value prop:selected=value == current>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
            {move || {
                policy_rate(acquisition().depreciation_policy.as_ref())
                    .map(|rate| {
                        view! {
                            <input
                                class=format!("{} {}", style::fld, style::fld_num)
                                inputmode="decimal"
                                aria-label="annual rate"
                                title="annual rate as a fraction, e.g. 0.25"
                                aria-invalid=move || {
                                    draft.with(|d| d.unparsed_rate.is_some()).to_string()
                                }
                                prop:value=move || {
                                    draft
                                        .with(|d| d.unparsed_rate.clone())
                                        .unwrap_or_else(|| rate.to_string())
                                }
                                on:change=move |ev| {
                                    let text = event_target_value(&ev);
                                    edit_draft(rows, key, |d| set_rate(d, &text));
                                    if text.trim().is_empty() {
                                        reset_input(&ev, &rate.to_string());
                                    }
                                }
                            />
                            {move || field_error(draft.with(|d| d.unparsed_rate.clone()))}
                        }
                    })
            }}
        </div>
    }
}

/// Applies rate text to a draft's depreciating policy.
///
/// Text that does not parse is kept as typed; blank text keeps the current
/// rate.
fn set_rate(draft: &mut RowDraft, text: &str) {
    match rows::parse_decimal(text) {
        Ok(None) => draft.unparsed_rate = None,
        Ok(Some(annual_rate)) => {
            draft.unparsed_rate = None;
            let acquisition = draft.acquisition.get_or_insert_with(Default::default);
            acquisition.depreciation_policy = match acquisition.depreciation_policy {
                Some(DepreciationPolicyDto::DecliningBalance { .. }) => {
                    Some(DepreciationPolicyDto::DecliningBalance { annual_rate })
                }
                _ => Some(DepreciationPolicyDto::StraightLine { annual_rate }),
            };
        }
        Err(_) => draft.unparsed_rate = Some(text.to_owned()),
    }
}

/// The inline message for a field holding `unparsed` text, if any.
fn field_error(unparsed: Option<String>) -> Option<impl IntoView> {
    unparsed.map(|text| {
        let message = rows::parse_decimal(&text).err().unwrap_or_default();
        view! {
            <span class=style::field_err role="alert">
                {message}
            </span>
        }
    })
}

/// The colour class for a state label from [`rows::state_label`].
fn state_class(state: &str) -> &'static str {
    match state {
        "archived" => style::state_archived,
        "closed" => style::state_closed,
        _ => style::state_active,
    }
}
