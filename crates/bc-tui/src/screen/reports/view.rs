//! Report selector and output view component.
//!
//! [`ReportView`] is a dual-mode component:
//! - [`ViewState::Selecting`] — renders the list of available reports as a navigable [`List`].
//! - [`ViewState::Viewing`] — renders the report output as a scrollable [`Paragraph`].
//!
//! State transitions:
//! - `Selecting` → `Viewing`: triggered by `app.attr(…, Attribute::Text, AttrValue::String(s))`.
//! - `Viewing` → `Selecting`: triggered by the `Esc` key.

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
use tuirealm::ratatui::widgets::Paragraph;
use tuirealm::ratatui::widgets::Wrap;

use crate::msg::Msg;
use crate::msg::ReportKind;
use crate::msg::ReportsMsg;

// ─── constants ────────────────────────────────────────────────────────────────

/// The available reports, paired with their display label and [`ReportKind`].
const REPORTS: &[(&str, ReportKind)] = &[
    ("Net Worth", ReportKind::NetWorth),
    ("Monthly Summary", ReportKind::MonthlySummary),
    ("Budget Summary", ReportKind::BudgetSummary),
];

// ─── view state ───────────────────────────────────────────────────────────────

/// The current display mode of the [`ReportViewWidget`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum ViewState {
    /// Showing the report selector list.
    #[default]
    Selecting,
    /// Showing the output of a completed report.
    Viewing,
}

// ─── private component ───────────────────────────────────────────────────────

/// Raw widget that renders either the report selector or report output.
struct ReportViewWidget {
    /// Component props storage.
    props: Props,
    /// Whether we are selecting a report or viewing output.
    view_state: ViewState,
    /// Index of the currently highlighted report in [`REPORTS`].
    selected: usize,
    /// Formatted report output string.
    output: String,
    /// Scroll offset for the output [`Paragraph`].
    scroll_offset: usize,
}

impl ReportViewWidget {
    /// Store report output and switch to viewing mode.
    ///
    /// # Arguments
    ///
    /// * `s` - The formatted report output string.
    #[inline]
    fn set_output(&mut self, s: String) {
        self.output = s;
        self.view_state = ViewState::Viewing;
        self.scroll_offset = 0;
    }

    /// Render the focused border colour.
    #[inline]
    fn border_color(&self) -> Color {
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

        if focused { Color::Cyan } else { Color::White }
    }
}

impl MockComponent for ReportViewWidget {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let border_color = self.border_color();

        match self.view_state {
            ViewState::Selecting => {
                let block = Block::default()
                    .title(" Reports ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color));

                let items: Vec<ListItem<'_>> = REPORTS
                    .iter()
                    .map(|(label, _)| ListItem::new(*label))
                    .collect();

                let list = List::new(items)
                    .block(block)
                    .highlight_style(Style::default().fg(Color::Yellow))
                    .highlight_symbol("> ");

                let mut state = ListState::default();
                state.select(Some(self.selected));

                frame.render_stateful_widget(list, area, &mut state);
            }
            ViewState::Viewing => {
                let block = Block::default()
                    .title(" Report Output ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color));

                #[expect(
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    reason = "scroll_offset grows via saturating_add; truncation to u16 is acceptable for any realistic report length"
                )]
                let paragraph = Paragraph::new(self.output.as_str())
                    .block(block)
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll_offset as u16, 0));

                frame.render_widget(paragraph, area);
            }
        }
    }

    #[inline]
    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    #[inline]
    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if attr == Attribute::Text {
            if let AttrValue::String(s) = value {
                self.set_output(s);
                return;
            }
        }
        self.props.set(attr, value);
    }

    #[inline]
    fn state(&self) -> State {
        State::None
    }

    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Cmd is non-exhaustive; all other variants return CmdResult::None"
    )]
    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                match self.view_state {
                    ViewState::Selecting => {
                        let max = REPORTS.len().saturating_sub(1);
                        self.selected = self.selected.saturating_add(1).min(max);
                    }
                    ViewState::Viewing => {
                        self.scroll_offset = self.scroll_offset.saturating_add(1);
                    }
                }
                CmdResult::Changed(self.state())
            }
            Cmd::Move(Direction::Up) => {
                match self.view_state {
                    ViewState::Selecting => {
                        self.selected = self.selected.saturating_sub(1);
                    }
                    ViewState::Viewing => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    }
                }
                CmdResult::Changed(self.state())
            }
            Cmd::Custom("back") => {
                self.view_state = ViewState::Selecting;
                CmdResult::Changed(self.state())
            }
            _ => CmdResult::None,
        }
    }
}

