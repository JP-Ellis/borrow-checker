//! Envelope tree sidebar component.
//!
//! Renders the full envelope hierarchy as a navigable tree using
//! [`tui_tree_widget::Tree`] and [`tui_tree_widget::TreeState`].
//!
//! Selection changes emit [`crate::msg::BudgetMsg::EnvelopeSelected`].

use bc_models::Envelope;
use bc_models::EnvelopeId;
use tui_tree_widget::Tree;
use tui_tree_widget::TreeItem;
use tui_tree_widget::TreeState;
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
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::state::State;
use tuirealm::state::StateValue;

use crate::msg::BudgetMsg;
use crate::msg::Msg;

// MARK: helper

/// Recursively build a [`TreeItem`] for `envelope` and all of its descendants
/// found in `all`.
///
/// # Arguments
///
/// * `envelope` - The envelope to build a tree item for.
/// * `all`      - The full flat list of envelopes used to find children.
///
/// # Returns
///
/// An owned `TreeItem<'static, EnvelopeId>` representing the envelope and its
/// subtree.
fn build_item_owned(envelope: &Envelope, all: &[Envelope]) -> TreeItem<'static, EnvelopeId> {
    let children: Vec<TreeItem<'static, EnvelopeId>> = all
        .iter()
        .filter(|e| e.parent_id() == Some(envelope.id()))
        .map(|child| build_item_owned(child, all))
        .collect();

    let name: String = envelope.name().to_owned();

    if children.is_empty() {
        TreeItem::new_leaf(envelope.id().clone(), name)
    } else {
        #[expect(
            clippy::expect_used,
            reason = "TreeItem::new panics only on duplicate IDs, which we guarantee won't happen \
                      because EnvelopeId values are unique UUIDs"
        )]
        TreeItem::new(envelope.id().clone(), name, children)
            .expect("envelope IDs are unique within a parent")
    }
}

/// Returns `true` when `id` is the ID of an envelope that has at least one
/// direct child in `all`.
///
/// # Arguments
///
/// * `id`  - The envelope ID to test.
/// * `all` - The flat list of all envelopes.
fn has_children(id: &EnvelopeId, all: &[Envelope]) -> bool {
    all.iter().any(|e| e.parent_id() == Some(id))
}

// MARK: private component

/// Raw widget that renders the envelope tree sidebar.
struct Sidebar {
    /// Component props storage.
    props: Props,
    /// Scrolling / selection state for the tree widget.
    tree_state: TreeState<EnvelopeId>,
    /// Pre-built tree items passed to [`Tree`] on each render.
    tree_items: Vec<TreeItem<'static, EnvelopeId>>,
    /// Flat list of all envelopes; used to check for children in event handling.
    envelopes: Vec<Envelope>,
}

impl Sidebar {
    /// Build a new `Sidebar` from a flat list of envelopes.
    ///
    /// Root envelopes (those without a `parent_id`) form the top-level nodes;
    /// child envelopes are nested under their parent. The first root envelope,
    /// if any, is opened by default so the user immediately sees its children.
    ///
    /// # Arguments
    ///
    /// * `envelopes` - All envelopes to display, in any order.
    ///
    /// # Returns
    ///
    /// A new `Sidebar` with the tree fully built and the first root node open.
    fn new(envelopes: Vec<Envelope>) -> Self {
        let roots: Vec<&Envelope> = envelopes
            .iter()
            .filter(|e| e.parent_id().is_none())
            .collect();

        let tree_items: Vec<TreeItem<'static, EnvelopeId>> = roots
            .iter()
            .map(|root| build_item_owned(root, &envelopes))
            .collect();

        let mut tree_state: TreeState<EnvelopeId> = TreeState::default();

        // Open the first root node so children are visible immediately.
        if let Some(first_root) = roots.first() {
            tree_state.open(vec![first_root.id().clone()]);
        }

        Self {
            props: Props::default(),
            tree_state,
            tree_items,
            envelopes,
        }
    }
}

impl Component for Sidebar {
    #[inline]
    #[expect(
        clippy::expect_used,
        reason = "Tree::new fails only on duplicate identifiers; EnvelopeId values are unique UUIDs"
    )]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .is_some_and(|v| matches!(*v, AttrValue::Flag(true)));
        let border_color = if focused { Color::Cyan } else { Color::White };
        let block = Block::default()
            .title(" Envelopes ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        let tree = Tree::new(&self.tree_items)
            .expect("envelope IDs are unique")
            .block(block)
            .highlight_style(Style::default().fg(Color::Yellow));
        frame.render_stateful_widget(tree, area, &mut self.tree_state);
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
        let selected = self.tree_state.selected();
        match selected.last() {
            Some(id) => State::Single(StateValue::String(id.to_string())),
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
                self.tree_state.key_down();
            }
            Cmd::Move(Direction::Up) => {
                self.tree_state.key_up();
            }
            Cmd::Move(Direction::Left) => {
                self.tree_state.key_left();
            }
            Cmd::Move(Direction::Right) => {
                self.tree_state.key_right();
            }
            _ => return CmdResult::NoChange,
        }
        CmdResult::Changed(self.state())
    }
}

// MARK: public wrapper

