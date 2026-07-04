//! QA showcase for [`super::BackupPanel`].

use leptos::prelude::*;

use super::BackupPanel;

/// Renders [`BackupPanel`] directly; it fetches its own state via IPC.
#[component]
pub fn BackupPanelQa() -> impl IntoView {
    view! { <BackupPanel /> }
}
