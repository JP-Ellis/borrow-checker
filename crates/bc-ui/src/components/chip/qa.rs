//! QA showcase for [`Chip`](super::Chip), [`ChipRow`](super::ChipRow) and
//! [`ChipVariant`](crate::components::ChipVariant).

use leptos::prelude::*;

use super::Chip;
use super::ChipRow;
use crate::components::ChipVariant;
use crate::components::tag_token::TagToken;

/// Renders every [`ChipVariant`] in removable and non-removable form, plus a
/// [`ChipRow`] holding a mix (including a `TagToken`-labelled bare chip).
#[component]
pub fn ChipQa() -> impl IntoView {
    let noop = Callback::new(|()| {});
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "outlined — removable / plain"
                </p>
                <ChipRow>
                    <Chip
                        variant=ChipVariant::Outlined
                        on_remove=noop
                        remove_label="remove amazon filter"
                    >
                        "text: amazon"
                    </Chip>
                    <Chip variant=ChipVariant::Outlined>"account: 3 selected"</Chip>
                </ChipRow>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "filled — removable (currency alias style)"
                </p>
                <ChipRow>
                    <Chip
                        variant=ChipVariant::Filled
                        on_remove=noop
                        remove_label="Remove alias US$"
                    >
                        "US$"
                    </Chip>
                    <Chip variant=ChipVariant::Filled on_remove=noop remove_label="Remove alias A$">
                        "A$"
                    </Chip>
                </ChipRow>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "bare — TagToken label, nested in a bordered container"
                </p>
                <div style="display:inline-flex;gap:4px;padding:4px 8px;border:1px solid var(--bc-border);border-radius:var(--bc-radius-control);background:var(--bc-surface-alt);">
                    <Chip variant=ChipVariant::Bare on_remove=noop remove_label="remove groceries">
                        <TagToken label="groceries".to_owned() />
                    </Chip>
                    <Chip variant=ChipVariant::Bare on_remove=noop remove_label="remove rent">
                        <TagToken label="rent".to_owned() />
                    </Chip>
                </div>
            </section>
        </div>
    }
}
