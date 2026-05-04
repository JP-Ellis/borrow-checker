//! Transaction list component.
//!
//! Renders a scrollable table of transactions for the selected account.
//! Columns: Date | Payee | Amount | Balance.
//! Amounts use the selected account's posting sign; balance runs newest-to-oldest.

use bc_models::AccountId;
use bc_models::Decimal;
use bc_models::Transaction;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::command::Direction;
use tuirealm::component::AppComponent;
use tuirealm::component::Component;
use tuirealm::event::Event;
use tuirealm::event::Key;
use tuirealm::event::KeyEvent;
use tuirealm::event::NoUserEvent;
use tuirealm::props::AttrValue;
use tuirealm::props::Attribute;
use tuirealm::props::Props;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::text::Span;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::List;
use tuirealm::ratatui::widgets::ListItem;
use tuirealm::ratatui::widgets::ListState;
use tuirealm::state::State;
use tuirealm::state::StateValue;

use crate::mode::AppMode;
use crate::msg::AccountsMsg;
use crate::msg::Msg;

// MARK: private component

/// Raw widget that renders the transaction table.
struct TxList {
    /// Component props storage.
    props: Props,
    /// Transactions to display (sorted newest-first).
    transactions: Vec<Transaction>,
    /// Pre-computed running balances, parallel to `transactions`.
    running_balances: Vec<Decimal>,
    /// The account currently being viewed (used to find the matching posting).
    account_id: Option<AccountId>,
    /// Commodity of the running balance (e.g. `"AUD"`).
    commodity: String,
    /// Index of the currently highlighted row.
    selected: usize,
}

impl TxList {
    /// Build an empty `TxList` with no account context.
    #[cfg(test)]
    fn empty() -> Self {
        Self {
            props: Props::default(),
            transactions: Vec::new(),
            running_balances: Vec::new(),
            account_id: None,
            commodity: String::new(),
            selected: 0,
        }
    }

    /// Move the selection down by one row, clamping at the last item.
    fn move_down(&mut self) {
        let last = self.transactions.len().saturating_sub(1);
        self.selected = self.selected.saturating_add(1).min(last);
    }

    /// Move the selection up by one row, clamping at zero.
    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Net posting amount for `account_id` in `commodity` within `tx`.
    ///
    /// Sums all postings whose account and commodity match. Returns zero if
    /// none match (should not happen for well-formed transactions).
    ///
    /// # Arguments
    ///
    /// * `tx`         - The transaction to inspect.
    /// * `account_id` - Account whose postings are summed.
    /// * `commodity`  - Commodity to filter by.
    ///
    /// # Returns
    ///
    /// Net signed amount for the account in this transaction.
    fn posting_net(tx: &Transaction, account_id: &AccountId, commodity: &str) -> Decimal {
        tx.postings()
            .iter()
            .filter(|p| {
                p.account_id() == account_id && p.amount().commodity().as_str() == commodity
            })
            .fold(Decimal::ZERO, |acc, p| {
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "summing Decimal posting amounts; no overflow risk for financial amounts"
                )]
                { acc + p.amount().value() }
            })
    }

    /// Compute a running balance array for `transactions` (newest-first).
    ///
    /// `running_balances[0]` is `current_balance` (after the most recent transaction).
    /// Each subsequent entry is the balance before the transaction above it.
    ///
    /// # Arguments
    ///
    /// * `transactions`    - Slice sorted newest-first.
    /// * `account_id`      - Account whose postings are summed.
    /// * `commodity`       - Commodity to filter by.
    /// * `current_balance` - Current (latest) balance for the account.
    ///
    /// # Returns
    ///
    /// A `Vec<Decimal>` parallel to `transactions`, or empty if `transactions` is empty.
    fn compute_running_balances(
        transactions: &[Transaction],
        account_id: &AccountId,
        commodity: &str,
        current_balance: Decimal,
    ) -> Vec<Decimal> {
        if transactions.is_empty() {
            return Vec::new();
        }
        let mut balances = vec![Decimal::ZERO; transactions.len()];
        #[expect(
            clippy::indexing_slicing,
            reason = "index 0 is within bounds: transactions is non-empty (checked above)"
        )]
        {
            balances[0] = current_balance;
        }
        for i in 1..transactions.len() {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "i >= 1 so i - 1 cannot underflow; balance subtraction is safe for financial amounts"
            )]
            #[expect(
                clippy::indexing_slicing,
                reason = "i and i-1 are within bounds: loop runs from 1..len"
            )]
            {
                let net = Self::posting_net(&transactions[i - 1], account_id, commodity);
                balances[i] = balances[i - 1] - net;
            }
        }
        balances
    }

    /// Returns the ratatui color for a signed decimal value.
    ///
    /// # Returns
    ///
    /// [`Color::Green`] for positive, [`Color::Red`] for negative, [`Color::Reset`] for zero.
    fn color_for(value: Decimal) -> Color {
        match value.cmp(&Decimal::ZERO) {
            core::cmp::Ordering::Greater => Color::Green,
            core::cmp::Ordering::Less => Color::Red,
            core::cmp::Ordering::Equal => Color::Reset,
        }
    }

    /// Truncate `s` to at most `max` chars, appending `…` if truncated.
    fn truncate(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            s.to_owned()
        } else {
            let t: String = s.chars().take(max.saturating_sub(1)).collect();
            format!("{t}\u{2026}")
        }
    }

    /// Format a decimal amount with 2 decimal places and an optional commodity suffix.
    ///
    /// # Arguments
    ///
    /// * `value`     - The amount to format.
    /// * `commodity` - Commodity suffix (e.g. `"AUD"`); empty string produces no suffix.
    ///
    /// # Returns
    ///
    /// A string like `"42.50 AUD"` or `"42.50"`.
    fn format_amount(value: Decimal, commodity: &str) -> String {
        if commodity.is_empty() {
            format!("{value:.2}")
        } else {
            format!("{value:.2} {commodity}")
        }
    }
}

