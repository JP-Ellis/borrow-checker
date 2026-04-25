//! Application model and top-level message handler.
//!
//! [`Model`] owns the tui-realm [`Application`], the active [`Screen`],
//! and all cross-cutting state. It implements [`Update<Msg>`] which is
//! called by the tui-realm event loop for each message.

use std::sync::Arc;

use tuirealm::Application;
use tuirealm::NoUserEvent;
use tuirealm::Update;
use tuirealm::terminal::CrosstermTerminalAdapter;
use tuirealm::terminal::TerminalBridge;

use crate::context::TuiContext;
use crate::id::Id;
use crate::mode::AppMode;
use crate::msg::Msg;
use crate::msg::Tab;
use crate::screen::Screen;

/// Top-level application state.
#[non_exhaustive]
pub struct Model {
    /// The tui-realm component registry and event loop.
    pub app: Application<Id, Msg, NoUserEvent>,
    /// The currently displayed screen (tab content).
    pub active_screen: Box<dyn Screen>,
    /// Which tab is currently active.
    pub active_tab: Tab,
    /// Current input mode.
    pub mode: AppMode,
    /// When `true`, the main loop exits after the current tick.
    pub quit: bool,
    /// When `true`, the terminal is redrawn on the next loop iteration.
    pub redraw: bool,
    /// The terminal bridge used to draw frames.
    pub terminal: TerminalBridge<CrosstermTerminalAdapter>,
    /// Shared bc-core services.
    pub ctx: Arc<TuiContext>,
}

impl Update<Msg> for Model {
    #[inline]
    fn update(&mut self, msg: Option<Msg>) -> Option<Msg> {
        #[expect(
            clippy::shadow_reuse,
            reason = "idiomatic destructuring of Option parameter via ?"
        )]
        let msg = msg?;
        self.redraw = true;
        match msg {
            Msg::AppQuit => {
                self.quit = true;
                None
            }
            Msg::TabSwitch(tab) => {
                self.switch_tab(tab);
                None
            }
            Msg::ModeChange(mode) => {
                self.mode = mode;
                None
            }
            Msg::HelpToggle => {
                self.toggle_help();
                None
            }
            Msg::FocusChange(id) => {
                #[expect(
                    clippy::unused_result_ok,
                    reason = "focus errors logged to stderr since we are in raw terminal mode"
                )]
                if let Err(e) = self.app.active(&id) {
                    eprintln!("failed to set focus: {e}");
                }
                None
            }
            Msg::Chrome(_) => {
                // Chrome messages reserved for future use; currently no-op.
                None
            }
            other @ (Msg::Accounts(_) | Msg::Budget(_) | Msg::Reports(_)) => {
                self.active_screen.handle(other)
            }
        }
    }
}

