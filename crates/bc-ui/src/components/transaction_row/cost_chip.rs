//! The cost chip on a posting row and its inline editor.

use bc_ipc::Amount;
use leptos::prelude::*;
use leptos::web_sys;

use super::style;
use crate::components::ChipVariant;
use crate::components::chip::Chip;
use crate::components::num::format_amount;
use crate::components::num::meta::display_meta_for;
use crate::components::transaction_row::cost::CostBuffers;
use crate::components::transaction_row::cost::cost_chip_text;
use crate::components::transaction_row::cost::cost_from_buffers;
use crate::components::transaction_row::edit_ctx::TxEditCtx;
use crate::components::transaction_row::editable::EditablePosting;

/// Renders the cost chip for posting `uid`, or the `{ cost` tinytool when the
/// leg has none, or the inline editor while it is open.
///
/// Local buffers mirror the spread editor: strings synced into the working
/// buffer by an `Effect` on every change, so the weight hint and the balance
/// line follow the typing. Blur, Enter and Escape close the editor, but only
/// when the basis parses; a negative or unparsable basis keeps the editor
/// open, writes the message to the leg's `cost_error` (which the row shows
/// under the amount box and which blocks saving) until it is fixed or
/// cleared with the × control. A blank basis clears the cost.
///
/// # Arguments
///
/// * `uid` - Stable identity of the posting in the buffer.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn CostChip(
    /// Stable identity of this posting in the buffer.
    uid: u64,
) -> impl IntoView {
    let ctx = expect_context::<TxEditCtx>();
    let working = ctx.working;
    let currencies = ctx.currencies;
    let reset_epoch = ctx.reset_epoch;

    let seed = working.with_untracked(|w| {
        CostBuffers::from_cost(
            w.postings
                .iter()
                .find(|p| p.uid == uid)
                .and_then(|p| p.cost.as_ref()),
        )
    });
    let is_total = RwSignal::new(seed.is_total);
    let basis = RwSignal::new(seed.basis);
    let date = RwSignal::new(seed.date);
    let label = RwSignal::new(seed.label);
    let editing = RwSignal::new(false);
    let show_date = RwSignal::new(false);
    let show_label = RwSignal::new(false);

    // MARK: Forward sync — buffers → working.cost / working.cost_error.
    Effect::new(move |_| {
        let parsed = cost_from_buffers(
            &currencies.get(),
            is_total.get(),
            &basis.get(),
            &date.get(),
            &label.get(),
        );
        let Some(i) = working.with_untracked(|w| w.postings.iter().position(|p| p.uid == uid))
        else {
            return;
        };
        // A parse failure leaves the last valid `cost` in place and records
        // the message beside it; the save path refuses the leg while it is
        // set. Write only on a change so a no-op sync does not dirty the row.
        let (cost, cost_error) = match parsed {
            Ok(cost) => (Some(cost), None),
            Err(message) => (None, Some(message)),
        };
        let changed = working.with_untracked(|w| {
            w.postings.get(i).is_some_and(|p| {
                cost.as_ref().is_some_and(|c| p.cost != *c) || p.cost_error != cost_error
            })
        });
        if changed {
            working.update(|w| {
                if let Some(p) = w.postings.get_mut(i) {
                    if let Some(c) = cost {
                        p.cost = c;
                    }
                    p.cost_error = cost_error;
                }
            });
        }
    });

    // MARK: Re-seed after an external reset (discard / escape).
    Effect::new(move |_| {
        reset_epoch.get();
        let b = working.with_untracked(|w| {
            CostBuffers::from_cost(
                w.postings
                    .iter()
                    .find(|p| p.uid == uid)
                    .and_then(|p| p.cost.as_ref()),
            )
        });
        if is_total.get_untracked() != b.is_total {
            is_total.set(b.is_total);
        }
        if basis.get_untracked() != b.basis {
            basis.set(b.basis);
        }
        if date.get_untracked() != b.date {
            date.set(b.date);
        }
        if label.get_untracked() != b.label {
            label.set(b.label);
        }
    });

    let has_error = move || {
        working.with_untracked(|w| {
            w.postings
                .iter()
                .find(|p| p.uid == uid)
                .is_some_and(|p| p.cost_error.is_some())
        })
    };
    let has_cost = move || {
        working.with(|w| {
            w.postings
                .iter()
                .find(|p| p.uid == uid)
                .is_some_and(|p| p.cost.is_some())
        })
    };
    let is_elided = move || {
        working.with(|w| {
            w.postings
                .iter()
                .find(|p| p.uid == uid)
                .is_none_or(EditablePosting::is_elided)
        })
    };

    let chip_text = move || {
        working.with(|w| {
            let cost = w.postings.iter().find(|p| p.uid == uid)?.cost.as_ref()?;
            let currencies = currencies.get();
            Some(cost_chip_text(cost, |a: &Amount| {
                let meta = display_meta_for(&a.currency_code, &currencies);
                format_amount(&a.value, &meta)
            }))
        })
    };

    let open_editor = move || {
        show_date.set(!date.get_untracked().is_empty());
        show_label.set(!label.get_untracked().is_empty());
        editing.set(true);
    };
    let open = move |_| open_editor();
    let open_key = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        if key == "Enter" || key == " " {
            ev.prevent_default();
            open_editor();
        }
    };
    let close_key = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        if (key == "Enter" || key == "Escape") && !has_error() {
            editing.set(false);
        }
    };
    let clear = Callback::new(move |()| {
        basis.set(String::new());
        date.set(String::new());
        label.set(String::new());
        editing.set(false);
    });

    // The segment toggle, `+ date` and `+ label` buttons are focusable controls
    // inside the editor, so focus leaving one of them for another is not a
    // reason to close — only focus leaving the whole editor wrapper is. Mirrors
    // `crate::components::meta_editor`'s key-cell focusout guard. `uid` (not a
    // captured `String`) keeps this closure `Copy`, matching `open`/`close`
    // above, so it can be reused across every reactive re-render of the editor.
    let editor_id = format!("cost-edit-{uid}");
    let on_editor_focusout = move |ev: web_sys::FocusEvent| {
        let selector = format!("#cost-edit-{uid}");
        let staying = ev
            .related_target()
            .and_then(|target| {
                web_sys::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(target).ok()
            })
            .and_then(|element| element.closest(&selector).ok().flatten())
            .is_some();
        if !staying && !has_error() {
            editing.set(false);
        }
    };

    view! {
        {move || {
            (!has_cost() && !editing.get() && !is_elided())
                .then(|| {
                    view! {
                        <button class=style::tinytool on:click=open type="button">
                            "{ cost"
                        </button>
                    }
                })
        }}
        {move || {
            (has_cost() && !editing.get())
                .then(|| {
                    view! {
                        <Chip variant=ChipVariant::Accent on_remove=clear remove_label="clear cost">
                            <span
                                on:click=open
                                on:keydown=open_key
                                role="button"
                                tabindex="0"
                                data-testid="posting-cost-chip"
                            >
                                {chip_text}
                            </span>
                        </Chip>
                    }
                })
        }}
        {move || {
            editing
                .get()
                .then(|| {
                    view! {
                        <div
                            id=editor_id.clone()
                            class=style::cost_edit
                            on:focusout=on_editor_focusout
                            data-testid="posting-cost-editor"
                        >
                            <span class=style::cost_seg>
                                <button
                                    class=move || seg_class(!is_total.get())
                                    on:click=move |_| is_total.set(false)
                                    type="button"
                                >
                                    "{ }"
                                </button>
                                <button
                                    class=move || seg_class(is_total.get())
                                    on:click=move |_| is_total.set(true)
                                    type="button"
                                >
                                    "{{ }}"
                                </button>
                            </span>
                            <input
                                class=format!(
                                    "{} {}",
                                    style::cost_edit_input,
                                    style::cost_edit_basis,
                                )
                                prop:value=move || basis.get()
                                on:input=move |ev| basis.set(event_target_value(&ev))
                                on:keydown=close_key
                                placeholder="A$105.00"
                                type="text"
                                data-testid="posting-cost-basis"
                            />
                            {move || {
                                show_date
                                    .get()
                                    .then(|| {
                                        view! {
                                            <span class=style::cost_edit_lbl>"on"</span>
                                            <input
                                                class=format!(
                                                    "{} {}",
                                                    style::cost_edit_input,
                                                    style::cost_edit_date,
                                                )
                                                prop:value=move || date.get()
                                                on:input=move |ev| date.set(event_target_value(&ev))
                                                on:keydown=close_key
                                                placeholder="YYYY-MM-DD"
                                                type="text"
                                            />
                                        }
                                    })
                            }}
                            {move || {
                                show_label
                                    .get()
                                    .then(|| {
                                        view! {
                                            <span class=style::cost_edit_lbl>"\""</span>
                                            <input
                                                class=format!(
                                                    "{} {}",
                                                    style::cost_edit_input,
                                                    style::cost_edit_label,
                                                )
                                                prop:value=move || label.get()
                                                on:input=move |ev| label.set(event_target_value(&ev))
                                                on:keydown=close_key
                                                placeholder="label"
                                                type="text"
                                            />
                                            <span class=style::cost_edit_lbl>"\""</span>
                                        }
                                    })
                            }}
                            {move || {
                                (!show_date.get())
                                    .then(|| {
                                        view! {
                                            <button
                                                class=style::cost_more
                                                on:click=move |_| show_date.set(true)
                                                type="button"
                                            >
                                                "+ date"
                                            </button>
                                        }
                                    })
                            }}
                            {move || {
                                (!show_label.get())
                                    .then(|| {
                                        view! {
                                            <button
                                                class=style::cost_more
                                                on:click=move |_| show_label.set(true)
                                                type="button"
                                            >
                                                "+ label"
                                            </button>
                                        }
                                    })
                            }}
                            <button
                                class=style::spread_edit_x
                                on:click=move |_| clear.run(())
                                type="button"
                                aria-label="clear cost"
                            >
                                "\u{00D7}"
                            </button>
                        </div>
                    }
                })
        }}
    }
}

/// Class list for one half of the `{ }` / `{{ }}` segment.
fn seg_class(on: bool) -> String {
    if on {
        format!("{} {}", style::cost_seg_btn, style::cost_seg_on)
    } else {
        style::cost_seg_btn.to_owned()
    }
}
