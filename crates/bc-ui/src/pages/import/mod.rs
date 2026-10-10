//! Import page: run a profile with a per-row preview, then commit it.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the page source with its SCSS module file"
    )
)]

#[cfg(target_arch = "wasm32")]
pub(crate) mod discard;
pub(crate) mod discard_model;
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) mod fixtures;
#[cfg(target_arch = "wasm32")]
pub(crate) mod history;
pub(crate) mod history_model;
pub(crate) mod links;
pub(crate) mod model;
#[cfg(target_arch = "wasm32")]
pub(crate) mod outcome;
#[cfg(target_arch = "wasm32")]
pub(crate) mod preview;
#[cfg(target_arch = "wasm32")]
pub(crate) mod profiles;
#[cfg(target_arch = "wasm32")]
pub(crate) mod refresh;
#[cfg(target_arch = "wasm32")]
pub(crate) mod rows;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos_router::hooks::use_query_map;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::error_banner::ErrorBanner;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::history::HistorySection;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::model::RunState;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::outcome::FailureBanner;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::outcome::OutcomePanel;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::preview::PreviewPanel;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::profiles::ProfilesSkeleton;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::profiles::ProfilesTable;
#[cfg(target_arch = "wasm32")]
use crate::pages::import::refresh::SharedStores;

#[cfg(target_arch = "wasm32")]
import_style!(pub(crate) style, "import.module.scss");

/// Page state shared by the profile rows and the run panel.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub(crate) struct ImportCtx {
    /// The profile whose row is expanded, if any.
    pub active: RwSignal<Option<String>>,
    /// The expanded row's run.
    pub run: RwSignal<RunState>,
    /// Starts a preview of the named profile; ignored while a run is in flight.
    pub preview: Callback<String>,
    /// Commits the open preview; ignored unless one is open and idle.
    pub commit: Callback<()>,
    /// Clears the panel.
    pub dismiss: Callback<()>,
}

/// Import page: Profiles, each expanding into its run panel.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn ImportPage() -> impl IntoView {
    let query = use_query_map();
    let profiles = LocalResource::new(bc_ipc::client::list_import_profiles);
    let active = RwSignal::new(Option::<String>::None);
    let run = RwSignal::new(RunState::Idle);

    let preview = Callback::new(move |name: String| {
        if run.with_untracked(RunState::busy) {
            return;
        }
        active.set(Some(name.clone()));
        run.set(RunState::Previewing);
        leptos::task::spawn_local(async move {
            let reply = bc_ipc::client::preview_import(&name).await;
            run.set(model::after_preview(reply));
        });
    });
    let history_version = RwSignal::new(0_u32);
    let stores = SharedStores::from_context();
    // The `Previewed` match is the single-flight guard: the state flips to
    // `Committing` before the call is awaited, so a second click finds no
    // preview to commit, and `preview` refuses while `busy`.
    let commit = Callback::new(move |()| {
        let RunState::Previewed { preview: open, .. } = run.get_untracked() else {
            return;
        };
        let Some(profile) = active.get_untracked() else {
            return;
        };
        let fingerprint = open.fingerprint.clone();
        run.set(RunState::Committing { preview: open });
        leptos::task::spawn_local(async move {
            let reply = bc_ipc::client::commit_import(&profile, &fingerprint).await;
            let next = model::after_commit(reply);
            let refresh = next.refresh_after_commit();
            run.set(next);
            if refresh {
                history_version.update(|v| *v = v.wrapping_add(1));
                stores.refresh();
            }
        });
    });
    let dismiss = Callback::new(move |()| {
        run.set(RunState::Idle);
        active.set(None);
    });
    provide_context(ImportCtx {
        active,
        run,
        preview,
        commit,
        dismiss,
    });

    // `?profile=` previews once per distinct value, after the list loads.
    let auto_started = StoredValue::new(Option::<String>::None);
    Effect::new(move |_| {
        let wanted = query.with(|q| q.get("profile"));
        let Some(Ok(list)) = profiles.get() else {
            return;
        };
        let Some(target) = model::auto_start_target(wanted.as_deref(), &list) else {
            return;
        };
        if auto_started.with_value(|s| s.as_deref() == Some(target.as_str())) {
            return;
        }
        auto_started.set_value(Some(target.clone()));
        preview.run(target);
    });

    view! {
        <div class=format!("page {}", style::page_import)>
            <h1 class=style::heading>"Import"</h1>
            <section class=style::section aria-labelledby="import-profiles-title">
                <h2 id="import-profiles-title" class=style::section_title>
                    "Profiles"
                </h2>
                {move || match profiles.get() {
                    None => view! { <ProfilesSkeleton /> }.into_any(),
                    Some(Err(e)) => {
                        view! {
                            <ErrorBanner message=format!("Failed to load import profiles: {e}") />
                        }
                            .into_any()
                    }
                    Some(Ok(data)) => view! { <ProfilesTable data=data /> }.into_any(),
                }}
            </section>
            <HistorySection version=history_version stores=stores />
        </div>
    }
}

/// The expanded profile row's content for the current [`RunState`].
#[cfg(target_arch = "wasm32")]
#[component]
pub fn RunPanel() -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    move || run_view(ctx.run.get())
}

/// The panel content for one [`RunState`].
#[cfg(target_arch = "wasm32")]
fn run_view(state: RunState) -> AnyView {
    match state {
        RunState::Idle => ().into_any(),
        RunState::Previewing => view! {
            <p class=style::spinner role="status" aria-live="polite">
                "Parsing the source\u{2026}"
            </p>
        }
        .into_any(),
        RunState::Previewed { preview, changed } => {
            view! { <PreviewPanel preview=*preview changed=changed /> }.into_any()
        }
        RunState::Committing { preview } => {
            view! { <PreviewPanel preview=*preview changed=false /> }.into_any()
        }
        RunState::Done(result) => view! { <OutcomePanel result=*result /> }.into_any(),
        RunState::Failed(failure) => view! { <FailureBanner failure=failure /> }.into_any(),
        RunState::Error(message) => view! { <ErrorBanner message=message /> }.into_any(),
    }
}
