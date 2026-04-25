//! Transaction detail panel component.
//!
//! Displays full details of a single selected transaction (read-only),
//! or a placeholder when no transaction is selected.

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
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;

use crate::msg::Msg;

// ─── private component ───────────────────────────────────────────────────────

/// Raw widget that renders the transaction detail panel.
struct TxDetail {
    /// Component props storage.
    props: Props,
    /// Transaction to display, if any.
    transaction: Option<Transaction>,
}

impl TxDetail {
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
                        lines.push(format!("  {}  {}", posting.account_id(), amount_str));

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

// ─── public wrapper ──────────────────────────────────────────────────────────

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

    /// Update the displayed transaction.
    #[inline]
    pub fn set_transaction(&mut self, transaction: Option<Transaction>) {
        self.component.transaction = transaction;
    }
}

impl Component<Msg, NoUserEvent> for TransactionDetail {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn render_content_with_no_transaction_returns_placeholder() {
        let detail = TxDetail {
            props: Props::default(),
            transaction: None,
        };

        let content = detail.render_content();
        assert_eq!(content, "Select a transaction to see details.");
    }
}
