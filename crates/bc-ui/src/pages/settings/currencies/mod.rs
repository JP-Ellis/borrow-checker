// Editable currency-registry panel for Settings.
//
// This file is also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim so `first_conflict` can be host-tested, which is why the
// module doc here uses `//` rather than `//!` (an inner doc comment is only
// valid as the first item when the file is compiled as a standalone module).

#[cfg(target_arch = "wasm32")]
use bc_ipc::CommodityInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "currencies.module.scss");

/// The first ambiguous marker across a set of drafts, if any: `(marker, code_a, code_b)`.
///
/// Codes compare case-insensitively; symbols and aliases exactly — mirroring the
/// backend `check_ambiguity` so the editor blocks exactly what the store would reject.
///
/// # Arguments
///
/// * `drafts` - The `(code, symbol, aliases)` triples of every non-deleted row.
///
/// # Returns
///
/// `Some((marker, code_a, code_b))` for the first collision, else `None`.
#[must_use]
pub fn first_conflict(
    drafts: &[(String, String, Vec<String>)],
) -> Option<(String, String, String)> {
    use std::collections::HashMap;
    let mut owner: HashMap<String, String> = HashMap::new();
    for (code, symbol, aliases) in drafts {
        let mut markers: Vec<String> = vec![code.to_uppercase()];
        if !symbol.is_empty() {
            markers.push(symbol.clone());
        }
        markers.extend(aliases.iter().cloned());
        for m in markers {
            if let Some(prev) = owner.get(&m) {
                if prev != code {
                    return Some((m, prev.clone(), code.clone()));
                }
            } else {
                owner.insert(m, code.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::first_conflict;

    #[test]
    fn detects_symbol_alias_collision() {
        let drafts = vec![
            ("USD".to_owned(), "$".to_owned(), vec![]),
            ("EUR".to_owned(), "€".to_owned(), vec!["$".to_owned()]),
        ];
        let c = first_conflict(&drafts).expect("collision");
        assert_eq!(c.0, "$");
    }

    #[test]
    fn no_collision_is_none() {
        let drafts = vec![
            ("USD".to_owned(), "$".to_owned(), vec!["US$".to_owned()]),
            ("AUD".to_owned(), "A$".to_owned(), vec!["AU$".to_owned()]),
        ];
        assert!(first_conflict(&drafts).is_none());
    }
}

/// One editable row: a `CommodityInfo` plus client-side identity and staging flags.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, PartialEq)]
struct Row {
    /// Stable client-side row id (independent of the persisted commodity id).
    key: u32,
    /// The persisted commodity id, or empty for a not-yet-saved row.
    info: CommodityInfo,
    /// True for a row added in this editing session (code still editable).
    is_new: bool,
    /// True when staged for deletion (committed on save).
    deleted: bool,
}

/// Editable currency registry with a dirty-gated save bar.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn CurrenciesPanel() -> impl IntoView {
    let store = crate::currency_ctx::use_currency_store();
    let rows = RwSignal::new(Vec::<Row>::new());
    let pristine = RwSignal::new(Vec::<Row>::new());
    let next_key = RwSignal::new(0_u32);
    let banner = RwSignal::new(Option::<String>::None);
    let saving = RwSignal::new(false);

    // Seed the working + pristine sets once, on the initial store load.
    // Post-save reconciliation is handled explicitly in `save()` so a failed
    // or in-flight edit is never clobbered by a store refresh.
    Effect::new(move |_| {
        let served = store.get();
        if !pristine.get_untracked().is_empty() {
            return;
        }
        let seeded: Vec<Row> = served
            .into_iter()
            .enumerate()
            .map(|(i, info)| Row {
                key: u32::try_from(i).unwrap_or(u32::MAX),
                info,
                is_new: false,
                deleted: false,
            })
            .collect();
        next_key.set(u32::try_from(seeded.len()).unwrap_or(u32::MAX));
        rows.set(seeded.clone());
        pristine.set(seeded);
    });

    let dirty = move || rows.get() != pristine.get();
    let conflict = move || {
        let drafts: Vec<(String, String, Vec<String>)> = rows
            .get()
            .iter()
            .filter(|r| !r.deleted)
            .map(|r| {
                (
                    r.info.code.clone(),
                    r.info.symbol.clone().unwrap_or_default(),
                    r.info.aliases.clone(),
                )
            })
            .collect();
        first_conflict(&drafts)
    };

    let add = move |_| {
        let key = next_key.get();
        next_key.set(key.saturating_add(1));
        rows.update(|rs| {
            rs.push(Row {
                key,
                info: CommodityInfo::new("", "", None, vec![], 2, true, false),
                is_new: true,
                deleted: false,
            });
        });
    };

    let discard = move |_| {
        rows.set(pristine.get());
        banner.set(None);
    };

    let save = move |_| {
        if conflict().is_some() || saving.get() {
            return;
        }
        saving.set(true);
        let snapshot = rows.get();
        let pristine_snapshot = pristine.get();
        leptos::task::spawn_local(async move {
            let mut err: Option<String> = None;
            // Deletes
            for r in &pristine_snapshot {
                let still = snapshot.iter().find(|s| s.key == r.key);
                let removed = still.is_none_or(|s| s.deleted);
                if removed
                    && !r.info.id.is_empty()
                    && let Err(e) = bc_ipc::client::delete_currency(&r.info.id).await
                {
                    err.get_or_insert(e.to_string());
                }
            }
            // Creates + updates
            for r in snapshot.iter().filter(|r| !r.deleted) {
                let res = if r.is_new {
                    bc_ipc::client::create_currency(&r.info).await.map(|_| ())
                } else {
                    let prev = pristine_snapshot.iter().find(|p| p.key == r.key);
                    if prev.is_some_and(|p| p.info == r.info) {
                        Ok(())
                    } else {
                        bc_ipc::client::update_currency(&r.info).await
                    }
                };
                if let Err(e) = res {
                    err.get_or_insert(e.to_string());
                }
            }
            saving.set(false);
            if let Some(e) = err {
                // Failure: keep the working edits intact so the user can
                // correct/retry; only surface the error in the banner.
                banner.set(Some(e));
            } else {
                banner.set(None);
                // Success: refresh the shared store AND rebuild a clean
                // working+pristine snapshot from server truth, so is_new/deleted
                // flags clear and the save bar retracts. The seed Effect no
                // longer re-fires (pristine is non-empty), so this explicit
                // reseed is what clears the dirty state.
                if let Ok(list) = bc_ipc::client::list_currencies().await {
                    store.set(list.clone());
                    let fresh: Vec<Row> = list
                        .into_iter()
                        .enumerate()
                        .map(|(i, info)| Row {
                            key: u32::try_from(i).unwrap_or(u32::MAX),
                            info,
                            is_new: false,
                            deleted: false,
                        })
                        .collect();
                    next_key.set(u32::try_from(fresh.len()).unwrap_or(u32::MAX));
                    rows.set(fresh.clone());
                    pristine.set(fresh);
                }
            }
        });
    };

    view! {
        <div>
            <div class=move || {
                if dirty() {
                    format!("{} {}", style::savebar, style::savebar_show)
                } else {
                    style::savebar.to_owned()
                }
            }>
                <span class=style::savebar_count>
                    {move || {
                        let n = rows
                            .get()
                            .iter()
                            .filter(|r| {
                                pristine
                                    .get()
                                    .iter()
                                    .find(|p| p.key == r.key)
                                    .is_none_or(|p| p != *r)
                            })
                            .count();
                        if n == 1 {
                            "1 unsaved change".to_owned()
                        } else {
                            format!("{n} unsaved changes")
                        }
                    }}
                </span>
                <span class=style::savebar_err>
                    {move || {
                        conflict()
                            .map(|(m, a, b)| format!("“{m}” maps to both {a} and {b}"))
                            .unwrap_or_default()
                    }}
                </span>
                <span class=style::spacer />
                <button class=style::abtn on:click=discard>
                    "discard"
                </button>
                <button
                    class=format!("{} {}", style::abtn, style::abtn_primary)
                    prop:disabled=move || conflict().is_some() || saving.get()
                    on:click=save
                >
                    "save"
                </button>
            </div>

            <div class=style::panel>
                <h1 class=style::title>"Currencies"</h1>
                <p class=style::sub>
                    "Known currencies for this ledger. Every field is editable — changes are staged until you save."
                </p>

                {move || banner.get().map(|msg| view! { <div class=style::banner>{msg}</div> })}

                <table class=style::table>
                    <thead>
                        <tr>
                            <th>"Code"</th>
                            <th>"Symbol"</th>
                            <th>"Aliases"</th>
                            <th class=style::flag>"Decimals"</th>
                            <th class=style::flag>"ISO"</th>
                            <th class=style::flag>"Sym after"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=move || rows.get() key=|r| r.key let:row>
                            {currency_row(rows, row, banner)}
                        </For>
                    </tbody>
                </table>

                <button class=style::addbtn on:click=add>
                    "+ add currency"
                </button>

                <p class=style::hint>
                    "Codes are immutable once saved. Every marker — code · symbol · alias — must be unique; a collision blocks save. A referenced currency can’t be deleted."
                </p>
            </div>
        </div>
    }
}

/// Renders one editable row, mutating `rows` by the row's stable `key`.
#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "row is a per-item owned value produced by the For loop"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
fn currency_row(
    rows: RwSignal<Vec<Row>>,
    row: Row,
    banner: RwSignal<Option<String>>,
) -> impl IntoView {
    let key = row.key;
    let update_field = move |f: Box<dyn Fn(&mut CommodityInfo)>| {
        rows.update(|rs| {
            if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                f(&mut r.info);
            }
        });
    };

    let code_ro = !row.is_new;
    let code_val = row.info.code.clone();
    let sym_val = row.info.symbol.clone().unwrap_or_default();
    let dec_val = row.info.decimals;
    let iso_val = row.info.is_iso;
    let after_val = row.info.symbol_after;
    let aliases = row.info.aliases.clone();
    let deleted = row.deleted;
    let is_new = row.is_new;
    let id = row.info.id.clone();

    let tr_class = if deleted {
        style::row_del.to_owned()
    } else if is_new {
        style::row_new.to_owned()
    } else {
        String::new()
    };

    let new_alias = RwSignal::new(String::new());

    view! {
        <tr class=tr_class>
            <td>
                <input
                    class=format!("{} {}", style::fld, style::fld_code)
                    prop:value=code_val
                    prop:readonly=code_ro
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        update_field(Box::new(move |i| i.code.clone_from(&v)));
                    }
                />
            </td>
            <td>
                <input
                    class=format!("{} {}", style::fld, style::fld_sym)
                    prop:value=sym_val
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        update_field(
                            Box::new(move |i| {
                                i.symbol = if v.is_empty() { None } else { Some(v.clone()) };
                            }),
                        );
                    }
                />
            </td>
            <td>
                <div class=style::aliases>
                    {aliases
                        .into_iter()
                        .map(|a| {
                            let a2 = a.clone();
                            view! {
                                <span class=style::chip>
                                    {a.clone()}
                                    <button
                                        class=style::chip_x
                                        on:click=move |_| {
                                            let a3 = a2.clone();
                                            update_field(
                                                Box::new(move |i| i.aliases.retain(|x| x != &a3)),
                                            );
                                        }
                                    >
                                        "×"
                                    </button>
                                </span>
                            }
                        })
                        .collect::<Vec<_>>()}
                    <input
                        class=format!("{} {}", style::fld, style::fld_alias)
                        prop:value=move || new_alias.get()
                        placeholder="+ alias"
                        on:input=move |ev| new_alias.set(event_target_value(&ev))
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" {
                                let v = new_alias.get().trim().to_owned();
                                if !v.is_empty() {
                                    update_field(Box::new(move |i| i.aliases.push(v.clone())));
                                    new_alias.set(String::new());
                                }
                                ev.prevent_default();
                            }
                        }
                    />
                </div>
            </td>
            <td class=style::flag>
                <input
                    class=format!("{} {}", style::fld, style::fld_num)
                    r#type="number"
                    prop:value=dec_val
                    on:input=move |ev| {
                        let v = event_target_value(&ev).parse::<u8>().unwrap_or(2);
                        update_field(Box::new(move |i| i.decimals = v));
                    }
                />
            </td>
            <td class=style::flag>
                <input
                    class=style::check
                    r#type="checkbox"
                    prop:checked=iso_val
                    on:change=move |ev| {
                        let c = event_target_checked(&ev);
                        update_field(Box::new(move |i| i.is_iso = c));
                    }
                />
            </td>
            <td class=style::flag>
                <input
                    class=style::check
                    r#type="checkbox"
                    prop:checked=after_val
                    on:change=move |ev| {
                        let c = event_target_checked(&ev);
                        update_field(Box::new(move |i| i.symbol_after = c));
                    }
                />
            </td>
            <td class=style::actions>
                {if deleted {
                    view! {
                        <button
                            class=style::iconbtn
                            on:click=move |_| {
                                rows.update(|rs| {
                                    if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                                        r.deleted = false;
                                    }
                                });
                            }
                        >
                            "↺"
                        </button>
                    }
                        .into_any()
                } else {
                    view! {
                        <button
                            class=style::iconbtn
                            on:click=move |_| delete_row(rows, key, is_new, id.clone(), banner)
                        >
                            "🗑"
                        </button>
                    }
                        .into_any()
                }}
            </td>
        </tr>
    }
}

/// Stages a row for deletion; the actual backend delete is committed on Save.
/// New (unsaved) rows are dropped outright; saved rows are flagged `deleted`
/// (struck-through with an undo) and removed on Save via the delete loop in
/// `save()`. A referenced currency is refused at Save time (the backend
/// `delete_currency` returns an in-use error surfaced in the banner).
#[cfg(target_arch = "wasm32")]
fn delete_row(
    rows: RwSignal<Vec<Row>>,
    key: u32,
    is_new: bool,
    _id: String,
    _banner: RwSignal<Option<String>>,
) {
    rows.update(|rs| {
        if is_new {
            rs.retain(|r| r.key != key);
        } else if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
            r.deleted = true;
        }
    });
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
