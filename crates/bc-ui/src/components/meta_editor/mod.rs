//! Repeatable, type-aware key-value metadata editor.
//!
//! One row per entry. Repeated keys are legal — a key is not a slot — and row
//! order is `position`, the display order, which `Alt+↑`/`Alt+↓` rewrites.
//!
//! A row's control comes from its key's registered type, joined from the shared
//! registry snapshot. A key the snapshot does not know leaves the row untyped
//! and read-only; the row still reorders, deletes and round-trips.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

/// Showcase route exercising every value type and every broken row state.
#[cfg(all(target_arch = "wasm32", debug_assertions))]
pub mod qa;

/// Leptos-free row buffer, classification and serialisation.
pub mod model;

#[cfg(target_arch = "wasm32")]
use core::sync::atomic::AtomicU64;
#[cfg(target_arch = "wasm32")]
use core::sync::atomic::Ordering;

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
#[cfg(target_arch = "wasm32")]
use bc_ipc::MetaKeyDefDto;
#[cfg(target_arch = "wasm32")]
use bc_ipc::MetaTypeDto;
#[cfg(target_arch = "wasm32")]
use bc_ipc::MetaValueDto;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::AccountPicker;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::MetaDraft;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::MetaRow;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::RowKind;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::canonical_text;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::classify;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::insert_row_below;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::move_row;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::parses_as;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::push_blank_row;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::registered_type;
#[cfg(target_arch = "wasm32")]
use crate::components::meta_editor::model::remove_row;

#[cfg(target_arch = "wasm32")]
import_style!(style, "meta_editor.module.scss");

/// Every value type, in the order the create row offers them.
#[cfg(target_arch = "wasm32")]
const ALL_TYPES: [MetaTypeDto; 7] = [
    MetaTypeDto::Text,
    MetaTypeDto::Number,
    MetaTypeDto::Boolean,
    MetaTypeDto::Date,
    MetaTypeDto::Timestamp,
    MetaTypeDto::Amount,
    MetaTypeDto::Account,
];

/// Hands each editor instance a distinct id, so two editors on one page do not
/// collide over the element ids their focus moves target.
#[cfg(target_arch = "wasm32")]
static INSTANCE: AtomicU64 = AtomicU64::new(0);

/// Moves focus to the element with `id`, on the next frame so the row it belongs
/// to has been rendered.
///
/// Rows are created, deleted and reordered, so addressing them by element id
/// outlives any `NodeRef` map.
#[cfg(target_arch = "wasm32")]
fn focus_soon(id: String) {
    request_animation_frame(move || {
        let Some(element) = document().get_element_by_id(&id) else {
            return;
        };
        if let Ok(html) = web_sys::wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(element) {
            let _focused = html.focus();
        }
    });
}

/// The element id of a row's key input.
#[cfg(target_arch = "wasm32")]
fn key_id(instance: u64, uid: u64) -> String {
    format!("meta-{instance}-key-{uid}")
}

/// The element id of a row's primary value control.
#[cfg(target_arch = "wasm32")]
fn val_id(instance: u64, uid: u64) -> String {
    format!("meta-{instance}-val-{uid}")
}

