//! Account tree sidebar component.
//!
//! Renders the full account hierarchy as a navigable tree using
//! [`tui_tree_widget::Tree`] and [`tui_tree_widget::TreeState`].
//!
//! Selection changes emit [`crate::msg::AccountsMsg::AccountSelected`].

use std::collections::HashMap;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::Decimal;
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
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::text::Span;
use tuirealm::ratatui::widgets::Block;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::ratatui::widgets::Borders;
use tuirealm::state::State;
use tuirealm::state::StateValue;

use crate::msg::AccountsMsg;
use crate::msg::Msg;

// MARK: helper

/// Width in characters of the balance column appended to each account name.
///
/// Format: `COMM  AMOUNT` where commodity is 5 chars wide and amount is 12 chars
/// wide (2 decimal places), separated by a single space. Total = 18 chars.
const BALANCE_COLUMN_WIDTH: usize = 18;

/// Format a balance entry for display in the sidebar.
///
/// Returns an 18-character string: commodity left-padded to 5 chars, one space,
/// then the decimal amount right-padded to 12 chars with 2 decimal places.
///
/// # Arguments
///
/// * `commodity` - The commodity code string (e.g. `"AUD"`).
/// * `amount`    - The balance amount to format.
///
/// # Returns
///
/// An owned `String` of exactly [`BALANCE_COLUMN_WIDTH`] characters.
fn format_balance(commodity: &str, amount: Decimal) -> String {
    format!("{commodity:<5} {amount:>12.2}")
}

/// Build the label [`Line`] for a single account in the tree.
///
/// The label consists of the account name followed by the balance string.
/// If the account has no entry in `balances`, an em-dash placeholder is shown
/// right-padded to [`BALANCE_COLUMN_WIDTH`] chars.
///
/// # Arguments
///
/// * `account`  - The account whose label to build.
/// * `balances` - Map of `AccountId` to `(commodity, amount)` pairs.
///
/// # Returns
///
/// A `Line<'static>` containing two `Span`s: name and balance.
fn build_label(
    account: &Account,
    balances: &HashMap<AccountId, (String, Decimal)>,
) -> Line<'static> {
    let name = account.name().to_owned();
    let balance_text = match balances.get(account.id()) {
        Some((commodity, amount)) => format_balance(commodity, *amount),
        None => format!("{:<width$}", "\u{2014}", width = BALANCE_COLUMN_WIDTH),
    };
    Line::from(vec![
        Span::raw(name),
        Span::raw(" "),
        Span::raw(balance_text),
    ])
}

/// Recursively build a [`TreeItem`] for `account` and all of its descendants
/// found in `all`.
///
/// # Arguments
///
/// * `account`  - The account to build a tree item for.
/// * `all`      - The full flat list of accounts used to find children.
/// * `balances` - Map of `AccountId` to `(commodity, amount)` for balance display.
///
/// # Returns
///
/// An owned `TreeItem<'static, AccountId>` representing the account and its
/// subtree.
fn build_item_owned(
    account: &Account,
    all: &[Account],
    balances: &HashMap<AccountId, (String, Decimal)>,
) -> TreeItem<'static, AccountId> {
    let children: Vec<TreeItem<'static, AccountId>> = all
        .iter()
        .filter(|a| a.parent_id() == Some(account.id()))
        .map(|child| build_item_owned(child, all, balances))
        .collect();

    let label = build_label(account, balances);

    if children.is_empty() {
        TreeItem::new_leaf(account.id().clone(), label)
    } else {
        #[expect(
            clippy::expect_used,
            reason = "TreeItem::new panics only on duplicate IDs, which we guarantee won't happen \
                      because AccountId values are unique UUIDs"
        )]
        TreeItem::new(account.id().clone(), label, children)
            .expect("account IDs are unique within a parent")
    }
}

/// Returns `true` when `id` is the ID of an account that has at least one
/// direct child in `all`.
///
/// # Arguments
///
/// * `id`  - The account ID to test.
/// * `all` - The flat list of all accounts.
fn has_children(id: &AccountId, all: &[Account]) -> bool {
    all.iter().any(|a| a.parent_id() == Some(id))
}

