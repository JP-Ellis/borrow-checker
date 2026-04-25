//! Status bar chrome component — display-only bottom bar.

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
use tuirealm::props::Color;
use tuirealm::props::Style;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::widgets::Paragraph;

use crate::msg::Msg;

/// Raw widget that renders the status bar.
struct Widget {
    /// Component props storage.
    props: Props,
    /// Fallback content when no `Attribute::Text` prop is set.
    content: String,
}

impl Widget {
    /// Create a new status bar with initial content.
    #[inline]
    #[must_use]
    fn new() -> Self {
        Self {
            props: Props::default(),
            content: "NORMAL  │  —  │  AUD  │  ?=help".into(),
        }
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
            .unwrap_or_else(|| self.content.clone());
        frame.render_widget(
            Paragraph::new(content).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
            area,
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

/// Tui-realm component wrapper for the status bar widget.
#[derive(MockComponent)]
#[non_exhaustive]
pub struct StatusBar {
    /// Inner raw widget.
    component: Widget,
}

impl StatusBar {
    /// Create a new `StatusBar`.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            component: Widget::new(),
        }
    }
}

impl Default for StatusBar {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Component<Msg, NoUserEvent> for StatusBar {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        // Display-only: no events handled.
        None
    }
}
