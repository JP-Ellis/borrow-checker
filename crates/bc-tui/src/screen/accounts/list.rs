//! Transaction list component (stub — implemented in Task 4).

use bc_models::Transaction;
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

/// Raw widget that renders the transaction list.
struct TxList {
    /// Component props storage.
    props: Props,
    /// Transactions to display.
    transactions: Vec<Transaction>,
}

impl MockComponent for TxList {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new("Transactions (stub)"), area);
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

/// Tui-realm component wrapper for the transaction list widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as list::TransactionList; repetition is intentional"
)]
#[derive(MockComponent)]
pub struct TransactionList {
    /// Inner raw widget.
    component: TxList,
}

impl TransactionList {
    /// Create a new `TransactionList` with the given transactions.
    #[inline]
    #[must_use]
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self {
            component: TxList {
                props: Props::default(),
                transactions,
            },
        }
    }
}

impl Component<Msg, NoUserEvent> for TransactionList {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
