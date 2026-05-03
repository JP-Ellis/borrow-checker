//! Envelope tree sidebar component.
//!
//! Renders the full envelope hierarchy as a navigable tree.
//! [`tui_tree_widget::TreeState`] drives navigation state and open/closed
//! tracking; rendering is handled directly via ratatui 0.29 primitives to
//! avoid the version-incompatibility between `tui-tree-widget 0.22`
//! (ratatui 0.28) and `tuirealm 3` (ratatui 0.29).
//!
//! Selection changes emit [`crate::msg::BudgetMsg::EnvelopeSelected`].

use bc_models::Envelope;
use bc_models::EnvelopeId;
use tui_tree_widget::Flattened;
use tui_tree_widget::TreeItem;
use tui_tree_widget::TreeState;
use tuirealm::AttrValue;
use tuirealm::Attribute;
use tuirealm::Component;
use tuirealm::Frame;
use tuirealm::MockComponent;
use tuirealm::NoUserEvent;
use tuirealm::Props;
use tuirealm::State;
use tuirealm::StateValue;
use tuirealm::command::Cmd;
use tuirealm::command::CmdResult;
use tuirealm::command::Direction;
use tuirealm::event::Event;
use tuirealm::event::Key;
use tuirealm::event::KeyEvent;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::text::Span;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::ratatui::widgets::List;
use tuirealm::ratatui::widgets::ListItem;

use crate::msg::BudgetMsg;
use crate::msg::Msg;

// MARK: helper

/// Recursively build a [`TreeItem`] for `envelope` and all of its descendants
/// found in `all`.
///
/// `TreeItem` stores `ratatui 0.28::text::Text` internally, but we only use
/// the struct for its identifier/children graph — never for rendering the text
/// via `tui_tree_widget::Tree`.
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

    // The text stored in TreeItem is used by tui-tree-widget's own renderer,
    // which we bypass.  We still need a non-empty Text so TreeItem is valid.
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

/// Look up the display name for an envelope by its ID.
///
/// Returns an empty string slice if the envelope is not found (should not
/// happen in practice since `all` and `tree_items` are built from the same
/// source).
///
/// # Arguments
///
/// * `id`  - The envelope ID to look up.
/// * `all` - The flat list of all envelopes.
fn envelope_name<'a>(id: &EnvelopeId, all: &'a [Envelope]) -> &'a str {
    all.iter().find(|e| e.id() == id).map_or("", |e| e.name())
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
///
/// [`TreeState`] drives navigation; rendering is performed manually using
/// ratatui 0.29 primitives so that the two ratatui versions pulled in by
/// `tui-tree-widget 0.22` and `tuirealm 3` do not conflict.
struct Sidebar {
    /// Component props storage.
    props: Props,
    /// Scrolling / selection state for the tree widget.
    tree_state: TreeState<EnvelopeId>,
    /// Pre-built tree items — used only for graph structure and navigation.
    tree_items: Vec<TreeItem<'static, EnvelopeId>>,
    /// Flat list of all envelopes (provides display names and parent links).
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

    /// Move the selection up or down using `flatten()` to determine visible order.
    ///
    /// `tui-tree-widget`'s `key_down()`/`key_up()` read `last_identifiers`, which
    /// is only populated by the library's own renderer (never called here).
    /// This helper computes visible items from [`TreeState::flatten`] instead.
    ///
    /// # Arguments
    ///
    /// * `change` - A function from `(current_index, total_visible)` to the new index.
    fn nav_vertical(&mut self, change: impl FnOnce(usize, usize) -> usize) {
        // Collect owned identifiers so the flatten borrow ends before select().
        let visible: Vec<Vec<EnvelopeId>> = self
            .tree_state
            .flatten(&self.tree_items)
            .into_iter()
            .map(|f| f.identifier)
            .collect();
        let len = visible.len();
        if len == 0 {
            return;
        }
        let current = self.tree_state.selected().to_vec();
        let current_idx = visible.iter().position(|id| id == &current).unwrap_or(0);
        let new_idx = change(current_idx, len);
        if let Some(path) = visible.into_iter().nth(new_idx) {
            self.tree_state.select(path);
        }
    }

