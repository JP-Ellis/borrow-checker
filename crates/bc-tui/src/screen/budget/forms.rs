//! Allocation form overlay.
//!
//! Renders a centred floating popup with a single amount input field for
//! allocating funds to an envelope.

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

use crate::msg::BudgetMsg;
use crate::msg::Msg;

// ─── private component ───────────────────────────────────────────────────────

/// Raw widget that renders the allocation form.
struct AllocForm {
    /// Component props storage.
    props: Props,
    /// Amount field input buffer.
    amount: Input,
    /// Name of the envelope being allocated to.
    envelope_name: String,
}

impl AllocForm {
    /// Create a new empty allocation form.
    ///
    /// # Arguments
    ///
    /// * `envelope_name` - Name of the envelope to allocate to.
    ///
    /// # Returns
    ///
    /// An [`AllocForm`] ready to be mounted with an empty amount field.
    fn new(envelope_name: impl Into<String>) -> Self {
        Self {
            props: Props::default(),
            amount: Input::default(),
            envelope_name: envelope_name.into(),
        }
    }

    /// Compute a centred ~30% × ~40% popup [`Rect`] from `area`.
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
                Constraint::Percentage(35),
                Constraint::Percentage(30),
                Constraint::Percentage(35),
            ])
            .split(area);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ])
            .split(v[1])[1]
    }
}

impl MockComponent for AllocForm {
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 2 chunks matching the 2 constraints"
    )]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let popup = Self::popup_rect(area);
        frame.render_widget(Clear, popup);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(1)])
            .split(popup);

        let block = Block::default()
            .title(format!(" Allocate to: {} ", self.envelope_name))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow));
        let amount_field = Paragraph::new(self.amount.value()).block(block);
        frame.render_widget(amount_field, rows[0]);

        let hint = Paragraph::new("Enter=Allocate  Esc=Cancel");
        frame.render_widget(hint, rows[1]);
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

/// Tui-realm component wrapper for the allocation form overlay.
///
/// Displays a centred floating popup with a single amount input field.
/// Enter emits [`BudgetMsg::FormSubmitted`](crate::msg::BudgetMsg::FormSubmitted);
/// Esc emits [`BudgetMsg::FormCancelled`](crate::msg::BudgetMsg::FormCancelled).
#[non_exhaustive]
#[derive(MockComponent)]
pub struct AllocationForm {
    /// Inner raw widget.
    component: AllocForm,
}

impl AllocationForm {
    /// Create a new allocation form for the given envelope.
    ///
    /// # Arguments
    ///
    /// * `envelope_name` - Name of the envelope to display in the form title.
    ///
    /// # Returns
    ///
    /// An [`AllocationForm`] ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(envelope_name: impl Into<String>) -> Self {
        Self {
            component: AllocForm::new(envelope_name),
        }
    }
}

impl Component<Msg, NoUserEvent> for AllocationForm {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; all non-keyboard variants return None"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => Some(Msg::Budget(BudgetMsg::FormSubmitted)),
            Event::Keyboard(KeyEvent { code: Key::Esc, .. }) => {
                Some(Msg::Budget(BudgetMsg::FormCancelled))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char(c), ..
            }) => {
                self.component.amount.handle(InputRequest::InsertChar(c));
                None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Backspace,
                ..
            }) => {
                self.component.amount.handle(InputRequest::DeletePrevChar);
                None
            }
            _ => None,
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tuirealm::event::KeyModifiers;

    use super::*;

    fn esc_event() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Esc,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn enter_event() -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn form_esc_emits_form_cancelled() {
        let mut form = AllocationForm::new("Groceries");
        let msg = form.on(esc_event());
        assert_eq!(msg, Some(Msg::Budget(BudgetMsg::FormCancelled)));
    }

    #[test]
    fn form_enter_emits_form_submitted() {
        let mut form = AllocationForm::new("Groceries");
        let msg = form.on(enter_event());
        assert_eq!(msg, Some(Msg::Budget(BudgetMsg::FormSubmitted)));
    }
}
