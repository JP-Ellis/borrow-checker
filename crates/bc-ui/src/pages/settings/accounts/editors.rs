//! The field editors the Settings account rows share: the edit target, the
//! description, tag, commodity and opening-date cells, and a manual asset's
//! acquisition fields.

use std::str::FromStr as _;

use bc_ipc::AcquisitionDto;
use bc_ipc::DepreciationPolicyDto;
use bc_ipc::TagInfo;
use jiff::civil::Date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::rows;
use super::rows::Drafts;
use super::rows::Row;
use super::rows::RowDraft;
use super::style;
use crate::components::ChipVariant;
use crate::components::chip::Chip;
use crate::components::chip::ChipRow;
use crate::components::tag_picker::TagPicker;

/// Resets an input to `value` after a change the draft ignored, so the
/// field never shows a value the draft does not hold.
pub(super) fn reset_input(ev: &leptos::ev::Event, value: &str) {
    event_target::<leptos::web_sys::HtmlInputElement>(ev).set_value(value);
}

/// The draft an editor writes to: a saved row by account ID, or a new
/// account by its local key.
#[derive(Clone, Copy)]
pub(super) enum Target {
    /// A saved account's row.
    Saved(RwSignal<Vec<Row>>, StoredValue<String>),
    /// An unsaved new account.
    New(RwSignal<Drafts>, u64),
}

impl Target {
    /// Applies `change` to the target's draft, if the row still exists.
    pub(super) fn edit(self, change: impl FnOnce(&mut RowDraft)) {
        match self {
            Self::Saved(rows, id) => id.with_value(|id| {
                rows.update(|rs| {
                    if let Some(r) = rs.iter_mut().find(|r| r.base.id == *id) {
                        change(&mut r.draft);
                    }
                });
            }),
            Self::New(drafts, key) => drafts.update(|d| {
                if let Some(r) = d.get_mut(key) {
                    change(&mut r.draft);
                }
            }),
        }
    }
}

/// A date input's value, or `None` when it is empty or malformed.
pub(super) fn parse_date(text: &str) -> Option<Date> {
    Date::from_str(text).ok()
}

/// The cells a saved row and a new-account row share: description, tags,
/// commodities and opening date.
pub(super) fn common_cells(
    target: Target,
    draft: Signal<RowDraft>,
    all_tags: RwSignal<Vec<TagInfo>>,
) -> impl IntoView {
    view! {
        <td>
            <input
                class=format!("{} {}", style::fld, style::fld_desc)
                aria-label="description"
                prop:value=move || draft.with(|d| d.description.clone())
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    target.edit(|d| d.description = v);
                }
            />
        </td>
        <td>
            <TagPicker
                tags=Signal::derive(move || draft.with(|d| d.tags.clone()))
                all_tags=all_tags.into()
                on_add=Callback::new(move |tag: TagInfo| {
                    target
                        .edit(|d| {
                            if !d.tags.iter().any(|t| t.id == tag.id) {
                                d.tags.push(tag);
                            }
                        });
                })
                on_remove=Callback::new(move |tag: TagInfo| {
                    target.edit(|d| d.tags.retain(|t| t.id != tag.id));
                })
                on_created=Callback::new(move |tag: TagInfo| all_tags.update(|v| v.push(tag)))
                compact=true
            />
        </td>
        <td>{commodity_editor(target, draft)}</td>
        <td>
            <input
                class=format!("{} {}", style::fld, style::fld_date)
                r#type="date"
                aria-label="opened"
                prop:value=move || rows::date_text(draft.with(|d| d.opened_on))
                on:change=move |ev| {
                    let v = parse_date(&event_target_value(&ev));
                    target.edit(|d| d.opened_on = v);
                }
            />
        </td>
    }
}

/// The ordered commodity chips of one row: the first is the default,
/// clicking another makes it the default, and a select adds a known code.
fn commodity_editor(target: Target, draft: Signal<RowDraft>) -> impl IntoView {
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
                    target.edit(|d| d.commodity_codes.retain(|x| *x != c));
                });
                view! {
                    <Chip variant on_remove=remove remove_label=format!("Remove {code}")>
                        <button
                            class=style::chip_btn
                            title=if i == 0 { "default commodity" } else { "make default" }
                            prop:disabled=i == 0
                            on:click=move |_| {
                                let c = first_code.clone();
                                target
                                    .edit(|d| {
                                        d.commodity_codes.retain(|x| *x != c);
                                        d.commodity_codes.insert(0, c);
                                    });
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
                        target.edit(|d| d.commodity_codes.push(code));
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
pub(super) fn acquisition_editor(target: Target, draft: Signal<RowDraft>) -> impl IntoView {
    let acquisition_memo =
        Memo::new(move |_| draft.with(|d| d.acquisition.clone().unwrap_or_default()));
    let acquisition = move || acquisition_memo.get();
    let edit = move |change: Box<dyn FnOnce(&mut AcquisitionDto)>| {
        target.edit(|d| {
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
                    target
                        .edit(|d| match rows::parse_decimal(&text) {
                            Ok(cost) => {
                                d.unparsed_cost = None;
                                d.acquisition.get_or_insert_with(Default::default).cost = cost;
                            }
                            Err(_) => d.unparsed_cost = Some(text),
                        });
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
                        target.edit(|d| d.unparsed_rate = None);
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
                                    target.edit(|d| set_rate(d, &text));
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