/// A repeatable, type-aware metadata editor over one owner's entries.
///
/// # Arguments
///
/// * `rows` - The owner's row buffer.
/// * `on_change` - Called with the whole buffer after every mutation; the owner
///   stores it back. Whole-list replacement matches the whole-owner replace the
///   backend's events already assume.
/// * `accounts` - All selectable accounts, for `account`-typed rows.
/// * `default_commodity` - Commodity a fresh `amount` row is seeded with.
/// * `add_label` - Label on the append button; defaults to `+ field`.
/// * `testid_prefix` - When set, rows carry `<prefix>-key-<index>` and
///   `<prefix>-value-<index>` test ids and the append button carries
///   `<prefix>-add`. When empty, rows carry the unindexed `meta-key` /
///   `meta-value` and the button carries `meta-add`.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn MetaEditor(
    /// The owner's row buffer.
    rows: Signal<Vec<MetaRow>>,
    /// Called with the whole buffer after every mutation.
    on_change: Callback<Vec<MetaRow>>,
    /// All selectable accounts, for `account`-typed rows.
    accounts: Vec<AccountRef>,
    /// Commodity a fresh `amount` row is seeded with.
    default_commodity: Signal<String>,
    /// Label on the append button; defaults to `+ field`.
    #[prop(into, optional)]
    add_label: String,
    /// Test-id prefix; empty means the unindexed `meta-*` names.
    #[prop(into, optional)]
    testid_prefix: String,
) -> impl IntoView {
    let instance = INSTANCE.fetch_add(1, Ordering::Relaxed);
    let accounts = StoredValue::new(accounts);
    let add_testid = if testid_prefix.is_empty() {
        "meta-add".to_owned()
    } else {
        format!("{testid_prefix}-add")
    };
    let testid_prefix = StoredValue::new(testid_prefix);
    let label = if add_label.is_empty() {
        "+ field".to_owned()
    } else {
        add_label
    };

    let append = move |_| {
        let mut next = rows.get_untracked();
        let uid = push_blank_row(&mut next, &default_commodity.get_untracked());
        on_change.run(next);
        focus_soon(key_id(instance, uid));
    };

    view! {
        <div class=style::editor>
            <For
                each=move || { rows.get().iter().map(|row| row.uid).collect::<Vec<_>>() }
                key=|uid| *uid
                children=move |uid| {
                    view! {
                        <MetaEditorRow
                            uid=uid
                            instance=instance
                            rows=rows
                            on_change=on_change
                            accounts=accounts
                            default_commodity=default_commodity
                            testid_prefix=testid_prefix
                        />
                    }
                }
            />
            <button class=style::add type="button" on:click=append data-testid=add_testid>
                {label}
            </button>
        </div>
    }
}

