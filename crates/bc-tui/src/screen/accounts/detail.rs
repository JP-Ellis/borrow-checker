//! Transaction detail panel component (stub — implemented in Task 5).

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

/// Raw widget that renders the transaction detail panel.
struct TxDetail {
    /// Component props storage.
    props: Props,
    /// Transaction to display, if any.
    transaction: Option<Transaction>,
}

impl MockComponent for TxDetail {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Paragraph::new("Detail (stub)"), area);
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

/// Tui-realm component wrapper for the transaction detail panel widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as detail::TransactionDetail; repetition is intentional"
)]
#[derive(MockComponent)]
pub struct TransactionDetail {
    /// Inner raw widget.
    component: TxDetail,
}

impl TransactionDetail {
    /// Create a new `TransactionDetail` showing the given transaction, or empty if `None`.
    #[inline]
    #[must_use]
    pub fn new(transaction: Option<Transaction>) -> Self {
        Self {
            component: TxDetail {
                props: Props::default(),
                transaction,
            },
        }
    }
}

impl Component<Msg, NoUserEvent> for TransactionDetail {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
