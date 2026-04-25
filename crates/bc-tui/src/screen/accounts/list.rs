//! Transaction list component.
//!
//! Renders a scrollable list of transactions for the currently selected account.
//! Navigation emits nothing; action keys emit [`crate::msg::AccountsMsg`] variants.

use bc_models::Transaction;
use tuirealm::AttrValue;
use tuirealm::Attribute;
use tuirealm::Component;
use tuirealm::Frame;
use tuirealm::MockComponent;
use tuirealm::NoUserEvent;
use tuirealm::Props;
use tuirealm::State;
use tuirealm::StateValue;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::command::Direction;
use tuirealm::event::Event;
use tuirealm::event::Key;
use tuirealm::event::KeyEvent;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::List;
use tuirealm::ratatui::widgets::ListItem;
use tuirealm::ratatui::widgets::ListState;

use crate::mode::AppMode;
use crate::msg::AccountsMsg;
use crate::msg::Msg;

// ─── private component ───────────────────────────────────────────────────────

/// Raw widget that renders the transaction list.
struct TxList {
    /// Component props storage.
    props: Props,
    /// Transactions to display.
    transactions: Vec<Transaction>,
    /// Index of the currently highlighted row.
    selected: usize,
}

impl TxList {
    /// Move the selection down by one row, clamping at the last item.
    fn move_down(&mut self) {
        let last = self.transactions.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(1).min(last);
    }

    /// Move the selection up by one row, clamping at zero.
    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Replace the displayed transactions and reset selection to zero.
    fn set_transactions(&mut self, transactions: Vec<Transaction>) {
        self.transactions = transactions;
        self.selected = 0;
    }

    /// Compute a display string for the net positive amount of a transaction.
    ///
    /// Sums the absolute values of all posting amounts and formats the first
    /// commodity found, or returns `"—"` when there are no postings.
    ///
    /// # Arguments
    ///
    /// * `t` - The transaction whose postings will be summed.
    ///
    /// # Returns
    ///
    /// A formatted string such as `"42.50 AUD"`.
    fn format_amount(t: &Transaction) -> String {
        // Find the first positive posting amount to represent the transaction value.
        t.postings()
            .iter()
            .find(|p| p.amount().value() > bc_models::Decimal::ZERO)
            .map_or_else(
                || "\u{2014}".to_owned(), // —
                |p| format!("{} {}", p.amount().value(), p.amount().commodity()),
            )
    }
}

impl MockComponent for TxList {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .and_then(|v| {
                if let AttrValue::Flag(b) = v {
                    Some(b)
                } else {
                    None
                }
            })
            .unwrap_or(false);

        let border_color = if focused { Color::Cyan } else { Color::White };

        let block = Block::default()
            .title(" Transactions ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));

        let items: Vec<ListItem<'_>> = self
            .transactions
            .iter()
            .enumerate()
            .map(|(idx, t)| {
                let payee = t.payee().unwrap_or("\u{2014}"); // —
                let amount = Self::format_amount(t);
                let text = format!("{:<12}  {:<20}  {}", t.date(), payee, amount);
                let style = if idx == self.selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                ListItem::new(text).style(style)
            })
            .collect();

        let list = List::new(items).block(block);

        let mut list_state = ListState::default();
        list_state.select(if self.transactions.is_empty() {
            None
        } else {
            Some(self.selected)
        });
        frame.render_stateful_widget(list, area, &mut list_state);
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
        match self.transactions.get(self.selected) {
            Some(t) => State::One(StateValue::String(t.id().to_string())),
            None => State::None,
        }
    }

    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Cmd is non-exhaustive; all other variants return CmdResult::None"
    )]
    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                self.move_down();
            }
            Cmd::Move(Direction::Up) => {
                self.move_up();
            }
            _ => return CmdResult::None,
        }
        CmdResult::Changed(self.state())
    }
}

// ─── public wrapper ──────────────────────────────────────────────────────────

/// Tui-realm component wrapper for the transaction list widget.
///
/// Renders a scrollable list of transactions with `j`/`k` (or Up/Down) navigation.
/// Pressing `a`, `e`, or `d` emits the corresponding [`AccountsMsg`] variant.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as list::TransactionList; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct TransactionList {
    /// Inner raw widget.
    component: TxList,
}

impl TransactionList {
    /// Create a new `TransactionList` with the given transactions.
    ///
    /// # Arguments
    ///
    /// * `transactions` - The transactions to display, in the order they will be shown.
    ///
    /// # Returns
    ///
    /// A new `TransactionList` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self {
            component: TxList {
                props: Props::default(),
                transactions,
                selected: 0,
            },
        }
    }

    /// Replace the displayed transactions and reset the selection to the first item.
    #[inline]
    pub fn set_transactions(&mut self, transactions: Vec<Transaction>) {
        self.component.set_transactions(transactions);
    }
}

impl Component<Msg, NoUserEvent> for TransactionList {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; remaining variants all produce None"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Down | Key::Char('j'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Down));
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Up | Key::Char('k'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Up));
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Right | Key::Char('l') | Key::Enter,
                ..
            }) => self
                .component
                .transactions
                .get(self.component.selected)
                .map(|tx| Msg::Accounts(AccountsMsg::OpenDetail(tx.id().clone()))),
            Event::Keyboard(KeyEvent {
                code: Key::Left | Key::Char('h'),
                ..
            }) => Some(Msg::Accounts(AccountsMsg::FocusSidebar)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('a'),
                ..
            }) => Some(Msg::Accounts(AccountsMsg::OpenAddTransaction)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('e'),
                ..
            }) => Some(Msg::Accounts(AccountsMsg::OpenEditTransaction)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('d'),
                ..
            }) => Some(Msg::Accounts(AccountsMsg::VoidRequested)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('v'),
                ..
            }) => Some(Msg::ModeChange(AppMode::Visual)),
            _ => None,
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn move_down_on_empty_list_does_not_panic() {
        let mut list = TxList {
            props: Props::default(),
            transactions: vec![],
            selected: 0,
        };
        list.move_down();
        assert_eq!(list.selected, 0);
    }

    #[test]
    fn move_up_does_not_underflow() {
        let mut list = TxList {
            props: Props::default(),
            transactions: vec![],
            selected: 0,
        };
        list.move_up();
        assert_eq!(list.selected, 0);
    }

    #[test]
    fn move_down_clamps_to_last() {
        use bc_models::TransactionId;
        use jiff::Timestamp;
        use jiff::civil::date;

        let make_tx = || {
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 1))
                .description("test")
                .status(bc_models::TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .build()
        };

        let mut list = TxList {
            props: Props::default(),
            transactions: vec![make_tx(), make_tx()],
            selected: 0,
        };

        for _ in 0_usize..10_usize {
            list.move_down();
        }

        assert_eq!(list.selected, 1);
    }
}
