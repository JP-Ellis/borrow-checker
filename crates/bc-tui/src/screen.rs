//! The [`Screen`] trait and associated types.

pub mod accounts;
pub mod budget;
pub mod reports;

use tuirealm::application::Application;
use tuirealm::event::NoUserEvent;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;

use crate::id::Id;
use crate::mode::AppMode;
use crate::msg::Msg;

/// A human-readable description of a single key binding.
#[derive(Debug)]
#[non_exhaustive]
pub struct KeyBinding {
    /// The key or key combination, e.g. `"j / ↓"`.
    pub key: String,
    /// What the key does, e.g. `"Move down"`.
    pub action: String,
    /// The mode in which this binding is active.
    pub mode: AppMode,
}

/// A top-level view that owns a set of tui-realm components.
///
/// Each tab (Accounts, Budget, Reports) implements this trait in its own
/// module. The application router calls [`Screen::mount`] when switching to
/// a tab and [`Screen::unmount`] when leaving it, keeping component sets
/// isolated.
pub trait Screen {
    /// Mount this screen's components into the application.
    ///
    /// Called when the user switches to this tab. Should load initial data
    /// and register all components via `app.mount(...)`.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to mount (e.g., duplicate ID).
    fn mount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) -> anyhow::Result<()>;

    /// Unmount this screen's components from the application.
    ///
    /// Called when the user switches away from this tab.
    fn unmount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>);

    /// Render this screen's components into the given area.
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect);

    /// Handle a message from the tui-realm event loop.
    ///
    /// `Model::update()` calls this for every message not handled at the
    /// cross-cutting level. Screens should match only on their own [`Msg`]
    /// variants and return `None` for anything unrecognised.
    ///
    /// May return a follow-up [`Msg`] for chained processing.
    fn handle(&mut self, msg: Msg) -> Option<Msg>;

    /// The component that should receive focus when this screen is first shown.
    fn initial_focus(&self) -> Id;

    /// Keybindings to display in the help overlay for the given mode.
    fn keybindings(&self, mode: &AppMode) -> Vec<KeyBinding>;
}

/// Construct the [`Screen`] implementation for the given [`Tab`].
///
/// Called by [`crate::app::Model::switch_tab`] to create a new screen
/// when the user switches tabs.
#[inline]
#[must_use]
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as screen::make_screen; repetition is intentional"
)]
pub fn make_screen(
    tab: &crate::msg::Tab,
    ctx: std::sync::Arc<crate::context::TuiContext>,
) -> Box<dyn Screen> {
    match tab {
        crate::msg::Tab::Accounts => Box::new(accounts::AccountsScreen::new(ctx)),
        crate::msg::Tab::Budget => Box::new(budget::BudgetScreen::new(ctx)),
        crate::msg::Tab::Reports => Box::new(reports::ReportsScreen::new(ctx)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::AccountsId;
    use crate::msg::AccountsMsg;

    /// A minimal Screen implementation used to verify the trait compiles
    /// and is object-safe (i.e., `Box<dyn Screen>` is valid).
    struct StubScreen;

    impl Screen for StubScreen {
        #[inline]
        fn mount(&mut self, _app: &mut Application<Id, Msg, NoUserEvent>) -> anyhow::Result<()> {
            Ok(())
        }

        #[inline]
        fn unmount(&mut self, _app: &mut Application<Id, Msg, NoUserEvent>) {}

        #[inline]
        fn view(
            &mut self,
            _app: &mut Application<Id, Msg, NoUserEvent>,
            _frame: &mut Frame,
            _area: Rect,
        ) {
        }

        #[inline]
        fn handle(&mut self, _msg: Msg) -> Option<Msg> {
            None
        }

        #[inline]
        fn initial_focus(&self) -> Id {
            Id::Accounts(AccountsId::Sidebar)
        }

        #[inline]
        fn keybindings(&self, _mode: &AppMode) -> Vec<KeyBinding> {
            vec![]
        }
    }

    #[test]
    fn screen_is_object_safe() {
        // Verifies Box<dyn Screen> compiles — object safety is a compile-time check.
        let _screen: Box<dyn Screen> = Box::new(StubScreen);
    }

    #[test]
    fn stub_handle_returns_none_for_all_messages() {
        let mut screen = StubScreen;
        pretty_assertions::assert_eq!(screen.handle(Msg::AppQuit), None);
        pretty_assertions::assert_eq!(
            screen.handle(Msg::Accounts(AccountsMsg::FormCancelled)),
            None,
        );
    }
}