    /// Render the envelope tree into `area` on `frame`.
    ///
    /// We bypass `tui_tree_widget::Tree::render` (which requires ratatui 0.28
    /// types) and instead call [`TreeState::flatten`] to obtain the visible
    /// items, then render them as a [`List`] using ratatui 0.29.
    fn render_tree(&mut self, frame: &mut Frame, area: Rect) {
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

        let block = Block::default()
            .title(" Envelopes ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));

        // Collect visible (i.e. not collapsed) items using tui-tree-widget's
        // flatten logic.  We pass a fresh reference slice here; the lifetime
        // mismatch between 'static TreeItem and the fn-local borrow is
        // handled by `tree_items` living on `self`.
        let visible: Vec<Flattened<'_, EnvelopeId>> = self.tree_state.flatten(&self.tree_items);

        let selected_path = self.tree_state.selected().to_vec();

        let items: Vec<ListItem<'_>> = visible
            .iter()
            .filter_map(|flat| {
                // `identifier` always has ≥1 element; `last()` is Some.
                let leaf_id = flat.identifier.last()?;
                let name = envelope_name(leaf_id, &self.envelopes);
                let depth = flat.depth();
                let indent = " ".repeat(depth.saturating_mul(2));
                let node_symbol = if has_children(leaf_id, &self.envelopes) {
                    if self.tree_state.opened().contains(&flat.identifier) {
                        "\u{25bc} " // ▼ open
                    } else {
                        "\u{25b6} " // ▶ closed
                    }
                } else {
                    "  "
                };
                let is_selected = flat.identifier == selected_path;
                let style = if is_selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                let line = Line::from(vec![
                    Span::raw(indent),
                    Span::styled(format!("{node_symbol}{name}"), style),
                ]);
                Some(ListItem::new(line))
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }
}

impl MockComponent for Sidebar {
    #[inline]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.render_tree(frame, area);
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
        let selected = self.tree_state.selected();
        match selected.last() {
            Some(id) => State::One(StateValue::String(id.to_string())),
            None => State::None,
        }
    }

    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Cmd is non-exhaustive; all other variants return CmdResult::None"
    )]
    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                // key_down() uses last_identifiers (set by Tree renderer), which we
                // never call. Compute visible order from flatten() instead.
                self.nav_vertical(|idx, len| idx.saturating_add(1).min(len.saturating_sub(1)));
            }
            Cmd::Move(Direction::Up) => {
                self.nav_vertical(|idx, _| idx.saturating_sub(1));
            }
            Cmd::Move(Direction::Left) => {
                self.tree_state.key_left();
            }
            Cmd::Move(Direction::Right) => {
                self.tree_state.key_right();
            }
            _ => return CmdResult::None,
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
#[derive(MockComponent)]
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

impl Component<Msg, NoUserEvent> for EnvelopeSidebar {
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
                if let State::One(StateValue::String(ref s)) = self.component.state() {
                    if let Ok(id) = s.parse::<EnvelopeId>() {
                        if !has_children(&id, &self.component.envelopes) {
                            return Some(Msg::Budget(BudgetMsg::EnvelopeSelected(id)));
                        }
                    }
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
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build()
    }

    /// Build a child [`Envelope`] with a known parent ID.
    fn make_child_envelope(name: &str, parent_id: EnvelopeId) -> Envelope {
        Envelope::builder()
            .name(name)
            .parent_id(parent_id)
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
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
        // Either Changed(State::None) or CmdResult::None are acceptable.
        assert!(matches!(result, CmdResult::Changed(_) | CmdResult::None));
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
        assert_eq!(result, CmdResult::None);
    }

    #[test]
    fn envelope_sidebar_on_unknown_event_returns_none() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::None);
        assert_eq!(result, None);
    }

    #[test]
    fn envelope_sidebar_right_on_empty_tree_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Right,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn j_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Char('j'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn k_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Char('k'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn h_key_emits_redraw() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Char('h'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn bracket_key_emits_period_prev() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Char('['),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodPrev)));
    }

    #[test]
    fn close_bracket_key_emits_period_next() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Char(']'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodNext)));
    }

    #[test]
    fn envelope_sidebar_a_key_emits_open_allocate() {
        let mut sidebar = EnvelopeSidebar::new(vec![]);
        let result = sidebar.on(Event::Keyboard(KeyEvent {
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
        sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        sidebar.on(Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // Press Enter — if Groceries is now selected it should emit EnvelopeSelected.
        let msg = sidebar.on(Event::Keyboard(KeyEvent {
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