// ─── public wrapper ──────────────────────────────────────────────────────────

/// Tui-realm component wrapper for the report selector/output view widget.
///
/// When `ViewState::Selecting`:
/// - `j`/`↓` moves the highlight down.
/// - `k`/`↑` moves the highlight up.
/// - `Enter` emits [`ReportsMsg::RunReport`] for the selected report.
///
/// When `ViewState::Viewing`:
/// - `j`/`↓` scrolls the output down.
/// - `k`/`↑` scrolls the output up.
/// - `Esc` returns to the selector.
/// - `r` emits [`ReportsMsg::Refresh`].
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as view::ReportView; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct ReportView {
    /// Inner raw widget.
    component: ReportViewWidget,
}

impl ReportView {
    /// Create a new `ReportView` in the selecting state.
    ///
    /// # Returns
    ///
    /// A new `ReportView` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            component: ReportViewWidget {
                props: Props::default(),
                view_state: ViewState::default(),
                selected: 0,
                output: String::new(),
                scroll_offset: 0,
            },
        }
    }
}

impl Default for ReportView {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Component<Msg, NoUserEvent> for ReportView {
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
                code: Key::Enter, ..
            }) => {
                if self.component.view_state == ViewState::Selecting {
                    let idx = self.component.selected;
                    REPORTS
                        .get(idx)
                        .map(|(_, kind)| Msg::Reports(ReportsMsg::RunReport(kind.clone())))
                } else {
                    None
                }
            }
            Event::Keyboard(KeyEvent { code: Key::Esc, .. }) => {
                (self.component.view_state == ViewState::Viewing).then(|| {
                    self.component.perform(Cmd::Custom("back"));
                    Msg::Reports(ReportsMsg::BackToSelector)
                })
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char('r'),
                ..
            }) => (self.component.view_state == ViewState::Viewing)
                .then_some(Msg::Reports(ReportsMsg::Refresh)),
            _ => None,
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tuirealm::command::Direction;

    use super::*;

    fn make_widget() -> ReportViewWidget {
        ReportViewWidget {
            props: Props::default(),
            view_state: ViewState::default(),
            selected: 0,
            output: String::new(),
            scroll_offset: 0,
        }
    }

    #[test]
    fn initial_state_is_selecting() {
        let w = make_widget();
        assert_eq!(w.view_state, ViewState::Selecting);
        assert_eq!(w.selected, 0);
    }

    #[test]
    fn move_down_increments_selection() {
        let mut w = make_widget();
        w.perform(Cmd::Move(Direction::Down));
        assert_eq!(w.selected, 1);
    }

    #[test]
    fn move_up_does_not_underflow() {
        let mut w = make_widget();
        // Already at 0; moving up should keep it at 0.
        w.perform(Cmd::Move(Direction::Up));
        assert_eq!(w.selected, 0);
    }

    #[test]
    fn set_output_switches_to_viewing() {
        let mut w = make_widget();
        w.set_output("Net Worth\n─────────\nAUD 1234".to_owned());
        assert_eq!(w.view_state, ViewState::Viewing);
        assert_eq!(w.scroll_offset, 0);
    }

    #[test]
    fn move_down_in_viewing_mode_scrolls() {
        let mut w = make_widget();
        w.set_output("line1\nline2\nline3".to_owned());
        assert_eq!(w.view_state, ViewState::Viewing);
        w.perform(Cmd::Move(Direction::Down));
        assert_eq!(w.scroll_offset, 1);
    }
}
