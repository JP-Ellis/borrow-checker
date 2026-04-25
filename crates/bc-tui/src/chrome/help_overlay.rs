//! Help overlay chrome component — floating popup shown on `?`.

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
use tuirealm::props::BorderType;
use tuirealm::props::Color;
use tuirealm::props::Style;
use tuirealm::ratatui::layout::Constraint;
use tuirealm::ratatui::layout::Direction;
use tuirealm::ratatui::layout::Layout;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Clear;
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;

use crate::msg::Msg;

/// Raw widget that renders the floating help popup.
struct Widget {
    /// Component props storage.
    props: Props,
}

impl Widget {
    /// Create a new (hidden) help overlay.
    #[inline]
    #[must_use]
    fn new() -> Self {
        let mut props = Props::default();
        props.set(Attribute::Display, AttrValue::Flag(false));
        Self { props }
    }
}

impl Default for Widget {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl MockComponent for Widget {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let visible = self
            .props
            .get(Attribute::Display)
            .is_some_and(|v| matches!(v, AttrValue::Flag(true)));

        if !visible {
            return;
        }

        let content = self
            .props
            .get(Attribute::Text)
            .and_then(|v| {
                if let AttrValue::String(s) = v {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Calculate centred floating area (~60% width, ~70% height).
        let popup_area = centered_rect(60, 70, area);

        frame.render_widget(Clear, popup_area);
        frame.render_widget(
            Paragraph::new(content)
                .block(
                    Block::default()
                        .title(" Help — Esc to close ")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .wrap(Wrap { trim: false }),
            popup_area,
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

/// Compute a centred [`Rect`] that is `percent_x`% wide and `percent_y`% tall.
#[expect(
    clippy::indexing_slicing,
    reason = "layout always returns exactly 3 chunks matching the 3 constraints"
)]
#[expect(
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "integer division by 2 for symmetric padding is intentional; no precision loss matters here"
)]
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pad_y = 100_u16.saturating_sub(percent_y) / 2;
    let pad_x = 100_u16.saturating_sub(percent_x) / 2;
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(pad_y),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(pad_y),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(pad_x),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(pad_x),
        ])
        .split(vert[1])[1]
}

/// Tui-realm component wrapper for the help overlay widget.
#[derive(MockComponent)]
#[non_exhaustive]
pub struct HelpOverlay {
    /// Inner raw widget.
    component: Widget,
}

impl HelpOverlay {
    /// Create a new (hidden) `HelpOverlay`.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            component: Widget::new(),
        }
    }
}

impl Default for HelpOverlay {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Component<Msg, NoUserEvent> for HelpOverlay {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is non-exhaustive; all non-keyboard variants are no-ops"
    )]
    fn on(&mut self, ev: Event<NoUserEvent>) -> Option<Msg> {
        // Only Esc (or ?) closes the overlay. Other keys fall through so that
        // global TabBar subscriptions (q, 1/2/3, Tab) can still fire.
        match ev {
            Event::Keyboard(
                KeyEvent { code: Key::Esc, .. }
                | KeyEvent {
                    code: Key::Char('?'),
                    ..
                },
            ) => Some(Msg::HelpToggle),
            _ => None,
        }
    }
}
