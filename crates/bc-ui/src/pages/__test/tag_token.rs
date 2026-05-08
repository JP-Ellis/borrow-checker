//! [`TagToken`] component QA page — renders with all syntax-colour tones.

use leptos::prelude::*;

use crate::components::tag_token::TagToken;

/// Tests the [`TagToken`] component across all syntax-colour tones.
#[component]
pub fn TagTokenTest() -> impl IntoView {
    view! {
        <div class="page">
            <h1 style="font-size: 20px; margin-bottom: 12px;">"TagToken component"</h1>

            <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px;">
                <TagToken label="default".to_owned() />
                <TagToken label="expenses:food".to_owned() />
                <TagToken label="a:very:deeply:nested:path".to_owned() />
            </div>

            <p style="color: var(--bc-ink-mute); font-size: 11px; margin-bottom: 8px;">
                "All tone variables:"
            </p>
            <div style="display: flex; flex-wrap: wrap; gap: 6px;">
                <TagToken label="keyword".to_owned() tone_var="--bc-keyword".to_owned() />
                <TagToken label="string".to_owned() tone_var="--bc-string".to_owned() />
                <TagToken label="number".to_owned() tone_var="--bc-number".to_owned() />
                <TagToken label="type".to_owned() tone_var="--bc-type".to_owned() />
                <TagToken label="fn".to_owned() tone_var="--bc-fn".to_owned() />
                <TagToken label="comment".to_owned() tone_var="--bc-comment".to_owned() />
            </div>
        </div>
    }
}
