//! Command palette (⌘K): a text editor for the query language.
//!
//! The input highlights each token by kind and underlines errors and warnings.
//! A hint line explains the problem or the term under the cursor, and a
//! dropdown offers fields, keys, operators and values. [`model`] decides all
//! of this; the component only renders it and routes keys. A commit lands as
//! the [`crate::filter_ctx::Edit`] the palette opened on says: joined onto the
//! stored query, in place of one chip, or in place of the whole query.

pub mod model;

#[cfg(target_arch = "wasm32")]
use bc_query::Expr;
#[cfg(target_arch = "wasm32")]
use bc_query::Severity;
#[cfg(target_arch = "wasm32")]
use bc_query::catalog::Snapshot;
#[cfg(target_arch = "wasm32")]
use bc_query::highlight::TokenKind;
#[cfg(target_arch = "wasm32")]
use bc_query::suggest::SuggestionKind;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::AccountPathLabel;
#[cfg(target_arch = "wasm32")]
use crate::filter_ctx::Edit;
#[cfg(target_arch = "wasm32")]
use crate::filter_ctx::EditTarget;

#[cfg(target_arch = "wasm32")]
import_style!(style, "palette.module.scss");

/// Today's date in the user's time zone, for period suggestions.
#[cfg(target_arch = "wasm32")]
fn today() -> jiff::civil::Date {
    jiff::Zoned::now().date()
}

/// The class for a run of the given token kind.
#[cfg(target_arch = "wasm32")]
fn token_class(kind: Option<TokenKind>) -> &'static str {
    match kind {
        Some(TokenKind::Field) => style::tok_field,
        Some(TokenKind::Key) => style::tok_key,
        Some(TokenKind::Operator) => style::tok_operator,
        Some(TokenKind::Value) => style::tok_value,
        Some(TokenKind::Text) => style::tok_text,
        Some(TokenKind::Keyword) => style::tok_keyword,
        Some(TokenKind::Paren) => style::tok_paren,
        None => "",
    }
}

/// The underline class for a severity; hints are not underlined. The chip row
/// underlines its labels with it too.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn mark_class(mark: Option<Severity>) -> &'static str {
    match mark {
        Some(Severity::Error) => style::mark_error,
        Some(Severity::Warning) => style::mark_warning,
        Some(Severity::Hint) | None => "",
    }
}

/// The class for a hint line.
#[cfg(target_arch = "wasm32")]
fn hint_class(severity: Option<Severity>) -> &'static str {
    match severity {
        Some(Severity::Error) => style::hint_error,
        Some(Severity::Warning) => style::hint_warning,
        Some(Severity::Hint) | None => style::hint_line,
    }
}

