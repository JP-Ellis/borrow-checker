//! Budget list sidebar component.
//!
//! Renders a flat list of budgets as a navigable tree using
//! [`tui_tree_widget::Tree`] and [`tui_tree_widget::TreeState`].
//!
//! Selection changes emit [`crate::msg::BudgetMsg::BudgetSelected`].

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

// MARK: private component

/// Raw widget that renders the budget list sidebar.
struct Sidebar {
    /// Component props storage.
    props: Props,
    /// Scrolling / selection state for the tree widget.
    tree_state: TreeState<bc_models::BudgetId>,
    /// Pre-built tree items passed to [`Tree`] on each render.
    tree_items: Vec<TreeItem<'static, bc_models::BudgetId>>,
}

impl Sidebar {
    /// Build a new `Sidebar` from a flat list of budgets.
    ///
    /// # Arguments
    ///
    /// * `budgets` - All budgets to display.
    ///
    /// # Returns
    ///
    /// A new `Sidebar` with the tree fully built.
    fn new(budgets: &[bc_models::Budget]) -> Self {
        let tree_items: Vec<TreeItem<'static, bc_models::BudgetId>> = budgets
            .iter()
            .map(|b| {
                let label = b
                    .name()
                    .map_or_else(|| b.account_id().to_string(), str::to_owned);
                TreeItem::new_leaf(b.id().clone(), label)
            })
            .collect();

        Self {
            props: Props::default(),
            tree_state: TreeState::default(),
            tree_items,
        }
    }
}

impl Component for Sidebar {
    #[inline]
    #[expect(
        clippy::expect_used,
        reason = "Tree::new fails only on duplicate identifiers; BudgetId values are unique UUIDs"
    )]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .is_some_and(|v| matches!(*v, AttrValue::Flag(true)));
        let border_color = if focused { Color::Cyan } else { Color::White };
        let block = Block::default()
            .title(" Budgets ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        let tree = Tree::new(&self.tree_items)
            .expect("budget IDs are unique")
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

/// Tui-realm component wrapper for the budget list sidebar widget.
///
/// Handles keyboard navigation and emits
/// [`BudgetMsg::BudgetSelected`](crate::msg::BudgetMsg::BudgetSelected)
/// when the user confirms a budget. Pressing `'a'` emits
/// [`BudgetMsg::OpenAllocate`](crate::msg::BudgetMsg::OpenAllocate).
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as sidebar::BudgetSidebar; repetition is intentional"
)]
#[non_exhaustive]
#[derive(Component)]
pub struct BudgetSidebar {
    /// Inner raw widget.
    component: Sidebar,
}

impl BudgetSidebar {
    /// Create a new `BudgetSidebar` displaying the given budgets.
    ///
    /// # Arguments
    ///
    /// * `budgets` - Flat list of all budgets to show.
    ///
    /// # Returns
    ///
    /// A new `BudgetSidebar` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(budgets: &[bc_models::Budget]) -> Self {
        Self {
            component: Sidebar::new(budgets),
        }
    }
}

impl AppComponent<Msg, NoUserEvent> for BudgetSidebar {
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
                // Emit BudgetSelected when an item is confirmed.
                if let State::Single(StateValue::String(ref s)) = self.component.state()
                    && let Ok(id) = s.parse::<bc_models::BudgetId>()
                {
                    return Some(Msg::Budget(BudgetMsg::BudgetSelected(id)));
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

    #[test]
    fn empty_sidebar_has_no_state() {
        let sidebar = Sidebar::new(&[]);
        assert_eq!(sidebar.state(), State::None);
    }

    #[test]
    fn perform_move_down_on_empty_tree_does_not_panic() {
        let mut sidebar = Sidebar::new(&[]);
        let result = sidebar.perform(Cmd::Move(Direction::Down));
        assert!(matches!(
            result,
            CmdResult::Changed(_) | CmdResult::NoChange
        ));
    }

    #[test]
    fn single_budget_builds_tree() {
        let budget = bc_models::Budget::builder()
            .account_id(bc_models::AccountId::new())
            .period(bc_models::Period::Monthly)
            .rollover(bc_models::RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build();
        let sidebar = Sidebar::new(&[budget]);
        assert_eq!(sidebar.state(), State::None);
        assert_eq!(sidebar.tree_items.len(), 1);
    }

    #[test]
    fn perform_unknown_cmd_returns_none() {
        let mut sidebar = Sidebar::new(&[]);
        let result = sidebar.perform(Cmd::None);
        assert_eq!(result, CmdResult::NoChange);
    }

    #[test]
    fn envelope_sidebar_on_unknown_event_returns_none() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::None);
        assert_eq!(result, None);
    }

    #[test]
    fn envelope_sidebar_right_on_empty_tree_emits_redraw() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Right,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn j_key_emits_redraw() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('j'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn k_key_emits_redraw() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('k'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn h_key_emits_redraw() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('h'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn bracket_key_emits_period_prev() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('['),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodPrev)));
    }

    #[test]
    fn close_bracket_key_emits_period_next() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char(']'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::PeriodNext)));
    }

    #[test]
    fn envelope_sidebar_a_key_emits_open_allocate() {
        let mut sidebar = BudgetSidebar::new(&[]);
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('a'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Budget(BudgetMsg::OpenAllocate)));
    }
}
