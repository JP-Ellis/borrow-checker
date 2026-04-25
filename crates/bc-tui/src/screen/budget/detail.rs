//! Envelope detail panel — full implementation in Task 3.

use bc_core::EnvelopeStatus;
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
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::Paragraph;

use crate::msg::Msg;

/// Raw widget stub for the envelope detail panel.
///
/// The full detail panel is implemented in Task 3. This stub renders a
/// placeholder paragraph so that the screen can mount and unmount cleanly.
struct EnvelopeDetailInner {
    /// Component props storage.
    props: Props,
    /// Current envelope status to display (used in the full Task 3 implementation).
    status: Option<EnvelopeStatus>,
}

impl MockComponent for EnvelopeDetailInner {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().title(" Detail ").borders(Borders::ALL);
        let paragraph = Paragraph::new("Detail (stub)").block(block);
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

/// Tui-realm component wrapper for the envelope detail panel stub.
///
/// The full status display is implemented in Task 3. This stub is sufficient
/// for [`crate::screen::budget::BudgetScreen`] to mount and unmount correctly.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as detail::EnvelopeDetail; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct EnvelopeDetail {
    /// Inner raw widget.
    component: EnvelopeDetailInner,
}

impl EnvelopeDetail {
    /// Create a new `EnvelopeDetail` displaying the given envelope status.
    ///
    /// # Arguments
    ///
    /// * `status` - Optional envelope status to display, or `None` if no envelope is selected.
    ///
    /// # Returns
    ///
    /// A new `EnvelopeDetail` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(status: Option<EnvelopeStatus>) -> Self {
        Self {
            component: EnvelopeDetailInner {
                props: Props::default(),
                status,
            },
        }
    }
}

impl Component<Msg, NoUserEvent> for EnvelopeDetail {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
