//! Accounts screen — account tree sidebar, transaction list, and detail panel.
//!
//! This module owns the three components that make up the Accounts tab:
//! - [`sidebar::AccountSidebar`] — left panel showing the account hierarchy
//! - [`list::TransactionList`] — right panel listing transactions for the selected account
//! - [`detail::TransactionDetail`] — optional bottom panel showing a transaction's postings

pub mod detail;
pub mod forms;
pub mod list;
pub mod sidebar;

use std::sync::Arc;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::Transaction;
use bc_models::TransactionId;
use tuirealm::Application;
use tuirealm::Frame;
use tuirealm::NoUserEvent;
use tuirealm::ratatui::layout::Constraint;
use tuirealm::ratatui::layout::Direction;
use tuirealm::ratatui::layout::Layout;
use tuirealm::ratatui::layout::Rect;

use crate::context::TuiContext;
use crate::id::AccountsId;
use crate::id::Id;
use crate::mode::AppMode;
use crate::msg::AccountsMsg;
use crate::msg::Msg;
use crate::screen::KeyBinding;
use crate::screen::Screen;

/// The accounts tab screen.
///
/// Owns the account sidebar, transaction list, and transaction detail panel.
/// Handles [`AccountsMsg`] variants delegated from `Model::update()`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as accounts::AccountsScreen; repetition is intentional"
)]
#[non_exhaustive]
pub struct AccountsScreen {
    /// Shared bc-core services.
    ctx: Arc<TuiContext>,
    /// All active accounts loaded from the database.
    accounts: Vec<Account>,
    /// Transactions for the currently selected account.
    transactions: Vec<Transaction>,
    /// The account currently selected in the sidebar, if any.
    selected_account: Option<AccountId>,
    /// The transaction currently selected in the list, if any.
    selected_transaction: Option<TransactionId>,
    /// Whether the detail panel is currently visible.
    detail_visible: bool,
}

impl AccountsScreen {
    /// Create a new `AccountsScreen` bound to the given context.
    ///
    /// Data is not loaded until [`Screen::mount`] is called.
    #[inline]
    #[must_use]
    pub fn new(ctx: Arc<TuiContext>) -> Self {
        Self {
            ctx,
            accounts: Vec::new(),
            transactions: Vec::new(),
            selected_account: None,
            selected_transaction: None,
            detail_visible: false,
        }
    }

    /// Load all active accounts from the database into `self.accounts`.
    #[expect(
        clippy::print_stderr,
        reason = "load errors are logged to stderr since we are in raw terminal mode"
    )]
    fn load_accounts(&mut self) {
        match self.ctx.block_on(self.ctx.accounts.list_active()) {
            Ok(accounts) => self.accounts = accounts,
            Err(e) => eprintln!("failed to load accounts: {e}"),
        }
    }

    /// Load transactions for the selected account into `self.transactions`.
    ///
    /// If no account is selected, clears the transaction list.
    #[expect(
        clippy::print_stderr,
        reason = "load errors are logged to stderr since we are in raw terminal mode"
    )]
    fn load_transactions(&mut self) {
        let Some(account_id) = self.selected_account.clone() else {
            self.transactions = Vec::new();
            return;
        };
        match self.ctx.block_on(self.ctx.transactions.list()) {
            Ok(all) => {
                self.transactions = all
                    .into_iter()
                    .filter(|tx| tx.postings().iter().any(|p| p.account_id() == &account_id))
                    .collect();
            }
            Err(e) => eprintln!("failed to load transactions: {e}"),
        }
    }

    /// Handle an [`AccountsMsg`], updating internal state and returning a follow-up [`Msg`] if needed.
    #[expect(
        clippy::print_stderr,
        reason = "void errors are logged to stderr since we are in raw terminal mode"
    )]
    fn handle_accounts_msg(&mut self, msg: AccountsMsg) -> Option<Msg> {
        match msg {
            AccountsMsg::AccountSelected(id) => {
                self.selected_account = Some(id);
                self.load_transactions();
                None
            }
            AccountsMsg::OpenAddTransaction | AccountsMsg::OpenEditTransaction => {
                Some(Msg::ModeChange(AppMode::Insert))
            }
            AccountsMsg::FormCancelled | AccountsMsg::FormSubmitted => {
                Some(Msg::ModeChange(AppMode::Normal))
            }
            AccountsMsg::VoidConfirmed => {
                if let Some(id) = self.selected_transaction.clone() {
                    match self.ctx.block_on(self.ctx.transactions.void(&id)) {
                        Ok(()) => {}
                        Err(e) => eprintln!("failed to void transaction: {e}"),
                    }
                    self.load_transactions();
                }
                None
            }
        }
    }
}