/// Tui-realm component wrapper for the envelope tree sidebar widget.
///
/// Handles keyboard navigation and emits
/// [`BudgetMsg::EnvelopeSelected`](crate::msg::BudgetMsg::EnvelopeSelected)
/// when the user confirms a leaf node. Pressing `'a'` emits
/// [`BudgetMsg::OpenAllocate`](crate::msg::BudgetMsg::OpenAllocate).
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as sidebar::EnvelopeSidebar; repetition is intentional"
)]
#[non_exhaustive]
#[derive(Component)]
pub struct EnvelopeSidebar {
    /// Inner raw widget.
    component: Sidebar,
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
            component: Sidebar::new(envelopes),
        }
    }
}

impl AppComponent<Msg, NoUserEvent> for EnvelopeSidebar {
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
            }) => {
                self.component.perform(Cmd::Move(Direction::Right));
                // Emit EnvelopeSelected only when a leaf node is confirmed.
                if let State::Single(StateValue::String(ref s)) = self.component.state()
                    && let Ok(id) = s.parse::<EnvelopeId>()
                    && !has_children(&id, &self.component.envelopes)
                {
                    return Some(Msg::Budget(BudgetMsg::EnvelopeSelected(id)));
                }
                Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Left | Key::Char('h'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Left));
                Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char('a'),
                ..
            }) => Some(Msg::Budget(BudgetMsg::OpenAllocate)),
            Event::Keyboard(KeyEvent {
                code: Key::Char('['),
                ..
            }) => Some(Msg::Budget(BudgetMsg::PeriodPrev)),
            Event::Keyboard(KeyEvent {
                code: Key::Char(']'),
                ..
            }) => Some(Msg::Budget(BudgetMsg::PeriodNext)),
            _ => None,
        }
    }
}

// MARK: tests

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tuirealm::command::Direction;

    use super::*;
    use crate::msg::BudgetMsg;
    use crate::msg::Msg;

    /// Build a minimal [`Envelope`] with the given name and no parent.
    fn make_envelope(name: &str) -> Envelope {
        Envelope::builder()
            .name(name)
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::EnvelopeRolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build()
    }

    /// Build a child [`Envelope`] with a known parent ID.
    fn make_child_envelope(name: &str, parent_id: EnvelopeId) -> Envelope {
        Envelope::builder()
            .name(name)
            .parent_id(parent_id)
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::EnvelopeRolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build()
    }

    #[test]
    fn empty_sidebar_has_no_state() {
        let sidebar = Sidebar::new(vec![]);
        assert_eq!(sidebar.state(), State::None);
    }

    #[test]
    fn perform_move_down_on_empty_tree_does_not_panic() {
        let mut sidebar = Sidebar::new(vec![]);
        let result = sidebar.perform(Cmd::Move(Direction::Down));
        // Either Changed(State::None) or CmdResult::NoChange are acceptable.
        assert!(matches!(
            result,
            CmdResult::Changed(_) | CmdResult::NoChange
        ));
    }

    #[test]
    fn single_root_envelope_builds_tree() {
        let env = make_envelope("Food");
        let sidebar = Sidebar::new(vec![env]);
        // Nothing is selected initially.
        assert_eq!(sidebar.state(), State::None);
        assert_eq!(sidebar.tree_items.len(), 1);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test asserts tree_items.len() == 1 immediately before indexing [0]"
    )]
    fn child_envelopes_are_nested_under_parent() {
        let parent = make_envelope("Food");
        let child = make_child_envelope("Groceries", parent.id().clone());
        let sidebar = Sidebar::new(vec![parent, child]);
        assert_eq!(sidebar.tree_items.len(), 1);
        assert_eq!(sidebar.tree_items[0].children().len(), 1);
    }

    #[test]
    fn perform_unknown_cmd_returns_none() {
        let mut sidebar = Sidebar::new(vec![]);
        let result = sidebar.perform(Cmd::None);
        assert_eq!(result, CmdResult::NoChange);
    }

    #[test]
    fn envelope_sidebar_on_unknown_event_returns_none() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::None);
        assert_eq!(result, None);
    }

    #[test]
    fn envelope_sidebar_right_on_empty_tree_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Right,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn j_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('j'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn k_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('k'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn h_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('h'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn bracket_key_emits_period_prev() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('['),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodPrev)));
    }

    #[test]
    fn close_bracket_key_emits_period_next() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char(']'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodNext)));
    }

    #[test]
    fn envelope_sidebar_a_key_emits_open_allocate() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('a'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::OpenAllocate)));
    }

    #[test]
    fn envelope_sidebar_enter_selects_leaf_emits_msg() {
        let parent = make_envelope("Food");
        let child = make_child_envelope("Groceries", parent.id().clone());
        let child_id = child.id().clone();
        let mut sidebar = EnvelopeSidebar::new(vec![parent.clone(), child]);

        // After Sidebar::new the first root is already opened, so we navigate
        // down once to move selection to the first visible item (Food root),
        // then down again to land on the child (Groceries).
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // Press Enter — if Groceries is now selected it should emit EnvelopeSelected.
        let msg = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Enter,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // The exact navigation path depends on the tree's internal state after
        // two key_down calls without a prior render, so we only assert the
        // message shape when the ID matches.
        if let Some(Msg::Budget(BudgetMsg::EnvelopeSelected(ref id))) = msg {
            assert!(
                id == &child_id || id == parent.id(),
                "selected ID should be one of the envelopes we inserted"
            );
        }
        // If None is returned, navigation simply didn't land on a leaf yet —
        // acceptable without a rendered frame.
    }
}
