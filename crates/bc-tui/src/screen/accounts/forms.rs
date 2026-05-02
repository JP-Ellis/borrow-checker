//! Transaction add/edit form overlay.
//!
//! Renders a centred floating popup with four fields (Date, Payee, Amount,
//! Counterpart Account).  Tab / Shift-Tab cycle through fields; Enter submits;
//! Esc cancels.

use bc_models;
use tui_input::Input;
use tui_input::InputRequest;
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
use tuirealm::ratatui::layout::Constraint;
use tuirealm::ratatui::layout::Direction;
use tuirealm::ratatui::layout::Layout;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Clear;
use tuirealm::ratatui::widgets::Paragraph;

use crate::msg::AccountsMsg;
use crate::msg::Msg;

// MARK: field enum

/// Which form field currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    /// Transaction date (YYYY-MM-DD).
    Date,
    /// Payee name.
    Payee,
    /// Amount (free-form text).
    Amount,
    /// Counterpart account name.
    Account,
}

impl FormField {
    /// Return the next field in tab order, wrapping from `Account` to `Date`.
    fn next(self) -> Self {
        match self {
            Self::Date => Self::Payee,
            Self::Payee => Self::Amount,
            Self::Amount => Self::Account,
            Self::Account => Self::Date,
        }
    }

    /// Return the previous field in tab order, wrapping from `Date` to `Account`.
    fn prev(self) -> Self {
        match self {
            Self::Date => Self::Account,
            Self::Payee => Self::Date,
            Self::Amount => Self::Payee,
            Self::Account => Self::Amount,
        }
    }

    /// Human-readable label shown as the field border title.
    fn label(self) -> &'static str {
        match self {
            Self::Date => "Date (YYYY-MM-DD)",
            Self::Payee => "Payee",
            Self::Amount => "Amount",
            Self::Account => "Counterpart Account",
        }
    }
}

// MARK: private component

/// Raw widget that renders the transaction add/edit form.
struct TxForm {
    /// Component props storage.
    props: Props,
    /// Which field currently has keyboard focus.
    focused_field: FormField,
    /// Date field input buffer.
    date: Input,
    /// Payee field input buffer.
    payee: Input,
    /// Amount field input buffer.
    amount: Input,
    /// Counterpart account field input buffer.
    account: Input,
}

impl TxForm {
    /// Create a new empty "add transaction" form starting at the Date field.
    fn new_add() -> Self {
        Self {
            props: Props::default(),
            focused_field: FormField::Date,
            date: Input::default(),
            payee: Input::default(),
            amount: Input::default(),
            account: Input::default(),
        }
    }

    /// Create an "edit transaction" form pre-populated from an existing transaction.
    ///
    /// # Arguments
    ///
    /// * `tx` - The transaction to edit; date, payee, and the first posting's
    ///   amount and account are used to pre-populate the fields.
    fn new_edit(tx: &bc_models::Transaction) -> Self {
        let date_val = tx.date().to_string();
        let payee_val = tx.payee().unwrap_or("").to_owned();
        let (amount_val, account_val) = tx
            .postings()
            .first()
            .map(|p| {
                (
                    format!("{} {}", p.amount().value(), p.amount().commodity()),
                    p.account_id().to_string(),
                )
            })
            .unwrap_or_default();

        Self {
            props: Props::default(),
            focused_field: FormField::Date,
            date: Input::new(date_val),
            payee: Input::new(payee_val),
            amount: Input::new(amount_val),
            account: Input::new(account_val),
        }
    }

    /// Return a mutable reference to the currently focused input buffer.
    fn active_input_mut(&mut self) -> &mut Input {
        match self.focused_field {
            FormField::Date => &mut self.date,
            FormField::Payee => &mut self.payee,
            FormField::Amount => &mut self.amount,
            FormField::Account => &mut self.account,
        }
    }