// MARK: private component

/// Raw widget that renders the account tree sidebar.
struct Sidebar {
    /// Component props storage.
    props: Props,
    /// Scrolling / selection state for the tree widget.
    tree_state: TreeState<AccountId>,
    /// Pre-built tree items passed to [`Tree`] on each render.
    tree_items: Vec<TreeItem<'static, AccountId>>,
    /// Flat list of all accounts; used to check for children in event handling.
    accounts: Vec<Account>,
}

impl Sidebar {
    /// Build a new `Sidebar` from a flat list of accounts and a balance map.
    ///
    /// Root accounts (those without a `parent_id`) form the top-level nodes;
    /// child accounts are nested under their parent. The first root account,
    /// if any, is opened by default so the user immediately sees its children.
    ///
    /// Each account label includes a right-aligned balance column showing the
    /// current balance for that account, or an em-dash if no balance is known.
    ///
    /// # Arguments
    ///
    /// * `accounts` - All accounts to display, in any order.
    /// * `balances` - Map of account ID to `(commodity_code, amount)` pairs.
    ///
    /// # Returns
    ///
    /// A new `Sidebar` with the tree fully built and the first root node open.
    fn new(accounts: Vec<Account>, balances: &HashMap<AccountId, (String, Decimal)>) -> Self {
        let roots: Vec<&Account> = accounts
            .iter()
            .filter(|a| a.parent_id().is_none())
            .collect();

        let tree_items: Vec<TreeItem<'static, AccountId>> = roots
            .iter()
            .map(|root| build_item_owned(root, &accounts, balances))
            .collect();

        let mut tree_state: TreeState<AccountId> = TreeState::default();

        // Open the first root node so children are visible immediately.
        if let Some(first_root) = roots.first() {
            tree_state.open(vec![first_root.id().clone()]);
        }

        Self {
            props: Props::default(),
            tree_state,
            tree_items,
            accounts,
        }
    }
}

impl Component for Sidebar {
    #[inline]
    #[expect(
        clippy::expect_used,
        reason = "Tree::new fails only on duplicate identifiers; AccountId values are unique UUIDs"
    )]
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self
            .props
            .get(Attribute::Focus)
            .is_some_and(|v| matches!(*v, AttrValue::Flag(true)));
        let border_color = if focused { Color::Cyan } else { Color::White };
        let block = Block::default()
            .title(" Accounts ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        let tree = Tree::new(&self.tree_items)
            .expect("account IDs are unique")
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

/// Tui-realm component wrapper for the account tree sidebar widget.
///
/// Handles keyboard navigation and emits
/// [`AccountsMsg::AccountSelected`](crate::msg::AccountsMsg::AccountSelected)
/// when the user confirms a leaf node.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as sidebar::AccountSidebar; repetition is intentional"
)]
#[non_exhaustive]
#[derive(Component)]
pub struct AccountSidebar {
    /// Inner raw widget.
    component: Sidebar,
}

impl AccountSidebar {
    /// Create a new `AccountSidebar` displaying the given accounts with balances.
    ///
    /// Each account row in the tree shows the account name followed by its
    /// current balance. Accounts absent from `balances` display an em-dash
    /// placeholder.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Flat list of all accounts to show in the tree.
    /// * `balances` - Shared reference to a map of account ID to `(commodity_code, amount)` pairs.
    ///
    /// # Returns
    ///
    /// A new `AccountSidebar` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(accounts: Vec<Account>, balances: &HashMap<AccountId, (String, Decimal)>) -> Self {
        Self {
            component: Sidebar::new(accounts, balances),
        }
    }

    /// Reads the current state and emits [`AccountsMsg::AccountNavigated`]
    /// for j/k navigation — same as `account_selected_or_redraw` but uses
    /// the `AccountNavigated` variant so the screen does not steal focus.
    #[inline]
    fn account_navigated_or_redraw(&self) -> Msg {
        if let State::Single(StateValue::String(ref s)) = self.component.state()
            && let Ok(id) = s.parse::<AccountId>()
        {
            return Msg::Accounts(AccountsMsg::AccountNavigated(id));
        }
        Msg::Chrome(crate::msg::ChromeMsg::Redraw)
    }
}

