//! The Profiles section: one row per import profile, its config and Preview.

use bc_ipc::ImportProfileInfo;
use bc_ipc::ImportProfiles;
use leptos::prelude::*;

use crate::pages::import::ImportCtx;
use crate::pages::import::RunPanel;
use crate::pages::import::model::RunState;
use crate::pages::import::model::block_reason;
use crate::pages::import::style;

/// The banner shown while `import.documents-root` is unset.
const DOCUMENTS_ROOT_BANNER: &str = "import.documents-root is not set. Importers read their \
     statements from that directory: set it in the config file and restart, then every profile \
     can preview.";

/// The profile table, with the documents-root banner above it when unset.
#[component]
pub fn ProfilesTable(
    /// The loaded profile list.
    data: ImportProfiles,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    let root_set = data.documents_root_set;
    let banner = (!root_set).then(|| {
        view! {
            <p class=style::banner role="alert" data-testid="import-root-banner">
                {DOCUMENTS_ROOT_BANNER}
            </p>
        }
    });
    let body = if data.profiles.is_empty() {
        view! {
            <p class=style::empty_state>
                "No import profiles. Create one with "
                <code class=style::mono>"borrow-checker profile create"</code>
                ", then reload this page."
            </p>
        }
        .into_any()
    } else {
        view! {
            <table class=style::table aria-label="import profiles">
                <thead>
                    <tr>
                        <th class=style::th>"Name"</th>
                        <th class=style::th>"Importer"</th>
                        <th class=style::th>"Action"</th>
                    </tr>
                </thead>
                {data
                    .profiles
                    .into_iter()
                    .map(|profile| profile_rows(profile, root_set, ctx))
                    .collect::<Vec<_>>()}
            </table>
        }
        .into_any()
    };
    view! {
        {banner}
        {body}
    }
}

/// One profile as a `<tbody>`: its row, its config disclosure and, when it is
/// the active profile, its run panel.
///
/// # Arguments
///
/// * `profile` - The profile.
/// * `root_set` - Whether `import.documents-root` is configured.
/// * `ctx` - The page state.
fn profile_rows(profile: ImportProfileInfo, root_set: bool, ctx: ImportCtx) -> impl IntoView {
    let reason = block_reason(&profile, root_set);
    let blocked = reason.is_some();
    let testid = format!("import-profile-row-{}", profile.name);
    let open_name = profile.name.clone();
    let click_name = profile.name.clone();
    let is_open = move || {
        ctx.active
            .with(|a| a.as_deref() == Some(open_name.as_str()))
    };
    view! {
        <tbody data-testid=testid>
            <tr class=style::row>
                <td class=style::td>
                    <span class=style::name>{profile.name}</span>
                </td>
                <td class=style::td>
                    <code class=style::mono>{profile.importer}</code>
                </td>
                <td class=style::td>
                    <button
                        type="button"
                        class=style::btn
                        disabled=move || blocked || ctx.run.with(RunState::busy)
                        on:click=move |_| ctx.preview.run(click_name.clone())
                    >
                        "Preview"
                    </button>
                    {reason.map(|r| view! { <span class=style::reason>{r}</span> })}
                </td>
            </tr>
            <tr>
                <td class=style::td colspan="3">
                    <details class=style::config>
                        <summary>"config"</summary>
                        <pre class=style::config>{profile.config_text}</pre>
                    </details>
                </td>
            </tr>
            <Show when=is_open>
                <tr>
                    <td class=style::panel_cell colspan="3">
                        <RunPanel />
                    </td>
                </tr>
            </Show>
        </tbody>
    }
}

/// Loading skeleton for the profile table, sized like two loaded rows.
#[component]
pub fn ProfilesSkeleton() -> impl IntoView {
    view! {
        <table class=style::table aria-label="import profiles loading">
            <tbody>
                {core::iter::repeat_with(|| {
                        view! {
                            <tr class=style::row>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:8rem" />
                                </td>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:3rem" />
                                </td>
                                <td class=style::td>
                                    <span class=style::skeleton_bar style="width:5rem" />
                                </td>
                            </tr>
                        }
                    })
                    .take(2)
                    .collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
