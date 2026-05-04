//! BorrowChecker TUI — keyboard-first terminal interface built with ratatui.
//!
//! This crate provides the TUI application. The public modules (`context`,
//! `id`, `msg`, `screen`) are exposed for integration testing; the internal
//! modules (`app`, `chrome`) are implementation details.

pub(crate) mod app;
pub(crate) mod chrome;
pub mod context;
pub mod id;
pub mod mode;
pub mod msg;
pub mod screen;

use core::time::Duration;
use std::sync::Arc;

use app::Model;
use chrome::mount as mount_chrome;
use context::TuiContext;
use msg::Tab;
use screen::make_screen;
use tuirealm::application::Application;
use tuirealm::application::PollStrategy;
use tuirealm::event::NoUserEvent;
use tuirealm::listener::EventListenerCfg;
use tuirealm::terminal::CrosstermTerminalAdapter;
use tuirealm::terminal::TerminalAdapter as _;

/// Initialise the terminal, run the main event loop, then restore the terminal.
///
/// # Arguments
///
/// * `ctx` - Shared bc-core services and tokio handle.
///
/// # Errors
///
/// Returns an error if the terminal cannot be initialised or if the event loop
/// encounters an unrecoverable error.
#[inline]
pub fn run(ctx: Arc<TuiContext>) -> anyhow::Result<()> {
    let mut app: Application<id::Id, msg::Msg, NoUserEvent> = Application::init(
        EventListenerCfg::default().crossterm_input_listener(Duration::from_millis(20), 3),
    );

    let initial_tab = Tab::Accounts;
    mount_chrome(&mut app, initial_tab.clone())?;

    let mut initial_screen = make_screen(&initial_tab, Arc::clone(&ctx));
    initial_screen.mount(&mut app)?;
    let initial_focus = initial_screen.initial_focus();
    app.active(&initial_focus)?;

    let mut terminal = CrosstermTerminalAdapter::new()?;
    terminal.enable_raw_mode()?;
    terminal.enter_alternate_screen()?;

    let mut model = Model {
        app,
        active_screen: initial_screen,
        active_tab: initial_tab,
        mode: mode::AppMode::Normal,
        quit: false,
        redraw: true,
        terminal,
        ctx,
        last_focus: initial_focus,
    };

    while !model.quit {
        match model
            .app
            .tick(PollStrategy::Once(Duration::from_millis(10)))
        {
            Err(e) => {
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
