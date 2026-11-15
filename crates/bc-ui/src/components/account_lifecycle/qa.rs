//! QA showcase for the lifecycle panels' preview states — fixture previews,
//! no IPC.

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::ArchivePreviewDto;
use bc_ipc::ClosePreviewDto;
use jiff::civil::date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::archive_details;
use super::close_details;
use super::style;

/// A fixture account reference under `Assets`.
fn account(id: &str, path: &str) -> AccountRef {
    AccountRef::new(id, format!("Assets :: {path}"))
}

/// One preview, framed as its panel renders it.
fn frame(title: &'static str, body: AnyView) -> impl IntoView {
    view! {
        <h2 style="font-family:var(--bc-font-mono);font-size:12px;color:var(--bc-ink-mute);">
            {title}
        </h2>
        <section class=style::panel aria-label=title>
            {body}
            <div class=style::actions>
                <button class=format!("{} {}", style::btn, style::btn_primary) disabled=true>
                    "confirm"
                </button>
            </div>
        </section>
    }
}

/// QA fixture: close previews with a cascade, balances and late postings,
/// and a clean close; archive previews with a cascade and the nudge.
#[component]
pub fn AccountLifecycleQa() -> impl IntoView {
    let on = date(2026, 10, 31);
    let busy_close = ClosePreviewDto::builder()
        .cascade(vec![
            account("b", "Coin Jar :: Spare"),
            account("c", "Coin Jar :: Travel"),
        ])
        .balances(vec![
            (
                account("a", "Coin Jar"),
                vec![
                    Amount::new(Decimal::new(12_050, 2), "AUD"),
                    Amount::new(Decimal::new(-3_000, 2), "USD"),
                ],
            ),
            (
                account("c", "Coin Jar :: Travel"),
                vec![Amount::new(Decimal::new(500, 2), "AUD")],
            ),
        ])
        .postings_after(3)
        .build();
    let clean_close = ClosePreviewDto::builder()
        .cascade(vec![])
        .balances(vec![])
        .postings_after(0)
        .build();
    let archive_cascade = ArchivePreviewDto::builder()
        .cascade(vec![account("b", "Coin Jar :: Spare")])
        .no_close_date(true)
        .build();
    let archive_clean = ArchivePreviewDto::builder()
        .cascade(vec![])
        .no_close_date(false)
        .build();

    view! {
        <div style="padding:24px;max-width:720px;display:flex;flex-direction:column;gap:12px;">
            {frame(
                "close: cascade, balances, late postings",
                close_details(&busy_close, on, RwSignal::new(false)).into_any(),
            )}
            {frame(
                "close: nothing to warn about",
                close_details(&clean_close, on, RwSignal::new(false)).into_any(),
            )}
            {frame(
                "archive: cascade ticked, no close date",
                archive_details(&archive_cascade, RwSignal::new(true)).into_any(),
            )}
            {frame(
                "archive: nothing to warn about",
                archive_details(&archive_clean, RwSignal::new(false)).into_any(),
            )}
        </div>
    }
}