    /// Render a single labelled input field.
    ///
    /// Draws a bordered [`Paragraph`] whose title is `label` and content is
    /// the current value of `input`.  The border is yellow when `focused` is
    /// `true`, white otherwise.
    ///
    /// # Arguments
    ///
    /// * `frame`   - The ratatui frame to render into.
    /// * `area`    - The area to render the field within.
    /// * `label`   - Border title text.
    /// * `input`   - The [`Input`] buffer whose value will be displayed.
    /// * `focused` - Whether this field currently has keyboard focus.
    fn render_field(frame: &mut Frame, area: Rect, label: &str, input: &Input, focused: bool) {
        let border_color = if focused { Color::Yellow } else { Color::White };
        let block = Block::default()
            .title(format!(" {label} "))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        let paragraph = Paragraph::new(input.value()).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Compute a centred ~50% × ~60% popup [`Rect`] from `area`.
    ///
    /// # Arguments
    ///
    /// * `area` - The full terminal area to centre the popup within.
    ///
    /// # Returns
    ///
    /// A [`Rect`] representing the centred popup region.
    #[expect(
        clippy::indexing_slicing,
        reason = "layout always returns exactly 3 chunks matching the 3 constraints"
    )]
    fn popup_rect(area: Rect) -> Rect {
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(area);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ])
            .split(v[1])[1]
    }

    /// Split `popup` into 5 vertical rows and render all fields plus the hint.
    ///
    /// # Arguments
    ///
    /// * `frame`   - The ratatui frame to render into.
    /// * `popup`   - The popup [`Rect`] (output of [`Self::popup_rect`]).
    /// * `focused` - Which form field currently has focus.
    /// * `date`    - The date input buffer.
    /// * `payee`   - The payee input buffer.
    /// * `amount`  - The amount input buffer.
    /// * `account` - The counterpart account input buffer.
    #[expect(
        clippy::indexing_slicing,
        reason = "layout always returns exactly 5 chunks matching the 5 constraints"
    )]
    #[expect(
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 5 chunks matching the 5 constraints"
    )]
    fn render_fields(
        frame: &mut Frame,
        popup: Rect,
        focused: FormField,
        date: &Input,
        payee: &Input,
        amount: &Input,
        account: &Input,
    ) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(popup);

        Self::render_field(
            frame,
            rows[0],
            FormField::Date.label(),
            date,
            focused == FormField::Date,
        );
        Self::render_field(
            frame,
            rows[1],
            FormField::Payee.label(),
            payee,
            focused == FormField::Payee,
        );
        Self::render_field(
            frame,
            rows[2],
            FormField::Amount.label(),
            amount,
            focused == FormField::Amount,
        );
        Self::render_field(
            frame,
            rows[3],
            FormField::Account.label(),
            account,
            focused == FormField::Account,
        );

        let hint = Paragraph::new("Enter=Submit  Tab=Next field  Shift-Tab=Prev  Esc=Cancel");
        frame.render_widget(hint, rows[4]);
    }
}

impl MockComponent for TxForm {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let popup = Self::popup_rect(area);
        frame.render_widget(Clear, popup);
        Self::render_fields(
            frame,
            popup,
            self.focused_field,
            &self.date,
            &self.payee,
            &self.amount,
            &self.account,
        );
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

/// Tui-realm component wrapper for the transaction add/edit form overlay.
///
/// Displays a centred floating popup with four input fields.  Tab / Shift-Tab
/// cycle through them; Enter emits
/// [`AccountsMsg::FormSubmitted`](crate::msg::AccountsMsg::FormSubmitted);
/// Esc emits [`AccountsMsg::FormCancelled`](crate::msg::AccountsMsg::FormCancelled).
#[non_exhaustive]
#[derive(MockComponent)]
pub struct TransactionForm {
    /// Inner raw widget.
    component: TxForm,
}

impl TransactionForm {
    /// Create a new empty "add transaction" form.
    ///
    /// # Returns
    ///
    /// A [`TransactionForm`] ready to be mounted, starting on the Date field.
    #[inline]
    #[must_use]
    pub fn new_add() -> Self {
        Self {
            component: TxForm::new_add(),
        }
    }

    /// Create a pre-populated "edit transaction" form from an existing transaction.
    ///
    /// # Arguments
    ///
    /// * `tx` - The transaction to pre-populate the form from.
    ///
    /// # Returns
    ///
    /// A [`TransactionForm`] ready to be mounted, starting on the Date field.
    #[inline]
    #[must_use]
    pub fn new_edit(tx: &bc_models::Transaction) -> Self {
        Self {
            component: TxForm::new_edit(tx),
        }
    }
}

impl Component<Msg, NoUserEvent> for TransactionForm {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; all non-keyboard variants return None"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent { code: Key::Tab, .. }) => {
                self.component.focused_field = self.component.focused_field.next();
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::BackTab, ..
            }) => {
                self.component.focused_field = self.component.focused_field.prev();
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => Some(Msg::Accounts(AccountsMsg::FormSubmitted {
                date: self.component.date.value().to_owned(),
                payee: self.component.payee.value().to_owned(),
                amount: self.component.amount.value().to_owned(),
                account: self.component.account.value().to_owned(),
            })),
            Event::Keyboard(KeyEvent { code: Key::Esc, .. }) => {
                Some(Msg::Accounts(AccountsMsg::FormCancelled))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char(c), ..
            }) => {
                self.component
                    .active_input_mut()
                    .handle(InputRequest::InsertChar(c));
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Backspace,
                ..
            }) => {
                self.component
                    .active_input_mut()
                    .handle(InputRequest::DeletePrevChar);
                None
            }
            _ => None,
        }
    }
}

// MARK: tests

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tuirealm::event::KeyModifiers;

    use super::*;

    fn tab_event() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Tab,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn back_tab_event() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::BackTab,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn tab_cycles_through_fields() {
        let mut form = TransactionForm::new_add();

        // Start at Date.
        assert_eq!(form.component.focused_field, FormField::Date);

        form.on(tab_event());
        assert_eq!(form.component.focused_field, FormField::Payee);

        form.on(tab_event());
        assert_eq!(form.component.focused_field, FormField::Amount);

        form.on(tab_event());
        assert_eq!(form.component.focused_field, FormField::Account);

        // Wraps back to Date.
        form.on(tab_event());
        assert_eq!(form.component.focused_field, FormField::Date);
    }

    #[test]
    fn back_tab_cycles_backwards() {
        let mut form = TransactionForm::new_add();

        // From Date, back_tab wraps to Account.
        assert_eq!(form.component.focused_field, FormField::Date);
        form.on(back_tab_event());
        assert_eq!(form.component.focused_field, FormField::Account);
    }
}
