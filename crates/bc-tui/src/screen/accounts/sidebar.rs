//! Account tree sidebar component (stub — implemented in Task 3).

use bc_models::Account;
use tuirealm::AttrValue;
use tuirealm::Attribute;
use tuirealm::Component;
use tuirealm::Frame;
use tuirealm::MockComponent;
use tuirealm::NoUserEvent;
use tuirealm::Props;
use tuirealm::State;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::event::Event;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::widgets::Paragraph;

use crate::msg::Msg;

/// Raw widget that renders the account sidebar.
struct Sidebar {
    /// Component props storage.
    props: Props,
    /// Accounts to display.
    accounts: Vec<Account>,
}

impl MockComponent for Sidebar {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new("Accounts (stub)"), area);
    }

    #[inline]
    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    #[inline]
    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    #[inline]
    fn state(&self) -> State {
        State::None
    }

    #[inline]
    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::None
    }
}

/// Tui-realm component wrapper for the account sidebar widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as sidebar::AccountSidebar; repetition is intentional"
)]
#[derive(MockComponent)]
pub struct AccountSidebar {
    /// Inner raw widget.
    component: Sidebar,
}

impl AccountSidebar {
    /// Create a new `AccountSidebar` with the given list of accounts.
    #[inline]
    #[must_use]
    pub fn new(accounts: Vec<Account>) -> Self {
        Self {
            component: Sidebar {
                props: Props::default(),
                accounts,
            },
        }
    }
}

impl Component<Msg, NoUserEvent> for AccountSidebar {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