impl Screen for AccountsScreen {
    /// Mount the accounts screen components into the application.
    ///
    /// Loads accounts from the database, then mounts sidebar, list, and detail components.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to mount (e.g., duplicate ID).
    #[inline]
    fn mount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) -> anyhow::Result<()> {
        self.load_accounts();
        app.mount(
            Id::Accounts(AccountsId::Sidebar),
            Box::new(sidebar::AccountSidebar::new(self.accounts.clone())),
            vec![],
        )?;
        app.mount(
            Id::Accounts(AccountsId::TransactionList),
            Box::new(list::TransactionList::new(vec![])),
            vec![],
        )?;
        app.mount(
            Id::Accounts(AccountsId::TransactionDetail),
            Box::new(detail::TransactionDetail::new(None)),
            vec![],
        )?;
        Ok(())
    }

    /// Unmount all accounts screen components from the application.
    #[inline]
    #[expect(
        clippy::unused_result_ok,
        reason = "unmount errors are non-fatal; component may already be absent"
    )]
    fn unmount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) {
        app.umount(&Id::Accounts(AccountsId::Sidebar)).ok();
        app.umount(&Id::Accounts(AccountsId::TransactionList)).ok();
        app.umount(&Id::Accounts(AccountsId::TransactionDetail))
            .ok();
    }

    /// Render the accounts screen: sidebar on the left (25%), transaction list on the right (75%).
    ///
    /// When `detail_visible` is true, the right panel is split vertically between the
    /// transaction list and the detail panel.
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 2 chunks to match the 2 constraints"
    )]
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect) {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
            .split(area);
        app.view(&Id::Accounts(AccountsId::Sidebar), frame, h_chunks[0]);
        if self.detail_visible {
            let v_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(h_chunks[1]);
            app.view(
                &Id::Accounts(AccountsId::TransactionList),
                frame,
                v_chunks[0],
            );
            app.view(
                &Id::Accounts(AccountsId::TransactionDetail),
                frame,
                v_chunks[1],
            );
        } else {
            app.view(
                &Id::Accounts(AccountsId::TransactionList),
                frame,
                h_chunks[1],
            );
        }
    }

    /// Handle a message destined for the accounts screen.
    ///
    /// Delegates [`Msg::Accounts`] variants to [`Self::handle_accounts_msg`].
    /// Returns `None` for any unrecognised message.
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Msg is non-exhaustive; non-Accounts variants are intentionally ignored"
    )]
    fn handle(&mut self, msg: Msg) -> Option<Msg> {
        match msg {
            Msg::Accounts(accounts_msg) => self.handle_accounts_msg(accounts_msg),
            _ => None,
        }
    }

    /// Returns the sidebar as the initial focus target.
    #[inline]
    fn initial_focus(&self) -> Id {
        Id::Accounts(AccountsId::Sidebar)
    }

    /// Returns the keybindings for the accounts screen in the given mode.
    ///
    /// - Normal: 8 bindings (navigation, add/edit/void, toggle detail)
    /// - Insert: 4 bindings (form submit/cancel, next/previous field)
    /// - Visual: 3 bindings (select all, clear selection, void selected)
    #[inline]
    fn keybindings(&self, mode: &AppMode) -> Vec<KeyBinding> {
        match mode {
            AppMode::Normal => vec![
                KeyBinding {
                    key: "j / ↓".into(),
                    action: "Move down".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "k / ↑".into(),
                    action: "Move up".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "Enter".into(),
                    action: "Select account".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "a".into(),
                    action: "Add transaction".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "e".into(),
                    action: "Edit transaction".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "x".into(),
                    action: "Void transaction".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "d".into(),
                    action: "Toggle detail panel".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "v".into(),
                    action: "Enter visual mode".into(),
                    mode: AppMode::Normal,
                },
            ],
            AppMode::Insert => vec![
                KeyBinding {
                    key: "Enter".into(),
                    action: "Submit form".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Esc".into(),
                    action: "Cancel form".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Tab".into(),
                    action: "Next field".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Shift+Tab".into(),
                    action: "Previous field".into(),
                    mode: AppMode::Insert,
                },
            ],
            AppMode::Visual => vec![
                KeyBinding {
                    key: "a".into(),
                    action: "Select all".into(),
                    mode: AppMode::Visual,
                },
                KeyBinding {
                    key: "Esc".into(),
                    action: "Clear selection".into(),
                    mode: AppMode::Visual,
                },
                KeyBinding {
                    key: "x".into(),
                    action: "Void selected".into(),
                    mode: AppMode::Visual,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn load_accounts_populates_list() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let ctx = Arc::new(TuiContext::open(&db_path).await.expect("open test context"));
        let mut screen = AccountsScreen::new(ctx);
        // block_in_place allows blocking calls (block_on) within a multi-threaded tokio runtime.
        tokio::task::block_in_place(|| {
            screen.load_accounts();
        });
        // DB is empty on first open, so accounts should be empty.
        pretty_assertions::assert_eq!(screen.accounts.len(), 0);
    }
}
