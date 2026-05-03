//! Transaction detail panel component.
//!
//! Displays full details of a single selected transaction (read-only),
//! or a placeholder when no transaction is selected.

use bc_models::Account;
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
use tuirealm::event::Key;
use tuirealm::event::KeyEvent;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;

use crate::msg::AccountsMsg;
use crate::msg::Msg;

// MARK: private component

/// Raw widget that renders the transaction detail panel.
struct TxDetail {
    /// Component props storage.
    props: Props,
    /// Transaction to display, if any.
    transaction: Option<Transaction>,
    /// All accounts — used to resolve posting account IDs to human-readable names.
    accounts: Vec<Account>,
}

impl TxDetail {
    /// Resolve an account ID to a display name, falling back to the raw ID string.
    ///
    /// # Arguments
    ///
    /// * `id`       - The account ID to look up.
    /// * `accounts` - The flat list of all accounts.
    ///
    /// # Returns
    ///
    /// The account's `name` if found; otherwise the ID formatted as a string.
    #[inline]
    fn account_name(id: &bc_models::AccountId, accounts: &[Account]) -> String {
        accounts
            .iter()
            .find(|a| a.id() == id)
            .map_or_else(|| id.to_string(), |a| a.name().to_owned())
    }

    /// Format a transaction as a multi-line string.
    ///
    /// If no transaction is set, returns the placeholder text.
    /// Otherwise formats: Date, Payee, Description (if non-empty),
    /// Status, and all postings with their accounts, amounts, and envelope.
    #[inline]
    fn render_content(&self) -> String {
        match &self.transaction {
            None => "Select a transaction to see details.".to_owned(),
            Some(tx) => {
                let mut lines = Vec::new();

                // Date
                lines.push(format!("Date:    {}", tx.date()));

                // Payee (optional)
                if let Some(payee) = tx.payee() {
                    lines.push(format!("Payee:   {payee}"));
                }

                // Description (only if non-empty)
                let desc = tx.description();
                if !desc.is_empty() {
                    lines.push(format!("Desc:    {desc}"));
                }

                // Status
                lines.push(format!("Status:  {:?}", tx.status()));

                // Postings
                if !tx.postings().is_empty() {
                    lines.push(String::new());
                    lines.push("Postings:".to_owned());
                    for posting in tx.postings() {
                        let amount_str = format!(
                            "{} {}",
                            posting.amount().value(),
                            posting.amount().commodity()
                        );
                        lines.push(format!(
                            "  {}  {}",
                            Self::account_name(posting.account_id(), &self.accounts),
                            amount_str
                        ));

                        // Envelope (optional)
                        if let Some(env_id) = posting.envelope_id() {
                            lines.push(format!("    envelope: {env_id}"));
                        }
                    }
                }

                lines.join("\n")
            }
        }
    }
}

impl MockComponent for TxDetail {
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
        let content = self.render_content();
        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Detail ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color)),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
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

// MARK: public wrapper

/// Tui-realm component wrapper for the transaction detail panel widget.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as detail::TransactionDetail; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct TransactionDetail {
    /// Inner raw widget.
    component: TxDetail,
}

impl TransactionDetail {
    /// Create a new `TransactionDetail` showing the given transaction, or empty if `None`.
    ///
    /// # Arguments
    ///
    /// * `transaction` - The transaction to display, or `None` for an empty panel.
    /// * `accounts`    - All accounts, used to resolve posting account IDs to names.
    ///
    /// # Returns
    ///
    /// A new `TransactionDetail` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(transaction: Option<Transaction>, accounts: Vec<Account>) -> Self {
        Self {
            component: TxDetail {
                props: Props::default(),
                transaction,
                accounts,
            },
        }
    }

    /// Update the displayed transaction.
    #[inline]
    pub fn set_transaction(&mut self, transaction: Option<Transaction>) {
        self.component.transaction = transaction;
    }
}

impl Component<Msg, NoUserEvent> for TransactionDetail {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; remaining variants all produce None"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Esc | Key::Left | Key::Char('h'),
                ..
            }) => Some(Msg::Accounts(AccountsMsg::CloseDetail)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Account;
    use bc_models::AccountId;
    use bc_models::AccountType;
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

    use super::*;

    #[test]
    fn render_content_with_no_transaction_returns_placeholder() {
        let detail = TxDetail {
            props: Props::default(),
            transaction: None,
            accounts: vec![],
        };
        assert_eq!(
            detail.render_content(),
            "Select a transaction to see details."
        );
    }

    #[test]
    fn render_content_resolves_account_name() {
        let acc_id = AccountId::new();
        let account = Account::builder()
            .name("Checking")
            .account_type(AccountType::Asset)
            .id(acc_id.clone())
            .build();

        let posting = Posting::builder()
            .id(PostingId::new())
            .account_id(acc_id)
            .amount(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .build();

        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 5, 1))
            .description("Test")
            .status(TransactionStatus::Cleared)
            .postings(vec![posting])
            .created_at(Timestamp::now())
            .build();

        let detail = TxDetail {
            props: Props::default(),
            transaction: Some(tx),
            accounts: vec![account],
        };

        let content = detail.render_content();
        assert!(
            content.contains("  Checking  100 AUD"),
            "expected posting line '  Checking  100 AUD' in: {content}"
        );
    }
}
