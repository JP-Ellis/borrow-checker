//! BorrowChecker TUI — keyboard-first terminal interface built with ratatui.
//!
//! Entry point. Initialises the tokio runtime, opens the database, mounts
//! chrome components, and runs the tui-realm event loop.

// Several Id variants, Msg variants, and TuiContext service fields are
// forward-declared for Phase 2–4 screens. Removed once all tabs are
// implemented and every item is reachable from `main`.
#![expect(
    dead_code,
    reason = "scaffold items for Phase 2–4 screens; removed once all screen implementations wire in every variant"
)]

mod app;
mod chrome;
mod context;
mod id;
mod mode;
mod msg;
mod screen;

use core::time::Duration;
use std::path::PathBuf;
use std::sync::Arc;

use app::Model;
use chrome::mount as mount_chrome;
use context::TuiContext;
use msg::Tab;
use screen::make_screen;
use tuirealm::Application;
use tuirealm::EventListenerCfg;
use tuirealm::NoUserEvent;
use tuirealm::PollStrategy;
use tuirealm::Update as _;

fn main() -> anyhow::Result<()> {
    // Build a multi-thread tokio runtime so Handle::block_on works from
    // the synchronous tui-realm loop.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Resolve database path (use --db-path arg or fall back to XDG default).
    let db_path = db_path_from_args();

    // Open DB synchronously before entering the TUI.
    let ctx = Arc::new(rt.block_on(TuiContext::open(&db_path))?);

    // Run the TUI in the synchronous context of the main thread.
    // TuiContext::block_on uses Handle::block_on, which is safe here because
    // we are NOT inside an async execution context.
    run(ctx)?;

    Ok(())
}

/// Returns the database path from `--db-path <path>` CLI argument, or a
/// default location under the user's config directory.
fn db_path_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--db-path" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    // Default: ~/.local/share/borrow-checker/borrow-checker.db
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("borrow-checker")
        .join("borrow-checker.db")
}

/// Initialise the terminal, run the main event loop, then restore the terminal.
fn run(ctx: Arc<TuiContext>) -> anyhow::Result<()> {
    // Initialise tui-realm application.
    let mut app: Application<id::Id, msg::Msg, NoUserEvent> = Application::init(
        EventListenerCfg::default()
            .crossterm_input_listener(Duration::from_millis(20), 3)
            .poll_timeout(Duration::from_millis(10)),
    );

    // Mount chrome (permanent components).
    let initial_tab = Tab::Accounts;
    mount_chrome(&mut app, initial_tab.clone())?;

    // Mount the initial screen.
    let mut initial_screen = make_screen(&initial_tab, Arc::clone(&ctx));
    initial_screen.mount(&mut app)?;
    app.active(&initial_screen.initial_focus())?;

    // Build the model.
    let terminal = tuirealm::terminal::TerminalBridge::init_crossterm()?;
    let mut model = Model {
        app,
        active_screen: initial_screen,
        active_tab: initial_tab,
        mode: mode::AppMode::Normal,
        quit: false,
        redraw: true,
        terminal,
        ctx,
    };

    // Main event loop.
    while !model.quit {
        match model.app.tick(PollStrategy::Once) {
            Err(e) => {
                // Restore terminal before propagating the error.
                model.terminal.restore()?;
                anyhow::bail!("tui-realm tick error: {e}");
            }
            Ok(messages) if !messages.is_empty() => {
                model.redraw = true;
                for msg in messages {
                    let mut next = Some(msg);
                    while let Some(m) = next {
                        next = model.update(Some(m));
                    }
                }
            }
            _ => {}
        }

        if model.redraw {
            model.render();
            model.redraw = false;
        }
    }

    model.terminal.restore()?;
    Ok(())
}
