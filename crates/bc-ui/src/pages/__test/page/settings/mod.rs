//! Route entry and QA component for `/__test/page/settings`.
//!
//! Renders the settings panel with mock data covering:
//! - A fully-populated snapshot (anchor date + plugin paths).
//! - A minimal snapshot (no anchor, no plugin paths).
//! - A snapshot where `config_file_path` is `None` (fallback hint text).

use bc_ipc::SettingsInfo;
use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;
use crate::pages::settings::SettingsPanel;

/// Display name shown in the QA index.
pub const TITLE: &str = "settings";
/// Route path.
pub const PATH: &str = "/__test/page/settings";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Read-only settings panel: populated, minimal, and no config path.";

/// All `/__test/page/settings/*` routes.
#[component(transparent)]
pub fn SettingsRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/settings") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=SettingsIndex />
            <Route path=path!("/populated") view=SettingsPopulatedQa />
            <Route path=path!("/minimal") view=SettingsMinimalQa />
            <Route path=path!("/no-config-path") view=SettingsNoConfigPathQa />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page for the settings QA section.
#[component]
pub fn SettingsIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// pages / settings"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title="Populated"
                    path="/__test/page/settings/populated"
                    description="Settings with anchor date and plugin paths."
                />
                <QaCard
                    title="Minimal"
                    path="/__test/page/settings/minimal"
                    description="Settings with no anchor date and no plugin paths."
                />
                <QaCard
                    title="No config path"
                    path="/__test/page/settings/no-config-path"
                    description="Settings where config_file_path is None (fallback hint)."
                />
            </div>
        </div>
    }
}

/// Builds a fully-populated mock [`SettingsInfo`].
fn populated_info() -> SettingsInfo {
    SettingsInfo::new(
        7,
        1,
        Some("2026-01-15".to_owned()),
        "AUD",
        "/Users/alice/Library/Application Support/borrow-checker/db.sqlite",
        vec![
            "/Users/alice/Library/Application Support/borrow-checker/plugins".to_owned(),
            "/usr/local/share/borrow-checker/plugins".to_owned(),
        ],
        Some("/Users/alice/Library/Application Support/borrow-checker/config.toml".to_owned()),
    )
}

/// Builds a minimal mock [`SettingsInfo`] with no anchor date and no plugins.
fn minimal_info() -> SettingsInfo {
    SettingsInfo::new(
        1,
        1,
        None,
        "USD",
        "/data/db.sqlite",
        vec![],
        Some("/home/alice/.config/borrow-checker/config.toml".to_owned()),
    )
}

/// Builds a mock [`SettingsInfo`] with no config file path (uses fallback hint).
fn no_config_path_info() -> SettingsInfo {
    SettingsInfo::new(7, 1, None, "AUD", "/data/db.sqlite", vec![], None)
}

/// Settings QA: fully-populated snapshot (anchor date + plugin paths).
#[component]
pub fn SettingsPopulatedQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:16px;">"Populated — anchor date + plugin paths"</h2>
            <SettingsPanel info=populated_info() />
        </div>
    }
}

/// Settings QA: minimal snapshot (no anchor date, no plugin paths).
#[component]
pub fn SettingsMinimalQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:16px;">"Minimal — no anchor, no plugins"</h2>
            <SettingsPanel info=minimal_info() />
        </div>
    }
}

/// Settings QA: snapshot with no config file path (fallback hint text).
#[component]
pub fn SettingsNoConfigPathQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:16px;">"No config path — fallback hint"</h2>
            <SettingsPanel info=no_config_path_info() />
        </div>
    }
}