/// Command palette modal, opened by ⌘K, the top bar, or a chip.
///
/// # Arguments
///
/// * `open` - Whether the palette is visible.
/// * `on_close` - Called when the palette should close.
/// * `edit` - What the palette opens on.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(clippy::too_many_lines, reason = "Leptos view! block")]
pub fn CommandPalette(
    /// Whether the palette is visible.
    open: ReadSignal<bool>,
    /// Called when the palette should close (Escape, backdrop click, an edit's commit).
    on_close: Callback<()>,
    /// What the palette opens on.
    edit: ReadSignal<Edit>,
) -> impl IntoView {
    let store = crate::filter_ctx::use_filter_store();
    let catalog = crate::query_catalog_ctx::use_query_catalog();

    let text = RwSignal::new(String::new());
    let cursor = RwSignal::new(0_usize);
    let selected = RwSignal::new(0_usize);
    let target = StoredValue::new(EditTarget::Append);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let backdrop_ref = NodeRef::<leptos::html::Div>::new();

    let analysis = Memo::new(move |_| {
        let current = text.get();
        let at = cursor.get();
        catalog.snapshot.with(|snapshot| match snapshot {
            Some(loaded) => model::analyse(&current, at, loaded, today()),
            None => model::analyse(&current, at, &Snapshot::default(), today()),
        })
    });

    /* Keeps the backdrop's horizontal scroll under the input's. */
    let sync_scroll = move || {
        if let (Some(input), Some(backdrop)) =
            (input_ref.get_untracked(), backdrop_ref.get_untracked())
        {
            backdrop.set_scroll_left(input.scroll_left());
        }
    };

    /* Reads the caret from the DOM, in bytes; a caret that moves into other
     * text to complete highlights the first suggestion again. */
    let read_cursor = move || {
        if let Some(input) = input_ref.get_untracked() {
            let value = input.value();
            let units = input.selection_start().ok().flatten().unwrap_or(0);
            let at = model::utf16_to_byte(&value, usize::try_from(units).unwrap_or(usize::MAX));
            if at != cursor.get_untracked() {
                let spans = move || analysis.with_untracked(|a| (a.replace, a.typed));
                let before = spans();
                cursor.set(at);
                if spans() != before {
                    selected.set(0);
                }
            }
        }
        sync_scroll();
    };

    /* Writes `next` into the input with the caret at byte `at`, and focuses it. */
    let place = move |next: String, at: usize| {
        if let Some(input) = input_ref.get_untracked() {
            input.set_value(&next);
            let unit = u32::try_from(model::byte_to_utf16(&next, at)).unwrap_or(u32::MAX);
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "set_selection_range and focus return Result<(), JsValue>; failures are benign"
            )]
            {
                let _ = input.set_selection_range(unit, unit);
                let _ = input.focus();
            }
        }
        text.set(next);
        cursor.set(at);
        selected.set(0);
        sync_scroll();
    };

    /* Commits `expr` where the edit points; an append clears the box, an edit closes. */
    let commit = move |expr: Option<Expr>| {
        let landing = target.get_value();
        if landing == EditTarget::Append && expr.is_none() {
            return;
        }
        store
            .filter
            .update(|f| *f = crate::filter_ctx::commit(f, &landing, expr));
        match landing {
            EditTarget::Append => place(String::new(), 0),
            EditTarget::Conjunct(_) | EditTarget::Whole => on_close.run(()),
        }
    };

    /* Inserts suggestion `index`. */
    let pick = move |index: usize| {
        let inserted = analysis.with_untracked(|a| {
            let suggestion = a.suggestions.get(index)?;
            model::accept(&text.get_untracked(), a.replace, suggestion)
        });
        if let Some((next, at)) = inserted {
            place(next, at);
        }
    };

    /* Loads the edit's text and refreshes the catalog each time the palette opens. */
    Effect::new(move |_| {
        if open.get() {
            let current = edit.get_untracked();
            target.set_value(current.target.clone());
            let len = current.text.len();
            text.set(current.text);
            cursor.set(len);
            selected.set(0);
            catalog.refresh();
        }
    });

    /* Focuses the input, caret at the end, whenever it mounts while open. */
    Effect::new(move |_| {
        if open.get()
            && let Some(input) = input_ref.get()
        {
            let value = text.get_untracked();
            let unit = u32::try_from(model::byte_to_utf16(&value, value.len())).unwrap_or(u32::MAX);
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "focus and set_selection_range return Result<(), JsValue>; failures are benign"
            )]
            {
                let _ = input.focus();
                let _ = input.set_selection_range(unit, unit);
            }
        }
    });

    let has_options = move || analysis.with(|a| !a.suggestions.is_empty());
    let active_descendant =
        move || has_options().then(|| format!("palette-opt-{}", selected.get()));

    let on_keydown = move |e: web_sys::KeyboardEvent| match e.key().as_str() {
        "Escape" => {
            on_close.run(());
            e.prevent_default();
        }
        "ArrowDown" => {
            let count = analysis.with_untracked(|a| a.suggestions.len());
            if count > 0 {
                selected.update(|i| *i = i.saturating_add(1).min(count.saturating_sub(1)));
            }
            e.prevent_default();
        }
        "ArrowUp" => {
            selected.update(|i| *i = i.saturating_sub(1));
            e.prevent_default();
        }
        "Tab" => {
            if has_options() {
                pick(selected.get_untracked());
                e.prevent_default();
            }
        }
        "Enter" => {
            e.prevent_default();
            let chosen = analysis
                .with_untracked(|a| (!a.suggestions.is_empty()).then(|| selected.get_untracked()));
            let current = text.get_untracked();
            let at = cursor.get_untracked();
            let outcome = catalog.snapshot.with_untracked(|snapshot| match snapshot {
                Some(loaded) => model::enter(&current, at, chosen, loaded, today()),
                None => model::enter(&current, at, chosen, &Snapshot::default(), today()),
            });
            match outcome {
                model::Enter::Commit(expr) => commit(expr),
                model::Enter::Edit(next, next_at) => place(next, next_at),
            }
        }
        _ => {}
    };

    view! {
        <Show when=move || open.get()>
            <div class=style::overlay on:click=move |_| on_close.run(())>
                <div
                    class=style::modal
                    role="dialog"
                    aria-label="Command palette"
                    aria-modal="true"
                    on:click=move |e| e.stop_propagation()
                >
                    <div class=style::field>
                        <div
                            class=style::backdrop
                            node_ref=backdrop_ref
                            aria-hidden="true"
                            data-testid="palette-highlight"
                        >
                            {move || {
                                analysis
                                    .with(|a| {
                                        a.segments
                                            .iter()
                                            .map(|segment| {
                                                let class = format!(
                                                    "{} {}",
                                                    token_class(segment.kind),
                                                    mark_class(segment.mark),
                                                );
                                                view! {
                                                    <span
                                                        class=class
                                                        data-kind=segment.kind.map(model::kind_name)
                                                        data-mark=segment.mark.map(model::severity_name)
                                                    >
                                                        {segment.text.clone()}
                                                    </span>
                                                }
                                            })
                                            .collect_view()
                                    })
                            }}
                        </div>
                        <input
                            node_ref=input_ref
                            class=style::input
                            type="text"
                            role="combobox"
                            placeholder="search or filter"
                            aria-label="Search filters"
                            aria-expanded=has_options
                            aria-controls="palette-listbox"
                            aria-activedescendant=active_descendant
                            aria-describedby="palette-hint"
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || text.get()
                            on:input=move |e| {
                                text.set(event_target_value(&e));
                                selected.set(0);
                                read_cursor();
                            }
                            on:keydown=on_keydown
                            on:keyup=move |_| read_cursor()
                            on:click=move |_| read_cursor()
                            on:scroll=move |_| sync_scroll()
                        />
                    </div>
                    <div
                        id="palette-hint"
                        class=style::hint
                        data-testid="palette-hint"
                        aria-live="polite"
                    >
                        {move || {
                            analysis
                                .with(|a| {
                                    a.hints
                                        .iter()
                                        .map(|line| {
                                            view! {
                                                <div
                                                    class=hint_class(line.severity)
                                                    data-severity=line.severity.map(model::severity_name)
                                                >
                                                    {line.text.clone()}
                                                </div>
                                            }
                                        })
                                        .collect_view()
                                })
                        }}
                    </div>
                    <div id="palette-listbox" class=style::list role="listbox" aria-label="Filter">
                        {move || {
                            let sel = selected.get();
                            analysis
                                .with(|a| {
                                    let partial = text
                                        .with(|t| {
                                            t.get(a.typed.start..a.typed.end)
                                                .unwrap_or_default()
                                                .trim_start_matches('"')
                                                .to_owned()
                                        });
                                    a.suggestions
                                        .iter()
                                        .cloned()
                                        .enumerate()
                                        .map(|(idx, suggestion)| {
                                            let class = if idx == sel {
                                                format!("{} {}", style::item, style::item_selected)
                                            } else {
                                                style::item.to_owned()
                                            };
                                            let detail = suggestion.detail.clone();
                                            let name = match suggestion.kind {
                                                SuggestionKind::Account => {
                                                    view! {
                                                        <AccountPathLabel
                                                            path=suggestion.label.clone()
                                                            query=partial.clone()
                                                        />
                                                    }
                                                        .into_any()
                                                }
                                                SuggestionKind::Field
                                                | SuggestionKind::Key
                                                | SuggestionKind::Operator
                                                | SuggestionKind::Keyword
                                                | SuggestionKind::Tag
                                                | SuggestionKind::Value => {
                                                    suggestion.label.clone().into_any()
                                                }
                                            };
                                            view! {
                                                <div
                                                    class=class
                                                    id=format!("palette-opt-{idx}")
                                                    role="option"
                                                    aria-selected=idx == sel
                                                    data-insert=suggestion.insert.clone()
                                                    on:mousedown=move |e| e.prevent_default()
                                                    on:click=move |_| pick(idx)
                                                    on:mouseenter=move |_| selected.set(idx)
                                                >
                                                    <span class=style::item_name>{name}</span>
                                                    {(!detail.is_empty())
                                                        .then(|| {
                                                            view! {
                                                                <span class=style::item_detail>{detail.clone()}</span>
                                                            }
                                                        })}
                                                </div>
                                            }
                                        })
                                        .collect_view()
                                })
                        }}
                    </div>
                </div>
            </div>
        </Show>
    }
}