/// One row of the metadata editor.
///
/// # Arguments
///
/// * `uid` - Identity of the row this component renders.
/// * `instance` - Identity of the owning editor, for element ids.
/// * `rows` - The owner's row buffer.
/// * `on_change` - Called with the whole buffer after every mutation.
/// * `accounts` - All selectable accounts.
/// * `default_commodity` - Commodity a fresh `amount` row is seeded with.
/// * `testid_prefix` - Test-id prefix; empty means the unindexed `meta-*` names.
#[cfg(target_arch = "wasm32")]
#[component]
fn MetaEditorRow(
    /// Identity of the row this component renders.
    uid: u64,
    /// Identity of the owning editor, for element ids.
    instance: u64,
    /// The owner's row buffer.
    rows: Signal<Vec<MetaRow>>,
    /// Called with the whole buffer after every mutation.
    on_change: Callback<Vec<MetaRow>>,
    /// All selectable accounts.
    accounts: StoredValue<Vec<AccountRef>>,
    /// Commodity a fresh `amount` row is seeded with.
    default_commodity: Signal<String>,
    /// Test-id prefix; empty means the unindexed `meta-*` names.
    testid_prefix: StoredValue<String>,
) -> impl IntoView {
    let meta_keys = crate::meta_keys_ctx::use_meta_key_store();
    let currencies = crate::currency_ctx::use_currency_store();

    let open = RwSignal::new(false);
    let highlighted = RwSignal::new(0_usize);

    /* A prefixed test id carries the row's position, which is what an e2e spec
    addressing one row of a repeatable editor needs; the unprefixed form does
    not, because the detail editor's specs take the first match. */
    let testid = move |part: &str| {
        testid_prefix.with_value(|prefix| {
            if prefix.is_empty() {
                format!("meta-{part}")
            } else {
                let index = rows
                    .get()
                    .iter()
                    .position(|row| row.uid == uid)
                    .unwrap_or_default();
                format!("{prefix}-{part}-{index}")
            }
        })
    };
    let key_testid = move || testid("key");
    let value_testid = move || testid("value");

    let row_now = move || rows.get().into_iter().find(|row| row.uid == uid);
    let row_untracked = move || rows.get_untracked().into_iter().find(|row| row.uid == uid);

    let kind = move || row_now().map_or(RowKind::Untyped, |row| classify(&row, &meta_keys.get()));

    /* Every mutation reads the buffer, edits it, and hands the whole thing
    back — the same whole-owner replacement the backend's events assume. */
    let with_draft = move |mutator: Box<dyn FnOnce(&mut MetaDraft)>| {
        let keys = meta_keys.get_untracked();
        let commodity = default_commodity.get_untracked();
        let mut next = rows.get_untracked();
        let Some(row) = next.iter_mut().find(|row| row.uid == uid) else {
            return;
        };
        let seed = MetaDraft::seed(row.source.as_ref(), &keys, &commodity);
        let draft = row.draft.get_or_insert(seed);
        mutator(draft);
        // The registry is the authority on a registered key's type, so a draft
        // never edits under a type the store disagrees with.
        if let Some(ty) = registered_type(&keys, draft.key.trim()) {
            draft.ty = ty;
        }
        on_change.run(next);
    };

    let draft_text = move || {
        row_now()
            .and_then(|row| row.draft.map(|draft| draft.text))
            .unwrap_or_else(|| {
                row_now()
                    .and_then(|row| row.source)
                    .map(|entry| canonical_text(&entry.value))
                    .unwrap_or_default()
            })
    };
    let draft_boolean = move || {
        row_now().is_some_and(|row| {
            row.draft.map_or_else(
                || {
                    row.source
                        .is_some_and(|entry| matches!(entry.value, MetaValueDto::Boolean(true)))
                },
                |draft| draft.boolean,
            )
        })
    };
    let draft_commodity = move || {
        row_now()
            .and_then(|row| row.draft.map(|draft| draft.commodity))
            .unwrap_or_else(|| default_commodity.get())
    };
    let account_id_now = move || {
        row_now()
            .map(|row| {
                row.draft.map_or_else(
                    || {
                        row.source
                            .as_ref()
                            .and_then(|entry| match entry.value {
                                MetaValueDto::Account(ref id) => Some(id.clone()),
                                MetaValueDto::Text(_)
                                | MetaValueDto::Number(_)
                                | MetaValueDto::Boolean(_)
                                | MetaValueDto::Date(_)
                                | MetaValueDto::Timestamp(_)
                                | MetaValueDto::Amount(_) => None,
                            })
                            .unwrap_or_default()
                    },
                    |draft| draft.account_id,
                )
            })
            .unwrap_or_default()
    };

    /* Does this draft's text currently satisfy its key's registered type? Drives
    the live badge on a row being repaired. */
    let live_parses = move |ty: MetaTypeDto| {
        row_now()
            .and_then(|row| row.draft)
            .is_some_and(|draft| parses_as(ty, &draft))
    };

    let remove = move |_| {
        let mut next = rows.get_untracked();
        if remove_row(&mut next, uid) {
            on_change.run(next);
        }
    };

    /* MARK: Account picker — local signals mirroring the row, re-seeded from it
    whenever it changes underneath (a discard, or an Alt+arrow reorder). */
    let sel_id = RwSignal::new(account_id_now());
    let sel_name = RwSignal::new(String::new());
    let name_of = move |id: &str| {
        accounts.with_value(|list| {
            list.iter()
                .find(|account| account.id == id)
                .map(|account| account.name.clone())
        })
    };
    {
        let seed_name = name_of(&sel_id.get_untracked()).unwrap_or_default();
        sel_name.set(seed_name);
    }
    Effect::new(move |_| {
        let id = account_id_now();
        if sel_id.get_untracked() != id {
            sel_id.set(id.clone());
            sel_name.set(name_of(&id).unwrap_or_default());
        }
    });

    /* MARK: Key combobox. */
    let suggestions = move || {
        let query = row_now()
            .map(|row| row.key().to_owned())
            .unwrap_or_default();
        meta_keys
            .get()
            .into_iter()
            .filter(|def| def.key.contains(query.trim()) && def.key != query.trim())
            .collect::<Vec<_>>()
    };
    let create_query = move || {
        let query = row_now()
            .map(|row| row.key().trim().to_owned())
            .unwrap_or_default();
        (!query.is_empty() && registered_type(&meta_keys.get(), &query).is_none()).then_some(query)
    };
    let create_ty = RwSignal::new(MetaTypeDto::Text);

    let commit_key = move |key: String, ty: MetaTypeDto| {
        with_draft(Box::new(move |draft| {
            draft.key = key;
            draft.ty = ty;
        }));
        open.set(false);
        focus_soon(val_id(instance, uid));
    };
    let commit_create = move || {
        let Some(query) = create_query() else {
            return;
        };
        let ty = create_ty.get_untracked();
        // Appended locally, following the tag picker's create-new row. The
        // backend registers the key on the save that first writes a value under
        // it; a row pruned before then registers nothing.
        meta_keys.update(|keys| keys.push(MetaKeyDefDto::new(query.clone(), ty)));
        commit_key(query, ty);
    };

    let on_key_keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Escape" => {
                // Closes the menu and nothing else — the detail panel's discard
                // handler must not see this.
                open.set(false);
                ev.stop_propagation();
                ev.prevent_default();
            }
            "ArrowDown" if !ev.alt_key() => {
                let count = suggestions().len();
                highlighted.update(|h| *h = h.saturating_add(1).min(count.saturating_sub(1)));
                ev.prevent_default();
            }
            "ArrowUp" if !ev.alt_key() => {
                highlighted.update(|h| *h = h.saturating_sub(1));
                ev.prevent_default();
            }
            "Enter" => {
                let matches = suggestions();
                match matches.get(highlighted.get_untracked()) {
                    Some(def) => commit_key(def.key.clone(), def.ty),
                    None => commit_create(),
                }
                ev.prevent_default();
            }
            "Backspace" => {
                let empty = row_untracked().is_some_and(|row| row.is_empty());
                if empty {
                    let mut next = rows.get_untracked();
                    let previous = next
                        .iter()
                        .position(|row| row.uid == uid)
                        .and_then(|index| index.checked_sub(1))
                        .and_then(|index| next.get(index))
                        .map(|row| row.uid);
                    if remove_row(&mut next, uid) {
                        on_change.run(next);
                        if let Some(previous_uid) = previous {
                            focus_soon(val_id(instance, previous_uid));
                        }
                    }
                    ev.prevent_default();
                }
            }
            _other => {}
        }
        if ev.alt_key() && (ev.key() == "ArrowUp" || ev.key() == "ArrowDown") {
            let mut next = rows.get_untracked();
            if move_row(&mut next, uid, ev.key() == "ArrowUp") {
                on_change.run(next);
                focus_soon(key_id(instance, uid));
            }
            ev.prevent_default();
        }
    };

    /* Enter in a value appends a blank row below and focuses its key. */
    let on_value_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.alt_key() && (ev.key() == "ArrowUp" || ev.key() == "ArrowDown") {
            let mut next = rows.get_untracked();
            if move_row(&mut next, uid, ev.key() == "ArrowUp") {
                on_change.run(next);
                focus_soon(val_id(instance, uid));
            }
            ev.prevent_default();
        } else if ev.key() == "Enter" {
            let mut next = rows.get_untracked();
            let new_uid = insert_row_below(&mut next, uid, &default_commodity.get_untracked());
            on_change.run(next);
            focus_soon(key_id(instance, new_uid));
            ev.prevent_default();
        }
    };

    let text_control = move |extra: &'static str| {
        view! {
            <input
                id=val_id(instance, uid)
                data-testid=value_testid
                class=format!("{} {extra}", style::val_input)
                prop:value=draft_text
                on:input=move |ev| {
                    let text = event_target_value(&ev);
                    with_draft(Box::new(move |draft| draft.text = text));
                }
                on:keydown=on_value_keydown
            />
        }
    };

    let value_control = move || {
        match kind() {
        RowKind::Untyped => {
            view! {
                <span class=style::raw>{draft_text}</span>
                <span
                    class=style::badge_ok
                    title="this key is not in the registry snapshot; the entry is preserved untouched"
                >
                    "raw"
                </span>
            }
                .into_any()
        }
        RowKind::Mismatched(ty) => {
            view! {
                {text_control("")}
                {move || {
                    if live_parses(ty) {
                        view! {
                            <span
                                class=style::badge_ok
                                title="saving sends this back as text; the backend parses it into the key's type and clears the flag"
                            >
                                "\u{21ba} repairs on save"
                            </span>
                        }
                            .into_any()
                    } else {
                        view! {
                            <span
                                class=style::badge_bad
                                title=format!(
                                    "the stored value is not a {}. Fix the value here, or retype the key with `borrow-checker meta retype {} text`",
                                    ty.label(),
                                    row_now().map(|row| row.key().to_owned()).unwrap_or_default(),
                                )
                            >
                                {format!("\u{26a0} not a {}", ty.label())}
                            </span>
                        }
                            .into_any()
                    }
                }}
            }
                .into_any()
        }
        RowKind::Tombstone => {
            view! {
                <span class=style::frozen>{format!("\u{2298} {} (deleted)", draft_text())}</span>
                <AccountPicker
                    accounts=accounts.get_value()
                    selected_id=sel_id
                    selected_name=sel_name
                    on_pick=Callback::new(move |account: AccountRef| {
                        with_draft(Box::new(move |draft| draft.account_id = account.id));
                    })
                />
            }
                .into_any()
        }
        RowKind::Typed(MetaTypeDto::Text) => text_control("").into_any(),
        RowKind::Typed(MetaTypeDto::Number) => text_control(style::val_num).into_any(),
        RowKind::Typed(MetaTypeDto::Timestamp) => {
            view! {
                {text_control("")}
                {move || {
                    (!live_parses(MetaTypeDto::Timestamp) && !draft_text().trim().is_empty())
                        .then(|| {
                            view! {
                                <span
                                    class=style::badge_bad
                                    title="RFC 3339, e.g. 2026-05-02T09:30:00+10:00"
                                >
                                    "\u{26a0} not a timestamp"
                                </span>
                            }
                        })
                }}
            }
                .into_any()
        }
        RowKind::Typed(MetaTypeDto::Boolean) => {
            view! {
                <input
                    id=val_id(instance, uid)
                    data-testid=value_testid
                    type="checkbox"
                    prop:checked=draft_boolean
                    on:change=move |ev| {
                        let checked = event_target_checked(&ev);
                        with_draft(Box::new(move |draft| draft.boolean = checked));
                    }
                    on:keydown=on_value_keydown
                />
            }
                .into_any()
        }
        RowKind::Typed(MetaTypeDto::Date) => {
            view! {
                <input
                    id=val_id(instance, uid)
                    data-testid=value_testid
                    class=style::val_input
                    type="date"
                    prop:value=draft_text
                    on:input=move |ev| {
                        let text = event_target_value(&ev);
                        with_draft(Box::new(move |draft| draft.text = text));
                    }
                    on:keydown=on_value_keydown
                />
            }
                .into_any()
        }
        RowKind::Typed(MetaTypeDto::Amount) => {
            view! {
                {text_control(style::val_num)}
                <select
                    class=style::commodity
                    on:change=move |ev| {
                        let code = event_target_value(&ev);
                        with_draft(Box::new(move |draft| draft.commodity = code));
                    }
                >
                    {move || {
                        let chosen = draft_commodity();
                        currencies
                            .get()
                            .into_iter()
                            .map(|commodity| {
                                let selected = commodity.code == chosen;
                                let code = commodity.code.clone();
                                view! {
                                    <option value=code selected=selected>
                                        {commodity.code}
                                    </option>
                                }
                            })
                            .collect::<Vec<_>>()
                    }}
                </select>
            }
                .into_any()
        }
        RowKind::Typed(MetaTypeDto::Account) => {
            view! {
                {move || {
                    let id = account_id_now();
                    (!id.is_empty() && name_of(&id).is_none())
                        .then(|| {
                            view! {
                                <span
                                    class=style::badge_bad
                                    title="this entry points at an account that is not in the account tree"
                                >
                                    {format!("\u{26a0} unknown account {id}")}
                                </span>
                            }
                        })
                }}
                <AccountPicker
                    accounts=accounts.get_value()
                    selected_id=sel_id
                    selected_name=sel_name
                    on_pick=Callback::new(move |account: AccountRef| {
                        with_draft(Box::new(move |draft| draft.account_id = account.id));
                    })
                />
            }
                .into_any()
        }
    }
    };

    let type_label = move || match kind() {
        RowKind::Untyped => String::new(),
        RowKind::Typed(ty) | RowKind::Mismatched(ty) => ty.label().to_owned(),
        RowKind::Tombstone => MetaTypeDto::Account.label().to_owned(),
    };

    let needs_key = move || {
        row_now()
            .and_then(|row| row.draft.map(|draft| draft.key.trim().is_empty()))
            .unwrap_or(false)
    };

    view! {
        <div class=style::row>
            <div class=style::key_cell>
                <input
                    id=key_id(instance, uid)
                    data-testid=key_testid
                    class=style::key_input
                    prop:value=move || {
                        row_now().map(|row| row.key().to_owned()).unwrap_or_default()
                    }
                    on:input=move |ev| {
                        let key = event_target_value(&ev);
                        let keys = meta_keys.get_untracked();
                        with_draft(
                            Box::new(move |draft| {
                                if let Some(ty) = registered_type(&keys, key.trim()) {
                                    draft.ty = ty;
                                }
                                draft.key = key;
                            }),
                        );
                        highlighted.set(0);
                        open.set(true);
                    }
                    on:focus=move |_| open.set(true)
                    on:blur=move |_| open.set(false)
                    on:keydown=on_key_keydown
                    placeholder="key"
                />
                {move || {
                    open.get()
                        .then(|| {
                            let list = suggestions();
                            let create = create_query();
                            if list.is_empty() && create.is_none() {
                                return None;
                            }
                            Some(
                                view! {
                                    <ul class=style::menu>
                                        {list
                                            .into_iter()
                                            .enumerate()
                                            .map(|(index, def)| {
                                                let picked = def.clone();
                                                view! {
                                                    <li
                                                        class=move || {
                                                            if highlighted.get() == index {
                                                                format!("{} {}", style::option, style::option_hi)
                                                            } else {
                                                                style::option.to_owned()
                                                            }
                                                        }
                                                        on:mouseenter=move |_| highlighted.set(index)
                                                        on:mousedown=move |ev| {
                                                            ev.prevent_default();
                                                            commit_key(picked.key.clone(), picked.ty);
                                                        }
                                                    >
                                                        <span>{def.key.clone()}</span>
                                                        <span class=style::ty>{def.ty.label()}</span>
                                                    </li>
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                        {create
                                            .map(|query| {
                                                view! {
                                                    <li class=style::create_option>
                                                        <span on:mousedown=move |ev| {
                                                            ev.prevent_default();
                                                            commit_create();
                                                        }>{format!("+ create key \"{query}\" as")}</span>
                                                        <select
                                                            class=style::create_ty
                                                            on:mousedown=|ev| ev.stop_propagation()
                                                            on:change=move |ev| {
                                                                let label = event_target_value(&ev);
                                                                if let Some(ty) = ALL_TYPES
                                                                    .into_iter()
                                                                    .find(|ty| ty.label() == label)
                                                                {
                                                                    create_ty.set(ty);
                                                                }
                                                            }
                                                        >
                                                            {ALL_TYPES
                                                                .into_iter()
                                                                .map(|ty| {
                                                                    view! {
                                                                        <option
                                                                            value=ty.label()
                                                                            selected=move || create_ty.get() == ty
                                                                        >
                                                                            {ty.label()}
                                                                        </option>
                                                                    }
                                                                })
                                                                .collect::<Vec<_>>()}
                                                        </select>
                                                    </li>
                                                }
                                            })}
                                    </ul>
                                },
                            )
                        })
                        .flatten()
                }}
            </div>
            <span class=style::ty>{type_label}</span>
            <div class=style::val_cell>
                {value_control}
                {move || {
                    needs_key().then(|| view! { <span class=style::hint>"needs a key"</span> })
                }}
            </div>
            <button
                class=style::remove
                type="button"
                on:click=remove
                aria-label="remove metadata entry"
            >
                "\u{00d7}"
            </button>
        </div>
    }
}
