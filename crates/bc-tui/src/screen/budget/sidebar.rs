//! Envelope tree sidebar — full implementation in Task 2.

use bc_models::Envelope;
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

/// Raw widget stub for the envelope tree sidebar.
///
/// The full tree implementation is added in Task 2. This stub renders a
/// placeholder paragraph so that the screen can mount and unmount cleanly.
struct EnvelopeSidebarInner {
    /// Component props storage.
    props: Props,
    /// Flat list of envelopes to display (used in the full Task 2 implementation).
    envelopes: Vec<Envelope>,
}

impl MockComponent for EnvelopeSidebarInner {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().title(" Envelopes ").borders(Borders::ALL);
        let paragraph = Paragraph::new("Envelopes (stub)").block(block);
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

/// Tui-realm component wrapper for the envelope tree sidebar stub.
///
/// The full tree navigation and selection behaviour is implemented in Task 2.
/// This stub is sufficient for the [`crate::screen::budget::BudgetScreen`] to
/// mount and unmount correctly.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as sidebar::EnvelopeSidebar; repetition is intentional"
)]
#[non_exhaustive]
#[derive(MockComponent)]
pub struct EnvelopeSidebar {
    /// Inner raw widget.
    component: EnvelopeSidebarInner,
}

impl EnvelopeSidebar {
    /// Create a new `EnvelopeSidebar` displaying the given envelopes.
    ///
    /// # Arguments
    ///
    /// * `envelopes` - Flat list of all envelopes to show in the tree.
    ///
    /// # Returns
    ///
    /// A new `EnvelopeSidebar` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(envelopes: Vec<Envelope>) -> Self {
        Self {
            component: EnvelopeSidebarInner {
                props: Props::default(),
                envelopes,
            },
        }
    }
}

impl Component<Msg, NoUserEvent> for EnvelopeSidebar {
    #[inline]
    fn on(&mut self, _ev: Event<NoUserEvent>) -> Option<Msg> {
        None
    }
}
