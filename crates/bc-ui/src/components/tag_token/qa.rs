//! QA page for [`super::TagToken`].

use leptos::prelude::*;

use super::TagToken;

/// Renders [`TagToken`] across all syntax-colour tone variables.
#[component]
pub fn TagTokenQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:24px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "default tone and various paths"
                </p>
                <div style="display:flex;flex-wrap:wrap;gap:6px">
                    <TagToken label="default".to_owned() />
                    <TagToken label="expenses:food".to_owned() />
                    <TagToken label="a:very:deeply:nested:path".to_owned() />
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "all syntax-colour tone variables"
                </p>
                <div style="display:flex;flex-wrap:wrap;gap:6px">
                    <TagToken label="keyword".to_owned() tone_var="--bc-keyword".to_owned() />
                    <TagToken label="string".to_owned() tone_var="--bc-string".to_owned() />
                    <TagToken label="number".to_owned() tone_var="--bc-number".to_owned() />
                    <TagToken label="type".to_owned() tone_var="--bc-type".to_owned() />
                    <TagToken label="fn".to_owned() tone_var="--bc-fn".to_owned() />
                    <TagToken label="comment".to_owned() tone_var="--bc-comment".to_owned() />
                </div>
            </section>

        </div>
    }
}