impl Model {
    /// Switch to a different tab, unmounting the old screen and mounting the new one.
    #[expect(
        clippy::print_stderr,
        reason = "mount/focus errors are logged to stderr since we are in raw terminal mode"
    )]
    fn switch_tab(&mut self, tab: Tab) {
        if tab == self.active_tab {
            return;
        }
        self.active_screen.unmount(&mut self.app);
        self.active_screen = crate::screen::make_screen(&tab, Arc::clone(&self.ctx));
        self.active_tab = tab;
        if let Err(e) = self.active_screen.mount(&mut self.app) {
            eprintln!("failed to mount screen: {e}");
        }
        let focus = self.active_screen.initial_focus();
        if let Err(e) = self.app.active(&focus) {
            eprintln!("failed to set focus: {e}");
        }
    }

    /// Show or hide the help overlay.
    #[expect(
        clippy::print_stderr,
        reason = "focus errors logged to stderr since we are in raw terminal mode"
    )]
    fn toggle_help(&mut self) {
        use tuirealm::AttrValue;
        use tuirealm::Attribute;

        use crate::id::ChromeId;

        // Query current display state.
        let currently_shown = self
            .app
            .query(&Id::Chrome(ChromeId::HelpOverlay), Attribute::Display)
            .ok()
            .flatten()
            .is_some_and(|v| matches!(v, AttrValue::Flag(true)));

        #[expect(
            clippy::unused_result_ok,
            reason = "best-effort attribute updates; component may not exist yet during startup"
        )]
        if currently_shown {
            // Hide overlay and restore focus to the active screen.
            self.app
                .attr(
                    &Id::Chrome(ChromeId::HelpOverlay),
                    Attribute::Display,
                    AttrValue::Flag(false),
                )
                .ok();
            let focus = self.active_screen.initial_focus();
            if let Err(e) = self.app.active(&focus) {
                eprintln!("failed to restore focus after closing help: {e}");
            }
        } else {
            // Build help content from active screen's keybindings, then show.
            let bindings = self.active_screen.keybindings(&self.mode);
            let content = format_help_content(&bindings);
            self.app
                .attr(
                    &Id::Chrome(ChromeId::HelpOverlay),
                    Attribute::Text,
                    AttrValue::String(content),
                )
                .ok();
            self.app
                .attr(
                    &Id::Chrome(ChromeId::HelpOverlay),
                    Attribute::Display,
                    AttrValue::Flag(true),
                )
                .ok();
            // Give focus to the overlay so keyboard events reach its on() handler.
            if let Err(e) = self.app.active(&Id::Chrome(ChromeId::HelpOverlay)) {
                eprintln!("failed to focus help overlay: {e}");
            }
        }
    }

    /// Render the full UI: chrome + active screen.
    ///
    /// Splits the terminal into three rows: tab bar (1 line), screen area
    /// (remaining), status bar (1 line). The help overlay floats over all of
    /// this when visible.
    #[expect(clippy::expect_used, reason = "terminal draw failure is unrecoverable")]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 3 chunks to match the 3 constraints"
    )]
    pub fn render(&mut self) {
        use tuirealm::ratatui::layout::Constraint;
        use tuirealm::ratatui::layout::Direction;
        use tuirealm::ratatui::layout::Layout;

        use crate::id::ChromeId;

        let app = &mut self.app;
        let screen = &mut self.active_screen;

        self.terminal
            .raw_mut()
            .draw(|frame| {
                let area = frame.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(0),
                        Constraint::Length(1),
                    ])
                    .split(area);
                app.view(&Id::Chrome(ChromeId::TabBar), frame, chunks[0]);
                screen.view(app, frame, chunks[1]);
                app.view(&Id::Chrome(ChromeId::StatusBar), frame, chunks[2]);
                // Help overlay always rendered; hidden via Display attribute when inactive.
                app.view(&Id::Chrome(ChromeId::HelpOverlay), frame, area);
            })
            .expect("terminal draw failed");
    }
}

/// Format the `Vec<KeyBinding>` into a displayable string for the help overlay.
#[expect(
    clippy::format_push_string,
    reason = "the write! alternative requires importing fmt::Write and handling its infallible error"
)]
fn format_help_content(bindings: &[crate::screen::KeyBinding]) -> String {
    let mut out = String::from("Key        Action                Mode\n");
    out.push_str("─────────────────────────────────────\n");
    for b in bindings {
        out.push_str(&format!("{:<10} {:<21} {}\n", b.key, b.action, b.mode));
    }
    out.push_str("\n  i=Insert  v=Visual  Esc=Normal");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::AppMode;
    use crate::screen::KeyBinding;

    #[test]
    fn format_help_content_includes_mode_footer() {
        let bindings = vec![KeyBinding {
            key: "j / ↓".into(),
            action: "Move down".into(),
            mode: AppMode::Normal,
        }];
        let content = format_help_content(&bindings);
        assert!(content.contains("j / ↓"));
        assert!(content.contains("Move down"));
        assert!(content.contains("i=Insert"));
    }
}