impl Component for TxList {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .cloned()
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
                let payee = Self::truncate(t.payee().unwrap_or("\u{2014}"), 24);
                let date_str = format!("{:<10}", t.date());

                let (amount_val, amount_str) = if let Some(ref acc_id) = self.account_id {
                    let val = Self::posting_net(t, acc_id, &self.commodity);
                    (val, Self::format_amount(val, &self.commodity))
                } else {
                    (Decimal::ZERO, "\u{2014}".to_owned())
                };

                let balance_val = self
                    .running_balances
                    .get(idx)
                    .copied()
                    .unwrap_or(Decimal::ZERO);
                let balance_str = Self::format_amount(balance_val, &self.commodity);

                let base_style = if idx == self.selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };

                let line = Line::from(vec![
                    Span::styled(format!("{date_str}  {payee:<24}  "), base_style),
                    Span::styled(
                        format!("{amount_str:>14}  "),
                        Style::default().fg(Self::color_for(amount_val)),
                    ),
                    Span::styled(
                        format!("{balance_str:>14}"),
                        Style::default().fg(Self::color_for(balance_val)),
                    ),
                ]);

                ListItem::new(line)
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
    fn query(&self, attr: Attribute) -> Option<tuirealm::props::QueryResult<'_>> {
        self.props.get_for_query(attr)
    }

    #[inline]
    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    #[inline]
    fn state(&self) -> State {
        match self.transactions.get(self.selected) {
            Some(t) => State::Single(StateValue::String(t.id().to_string())),
            None => State::None,
        }
    }

    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Cmd is non-exhaustive; all other variants return CmdResult::NoChange"
    )]
    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                self.move_down();
            }
            Cmd::Move(Direction::Up) => {
                self.move_up();
            }
            _ => return CmdResult::NoChange,
        }
        CmdResult::Changed(self.state())
    }
}

// MARK: public wrapper

/// Tui-realm component wrapper for the transaction list.
///
/// Renders a scrollable table with columns: Date | Payee | Amount | Balance.
/// Amounts show the sign of the selected account's posting (outflows negative).
/// `j`/`k` navigation emits [`crate::msg::ChromeMsg::Redraw`].
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as list::TransactionList; repetition is intentional"
)]
#[non_exhaustive]
#[derive(Component)]
pub struct TransactionList {
    /// Inner raw widget.
    component: TxList,
}

impl TransactionList {
    /// Create a new `TransactionList`.
    ///
    /// # Arguments
    ///
    /// * `transactions`    - Rows to display, sorted newest-first.
    /// * `account_id`      - The account being viewed (selects the matching posting).
    /// * `current_balance` - Current balance used to compute the running-balance column.
    /// * `commodity`       - Commodity code for balances (e.g. `"AUD"`).
    ///
    /// # Returns
    ///
    /// A new `TransactionList` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(
        transactions: Vec<Transaction>,
        account_id: Option<AccountId>,
        current_balance: Decimal,
        commodity: String,
    ) -> Self {
        let running_balances = account_id.as_ref().map_or_else(Vec::new, |id| {
            TxList::compute_running_balances(&transactions, id, &commodity, current_balance)
        });
        Self {
            component: TxList {
                props: Props::default(),
                transactions,
                running_balances,
                account_id,
                commodity,
                selected: 0,
            },
        }
    }
}