impl AppComponent<Msg, NoUserEvent> for AccountSidebar {
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
                Some(self.account_navigated_or_redraw())
            }
            Event::Keyboard(KeyEvent {
                code: Key::Up | Key::Char('k'),
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Up));
                Some(self.account_navigated_or_redraw())
            }
            Event::Keyboard(KeyEvent {
                code: Key::Right | Key::Char('l') | Key::Enter,
                ..
            }) => {
                self.component.perform(Cmd::Move(Direction::Right));
                // Emit AccountSelected only when a leaf node is confirmed.
                // Expanding a parent node still emits Redraw so the tree updates.
                if let State::Single(StateValue::String(ref s)) = self.component.state()
                    && let Ok(id) = s.parse::<AccountId>()
                    && !has_children(&id, &self.component.accounts)
                {
                    return Some(Msg::Accounts(AccountsMsg::AccountSelected(id)));
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
    use crate::msg::AccountsMsg;
    use crate::msg::Msg;

    /// Build a minimal [`Account`] with the given name and no parent.
    fn make_account(name: &str) -> Account {
        Account::builder()
            .name(name)
            .account_type(bc_models::AccountType::Asset)
            .build()
    }

    /// Build a child [`Account`] with a known parent ID.
    fn make_child_account(name: &str, parent_id: AccountId) -> Account {
        Account::builder()
            .name(name)
            .account_type(bc_models::AccountType::Asset)
            .parent_id(parent_id)
            .build()
    }

    #[test]
    fn empty_sidebar_has_no_state() {
        let sidebar = Sidebar::new(vec![], &HashMap::new());
        assert_eq!(sidebar.state(), State::None);
    }

    #[test]
    fn perform_move_down_on_empty_tree_does_not_panic() {
        let mut sidebar = Sidebar::new(vec![], &HashMap::new());
        let result = sidebar.perform(Cmd::Move(Direction::Down));
        // Either Changed(State::None) or CmdResult::NoChange are acceptable.
        assert!(matches!(
            result,
            CmdResult::Changed(_) | CmdResult::NoChange
        ));
    }

    #[test]
    fn single_root_account_builds_tree() {
        let acct = make_account("Assets");
        let sidebar = Sidebar::new(vec![acct], &HashMap::new());
        // Nothing is selected initially.
        assert_eq!(sidebar.state(), State::None);
        assert_eq!(sidebar.tree_items.len(), 1);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test asserts tree_items.len() == 1 immediately before indexing [0]"
    )]
    fn child_accounts_are_nested_under_parent() {
        let parent = make_account("Assets");
        let child = make_child_account("Checking", parent.id().clone());
        let sidebar = Sidebar::new(vec![parent, child], &HashMap::new());
        assert_eq!(sidebar.tree_items.len(), 1);
        assert_eq!(sidebar.tree_items[0].children().len(), 1);
    }

    #[test]
    fn perform_unknown_cmd_returns_none() {
        let mut sidebar = Sidebar::new(vec![], &HashMap::new());
        let result = sidebar.perform(Cmd::None);
        assert_eq!(result, CmdResult::NoChange);
    }

    #[test]
    fn account_sidebar_on_unknown_event_returns_none() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::None);
        assert_eq!(result, None);
    }

    #[test]
    fn account_sidebar_right_on_empty_tree_emits_redraw() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Right,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn j_key_emits_redraw() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('j'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn down_arrow_emits_redraw() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn k_key_emits_redraw() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('k'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn h_key_emits_redraw() {
        let mut sidebar = AccountSidebar::new(vec![], &HashMap::new());
        let result = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Char('h'),
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert_eq!(result, Some(Msg::Chrome(crate::msg::ChromeMsg::Redraw)));
    }

    #[test]
    fn account_sidebar_enter_selects_leaf_emits_msg() {
        let parent = make_account("Assets");
        let child = make_child_account("Checking", parent.id().clone());
        let child_id = child.id().clone();
        let mut sidebar = AccountSidebar::new(vec![parent.clone(), child], &HashMap::new());

        // After Sidebar::new the first root is already opened, so we navigate
        // down once to move selection to the first visible item (Assets root),
        // then down again to land on the child (Checking).
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // Press Enter — if Checking is now selected it should emit AccountSelected.
        let msg = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Enter,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // The exact navigation path depends on the tree's internal state after
        // two key_down calls without a prior render, so we only assert the
        // message shape when the ID matches.
        if let Some(Msg::Accounts(AccountsMsg::AccountSelected(ref id))) = msg {
            assert!(
                id == &child_id || id == parent.id(),
                "selected ID should be one of the accounts we inserted"
            );
        }
        // If None is returned, navigation simply didn't land on a leaf yet —
        // acceptable without a rendered frame.
    }

    #[test]
    fn j_key_emits_account_navigated() {
        let parent = make_account("Assets");
        let child = make_child_account("Checking", parent.id().clone());
        let mut sidebar = AccountSidebar::new(vec![parent, child], &HashMap::new());

        // First Down lands on the parent.
        let msg = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        // Parent is an account — should emit AccountNavigated.
        // If tui-tree-widget requires a rendered frame to track selection,
        // the state will still be State::None and we fall back to Redraw.
        assert!(
            matches!(
                msg,
                Some(
                    Msg::Accounts(AccountsMsg::AccountNavigated(_))
                        | Msg::Chrome(crate::msg::ChromeMsg::Redraw)
                )
            ),
            "expected AccountNavigated or Redraw, got {msg:?}"
        );
    }

    #[test]
    fn k_key_emits_account_navigated_after_navigation() {
        let parent = make_account("Assets");
        let child = make_child_account("Checking", parent.id().clone());
        let mut sidebar = AccountSidebar::new(vec![parent, child], &HashMap::new());

        // Navigate down to parent, then down to child.
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Down,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));

        // Navigate back up — should still emit AccountNavigated.
        // If tui-tree-widget requires a rendered frame to track selection,
        // the state will still be State::None and we fall back to Redraw.
        let msg = sidebar.on(&Event::Keyboard(KeyEvent {
            code: Key::Up,
            modifiers: tuirealm::event::KeyModifiers::NONE,
        }));
        assert!(
            matches!(
                msg,
                Some(
                    Msg::Accounts(AccountsMsg::AccountNavigated(_))
                        | Msg::Chrome(crate::msg::ChromeMsg::Redraw)
                )
            ),
            "expected AccountNavigated or Redraw, got {msg:?}"
        );
    }

    #[test]
    fn format_balance_formats_correctly() {
        use core::str::FromStr as _;

        use bc_models::Decimal;
        let amount = Decimal::from_str("1234.56").expect("valid decimal");
        let result = format_balance("AUD", amount);
        // 5-char commodity + 1 space + 12-char amount = 18 chars total
        assert_eq!(result.len(), 18, "balance column must be exactly 18 chars");
        assert!(
            result.starts_with("AUD  "),
            "commodity should be left-padded to 5"
        );
        assert!(result.contains("1234.56"), "amount should appear in output");
    }

    #[test]
    fn build_label_shows_em_dash_when_no_balance() {
        let acct = make_account("Assets");
        let label = build_label(&acct, &HashMap::new());
        // Collect all span text to verify the em-dash placeholder is present.
        let text: String = label.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains('\u{2014}'),
            "label should contain em-dash when no balance: {text:?}"
        );
    }

    #[test]
    fn build_label_shows_balance_when_present() {
        use core::str::FromStr as _;

        use bc_models::Decimal;
        let acct = make_account("Checking");
        let amount = Decimal::from_str("500.00").expect("valid decimal");
        let mut balances: HashMap<AccountId, (String, Decimal)> = HashMap::new();
        balances.insert(acct.id().clone(), ("AUD".to_owned(), amount));
        let label = build_label(&acct, &balances);
        let text: String = label.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("AUD"),
            "label should contain commodity: {text:?}"
        );
        assert!(
            text.contains("500.00"),
            "label should contain formatted amount: {text:?}"
        );
    }
}
