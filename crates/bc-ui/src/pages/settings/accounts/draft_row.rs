//! The new-account draft row of the Settings accounts table.

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountType;
use bc_ipc::TagInfo;
use leptos::prelude::*;

use super::editors::Target;
use super::editors::acquisition_editor;
use super::editors::common_cells;
use super::rows;
use super::rows::Drafts;
use super::rows::RowDraft;
use super::style;

/// The root types a new top-level account may take, in sidebar order.
const ACCOUNT_TYPES: [AccountType; 5] = [
    AccountType::Asset,
    AccountType::Liability,
    AccountType::Equity,
    AccountType::Income,
    AccountType::Expense,
];

/// The kinds a new account may take; the first is the default.
const KINDS: [AccountKindDto; 5] = [
    AccountKindDto::DepositAccount,
    AccountKindDto::ManualAsset,
    AccountKindDto::Receivable,
    AccountKindDto::VirtualAllocation,
    AccountKindDto::Group,
];

/// Renders one new-account draft row.
///
/// Kind is editable until the account is saved. The parent picker offers
/// saved accounts only; with no parent, a type select picks the root type.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub(super) fn draft_row(
    drafts: RwSignal<Drafts>,
    bases: Memo<Vec<AccountDetail>>,
    all_tags: RwSignal<Vec<TagInfo>>,
    key: u64,
) -> impl IntoView {
    let target = Target::New(drafts, key);
    let draft: Signal<RowDraft> =
        Memo::new(move |_| drafts.with(|d| d.get(key).map_or_default(|r| r.draft.clone()))).into();
    let error = Memo::new(move |_| drafts.with(|d| d.get(key)?.error.clone()));
    let missing = Memo::new(move |_| draft.with(|d| rows::create_args(d).err()));
    let parent = Memo::new(move |_| draft.with(|d| d.parent_id.clone()));
    let account_type = Memo::new(move |_| draft.with(|d| d.account_type));
    let kind = Memo::new(move |_| draft.with(|d| d.kind));

    let parent_options = Memo::new(move |_| {
        bases.with(|b| {
            rows::new_parents(b, account_type.get())
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
    let type_select = move || {
        parent.with(Option::is_none).then(|| {
            view! {
                <select
                    class=style::fld
                    aria-label="type"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        let chosen = ACCOUNT_TYPES.into_iter().find(|t| t.label() == v);
                        target.edit(|d| d.account_type = chosen);
                    }
                >
                    <option value="" prop:selected=move || account_type.get().is_none()>
                        "(type)"
                    </option>
                    {ACCOUNT_TYPES
                        .into_iter()
                        .map(|t| {
                            view! {
                                <option
                                    value=t.label()
                                    prop:selected=move || account_type.get() == Some(t)
                                >
                                    {t.label()}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            }
        })
    };

    view! {
        <tr class=style::row_dirty data-testid="account-draft" aria-label="new account">
            <td class=style::cell_path>
                <input
                    class=format!("{} {}", style::fld, style::fld_name)
                    aria-label="name"
                    placeholder="name"
                    prop:value=move || draft.with(|d| d.name.clone())
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        target.edit(|d| d.name = v);
                    }
                />
                {move || {
                    missing.get().map(|m| view! { <span class=style::field_err>{m}</span> })
                }}
                {move || {
                    error
                        .get()
                        .map(|e| {
                            view! {
                                <p class=style::row_err role="alert">
                                    {e.to_string()}
                                </p>
                            }
                        })
                }}
            </td>
            <td>
                <select
                    class=style::fld
                    aria-label="parent"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        let chosen = (!v.is_empty()).then_some(v);
                        drafts
                            .update(|d| {
                                if let Some(r) = d.get_mut(key) {
                                    bases.with_untracked(|b| r.draft.set_parent(b, chosen));
                                }
                            });
                    }
                >
                    {parent_view}
                </select>
                {type_select}
            </td>
            <td>
                <select
                    class=style::fld
                    aria-label="kind"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        if let Some(chosen) = KINDS.into_iter().find(|k| rows::kind_label(*k) == v)
                        {
                            target.edit(|d| d.set_kind(chosen));
                        }
                    }
                >
                    {KINDS
                        .into_iter()
                        .map(|k| {
                            view! {
                                <option
                                    value=rows::kind_label(k)
                                    prop:selected=move || kind.get() == k
                                >
                                    {rows::kind_label(k)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </td>
            {common_cells(target, draft, all_tags)}
            <td />
            <td>
                {move || {
                    (kind.get() == AccountKindDto::ManualAsset)
                        .then(|| acquisition_editor(target, draft))
                }}
            </td>
            <td class=format!("{} {}", style::state, style::state_new)>"new"</td>
        </tr>
    }
}