impl AppComponent<Msg, NoUserEvent> for TransactionList {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; remaining variants all produce None"
    )]
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Down | Key::Char('j'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Down));
                Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Up | Key::Char('k'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Up));
                Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw))
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
            }) => self
                .component
                .transactions
                .get(self.component.selected)
                .map(|tx| Msg::Accounts(AccountsMsg::OpenEditTransaction(tx.id().clone()))),
            Event::Keyboard(KeyEvent {
                code: Key::Char('d'),
                ..
            }) => self
                .component
                .transactions
                .get(self.component.selected)
                .map(|tx| Msg::Accounts(AccountsMsg::VoidRequested(tx.id().clone()))),
            Event::Keyboard(KeyEvent {
                code: Key::Char('v'),
                ..
            }) => Some(Msg::ModeChange(AppMode::Visual)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use bc_models::TransactionStatus;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use tuirealm::event::Key;
    use tuirealm::event::KeyEvent;
    use tuirealm::event::KeyModifiers;

    use super::*;
    use crate::msg::ChromeMsg;
    use crate::msg::Msg;

    fn make_posting(account_id: &AccountId, value: i32, commodity: &str) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account_id.clone())
            .amount(Amount::new(
                Decimal::from(value),
                CommodityCode::new(commodity),
            ))
            .build()
    }

    fn make_tx(
        date: jiff::civil::Date,
        account_id: &AccountId,
        value: i32,
        commodity: &str,
        other_id: &AccountId,
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date)
            .description("test")
            .status(TransactionStatus::Cleared)
            .postings(vec![
                make_posting(account_id, value, commodity),
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "negating a small test i32; overflow is not possible in test context"
                )]
                make_posting(other_id, -value, commodity),
            ])
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn posting_net_sums_matching_postings() {
        let acc = AccountId::new();
        let other = AccountId::new();
        let tx = make_tx(date(2026, 5, 1), &acc, 100, "AUD", &other);
        let net = TxList::posting_net(&tx, &acc, "AUD");
        assert_eq!(net, Decimal::from(100_i32));
    }

    #[test]
    fn posting_net_returns_zero_for_no_match() {
        let acc = AccountId::new();
        let other = AccountId::new();
        let unrelated = AccountId::new();
        let tx = make_tx(date(2026, 5, 1), &acc, 100, "AUD", &other);
        let net = TxList::posting_net(&tx, &unrelated, "AUD");
        assert_eq!(net, Decimal::ZERO);
    }

    #[test]
    fn compute_running_balances_newest_first() {
        let acc = AccountId::new();
        let other = AccountId::new();
        // Two transactions: newest first
        // tx0 is most recent (+50), tx1 is older (+100)
        // current_balance = 150 (100 + 50)
        let tx0 = make_tx(date(2026, 5, 1), &acc, 50, "AUD", &other);
        let tx1 = make_tx(date(2026, 4, 1), &acc, 100, "AUD", &other);
        let transactions = vec![tx0, tx1];
        let current_balance = Decimal::from(150_i32);
        let balances =
            TxList::compute_running_balances(&transactions, &acc, "AUD", current_balance);
        #[expect(
            clippy::indexing_slicing,
            reason = "we just created a 2-element Vec above; indices 0 and 1 are in bounds"
        )]
        {
            assert_eq!(balances[0], Decimal::from(150_i32)); // after most recent
            assert_eq!(balances[1], Decimal::from(100_i32)); // before most recent = 150 - 50
        }
    }

    #[test]
    fn compute_running_balances_empty() {
        let acc = AccountId::new();
        let balances = TxList::compute_running_balances(&[], &acc, "AUD", Decimal::from(100_i32));
        assert!(balances.is_empty());
    }

    #[test]
    fn color_for_positive_is_green() {
        assert_eq!(TxList::color_for(Decimal::from(1_i32)), Color::Green);
    }

    #[test]
    fn color_for_negative_is_red() {
        assert_eq!(TxList::color_for(Decimal::from(-1_i32)), Color::Red);
    }

    #[test]
    fn color_for_zero_is_reset() {
        assert_eq!(TxList::color_for(Decimal::ZERO), Color::Reset);
    }

    #[test]
    fn move_down_on_empty_list_does_not_panic() {
        let mut list = TxList::empty();
        list.move_down();
        assert_eq!(list.selected, 0);
    }

    #[test]
    fn move_up_does_not_underflow() {
        let mut list = TxList::empty();
        list.move_up();
        assert_eq!(list.selected, 0);
    }

    #[test]
    fn move_down_clamps_to_last() {
        let acc = AccountId::new();
        let other = AccountId::new();
        let tx0 = make_tx(date(2026, 5, 1), &acc, 50, "AUD", &other);
        let tx1 = make_tx(date(2026, 4, 1), &acc, 100, "AUD", &other);
        let mut list = TxList::empty();
        list.transactions = vec![tx0, tx1];

        for _ in 0_usize..10_usize {
            list.move_down();
        }
        assert_eq!(list.selected, 1);
    }

    #[test]
    fn j_key_emits_redraw() {
        let mut list = TransactionList::new(vec![], None, Decimal::ZERO, String::new());
        let result = list.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('j'),
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(ChromeMsg::Redraw)));
    }

    #[test]
    fn k_key_emits_redraw() {
        let mut list = TransactionList::new(vec![], None, Decimal::ZERO, String::new());
        let result = list.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('k'),
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(ChromeMsg::Redraw)));
    }
}
