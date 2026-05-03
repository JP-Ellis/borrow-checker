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

/// Which form overlay, if any, should be mounted on the next render.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PendingForm {
    /// No pending form action.
    #[default]
    None,
    /// Mount the "add transaction" form.
    Add,
    /// Mount the "edit transaction" form pre-filled with the selected transaction.
    Edit,
}

/// The accounts tab screen.
///
/// Owns the account sidebar, transaction list, and transaction detail panel.
/// Handles [`AccountsMsg`] variants delegated from `Model::update()`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as accounts::AccountsScreen; repetition is intentional"
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "five independent state flags: loading, list/detail dirty tracking, form state — not reducible to an enum"
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
    /// Whether accounts are currently being loaded from the database.
    loading: bool,
    /// Whether the transaction list needs to be re-mounted on the next `view()` call.
    list_dirty: bool,
    /// Whether the detail panel needs to be re-mounted on the next `view()` call.
    detail_dirty: bool,
    /// Pending form action to execute on the next `view()` call.
    pending_form: PendingForm,
    /// Whether the transaction form is currently mounted.
    form_mounted: bool,
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
            loading: false,
            list_dirty: false,
            detail_dirty: false,
            pending_form: PendingForm::None,
            form_mounted: false,
        }
    }

    /// Load all active accounts from the database into `self.accounts`.
    #[inline]
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
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "load errors are logged to stderr since we are in raw terminal mode"
    )]
    fn load_transactions(&mut self) {
        let Some(account_id) = self.selected_account.clone() else {
            self.transactions = Vec::new();
            return;
        };
        match self
            .ctx
            .block_on(self.ctx.transactions.list_for_account(&account_id))
        {
            Ok(txns) => self.transactions = txns.collect(),
            Err(e) => eprintln!("failed to load transactions: {e}"),
        }
    }

    /// Parse form field strings and create or amend a transaction via bc-core.
    ///
    /// If `is_edit` is `true`, the currently selected transaction is amended; otherwise
    /// a new transaction is created. Errors are logged to stderr — no attempt is made
    /// to surface them in the UI.
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "form errors are logged to stderr since we are in raw terminal mode"
    )]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "negating a Decimal value to produce the debit posting; overflow is not a concern for financial amounts"
    )]
    fn save_transaction(
        &mut self,
        date_str: &str,
        payee: &str,
        amount_str: &str,
        account_str: &str,
        is_edit: bool,
    ) {
        let date = match date_str.parse::<jiff::civil::Date>() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("invalid date '{date_str}': {e}");
                return;
            }
        };
        let Some((value_str, commodity_str)) = amount_str.rsplit_once(' ') else {
            eprintln!("invalid amount '{amount_str}': expected 'VALUE COMMODITY'");
            return;
        };
        let value = match value_str.parse::<bc_models::Decimal>() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("invalid amount value '{value_str}': {e}");
                return;
            }
        };
        let commodity = bc_models::CommodityCode::new(commodity_str);
        let counterpart_id = match account_str.parse::<bc_models::AccountId>() {
            Ok(id) => id,
            Err(e) => {
                eprintln!("invalid account id '{account_str}': {e}");
                return;
            }
        };
        let Some(ref account_id) = self.selected_account else {
            eprintln!("no account selected");
            return;
        };
        let debit = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id.clone())
            .amount(bc_models::Amount::new(-value, commodity.clone()))
            .build();
        let credit = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(counterpart_id)
            .amount(bc_models::Amount::new(value, commodity))
            .build();
        let maybe_payee: Option<String> = if payee.is_empty() {
            None
        } else {
            Some(payee.to_owned())
        };
        if is_edit {
            let Some(ref tx_id) = self.selected_transaction else {
                eprintln!("no transaction selected for edit");
                return;
            };
            let Some(existing) = self.transactions.iter().find(|t| t.id() == tx_id).cloned() else {
                eprintln!("selected transaction not found in cache");
                return;
            };
            let updated = bc_models::Transaction::builder()
                .id(tx_id.clone())
                .date(date)
                .maybe_payee(maybe_payee)
                .description("")
                .postings(vec![debit, credit])
                .status(bc_models::TransactionStatus::Cleared)
                .created_at(*existing.created_at())
                .build();
            match self.ctx.block_on(self.ctx.transactions.amend(updated)) {
                Ok(()) => {}
                Err(e) => eprintln!("failed to amend transaction: {e}"),
            }
        } else {
            let new_tx = bc_models::Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(date)
                .maybe_payee(maybe_payee)
                .description("")
                .postings(vec![debit, credit])
                .status(bc_models::TransactionStatus::Cleared)
                .created_at(jiff::Timestamp::now())
                .build();
            match self.ctx.block_on(self.ctx.transactions.create(new_tx)) {
                Ok(_id) => {}
                Err(e) => eprintln!("failed to create transaction: {e}"),
            }
        }
    }

    /// Handle an [`AccountsMsg`], updating internal state and returning a follow-up [`Msg`] if needed.
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "void errors are logged to stderr since we are in raw terminal mode"
    )]
    fn handle_accounts_msg(&mut self, msg: AccountsMsg) -> Option<Msg> {
        match msg {
            AccountsMsg::AccountSelected(id) => {
                self.selected_account = Some(id);
                self.selected_transaction = None;
                self.detail_visible = false;
                self.load_transactions();
                self.list_dirty = true;
                None
            }
            AccountsMsg::OpenAddTransaction => {
                self.pending_form = PendingForm::Add;
                Some(Msg::ModeChange(AppMode::Insert))
            }
            AccountsMsg::OpenEditTransaction(id) => {
                self.selected_transaction = Some(id);
                self.pending_form = PendingForm::Edit;
                Some(Msg::ModeChange(AppMode::Insert))
            }
            AccountsMsg::FormCancelled => {
                self.pending_form = PendingForm::None;
                // form_mounted stays true — view() will unmount and clear it
                Some(Msg::ModeChange(AppMode::Normal))
            }
            AccountsMsg::FormSubmitted {
                date,
                payee,
                amount,
                account,
            } => {
                let is_edit = self.pending_form == PendingForm::Edit;
                self.pending_form = PendingForm::None;
                // form_mounted stays true — view() will unmount and clear it
                self.save_transaction(&date, &payee, &amount, &account, is_edit);
                self.load_transactions();
                self.list_dirty = true;
                Some(Msg::ModeChange(AppMode::Normal))
            }
            AccountsMsg::VoidRequested(id) => {
                match self.ctx.block_on(self.ctx.transactions.void(&id)) {
                    Ok(()) => {}
                    Err(e) => eprintln!("failed to void transaction: {e}"),
                }
                self.load_transactions();
                self.list_dirty = true;
                None
            }
            AccountsMsg::OpenDetail(id) => {
                self.selected_transaction = Some(id);
                self.detail_visible = true;
                self.detail_dirty = true;
                Some(Msg::FocusChange(Id::Accounts(
                    AccountsId::TransactionDetail,
                )))
            }
            AccountsMsg::CloseDetail => {
                self.detail_visible = false;
                Some(Msg::FocusChange(Id::Accounts(AccountsId::TransactionList)))
            }
            AccountsMsg::FocusSidebar => Some(Msg::FocusChange(Id::Accounts(AccountsId::Sidebar))),
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
        self.loading = true;
        self.load_accounts();
        self.loading = false;
        app.mount(
            Id::Accounts(AccountsId::Sidebar),
            Box::new(sidebar::AccountSidebar::new(self.accounts.clone())),
            vec![],
        )?;
        app.mount(
            Id::Accounts(AccountsId::TransactionList),
            Box::new(list::TransactionList::new(
                vec![],
                None,
                bc_models::Decimal::ZERO,
                String::new(),
            )),
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
        app.umount(&Id::Accounts(AccountsId::TransactionForm)).ok();
    }

    /// Render the accounts screen: sidebar on the left (25%), transaction list on the right (75%).
    ///
    /// When `detail_visible` is true, the right panel is split vertically between the
    /// transaction list and the detail panel.
    ///
    /// Form overlays are mounted/unmounted here based on `pending_form` state, and
    /// rendered on top of the rest of the screen when `form_mounted` is true.
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 2 chunks to match the 2 constraints"
    )]
    #[expect(
        clippy::print_stderr,
        reason = "mount errors logged to stderr since we are in raw terminal mode"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "view() handles form overlay, dirty-flag re-mounts, and layout in one pass — splitting would obscure the rendering order"
    )]
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect) {
        // Mount or unmount the form overlay based on pending_form state.
        match &self.pending_form {
            PendingForm::Add if !self.form_mounted => {
                // Pre-unmount guards against a stale component from a previous failed cleanup.
                #[expect(
                    clippy::unused_result_ok,
                    reason = "pre-unmount is best-effort; component may not be present"
                )]
                {
                    app.umount(&Id::Accounts(AccountsId::TransactionForm)).ok();
                }
                match app.mount(
                    Id::Accounts(AccountsId::TransactionForm),
                    Box::new(forms::TransactionForm::new_add()),
                    vec![],
                ) {
                    Ok(()) => {
                        #[expect(
                            clippy::unused_result_ok,
                            reason = "focus is best-effort; form is still visible if active() fails"
                        )]
                        {
                            app.active(&Id::Accounts(AccountsId::TransactionForm)).ok();
                        }
                        self.form_mounted = true;
                    }
                    Err(e) => {
                        eprintln!("failed to mount transaction form: {e}");
                        self.pending_form = PendingForm::None;
                    }
                }
            }
            PendingForm::Edit if !self.form_mounted => {
                let pending_tx = self
                    .selected_transaction
                    .as_ref()
                    .and_then(|id| self.transactions.iter().find(|t| t.id() == id))
                    .cloned();
                match pending_tx {
                    Some(tx) => {
                        #[expect(
                            clippy::unused_result_ok,
                            reason = "pre-unmount is best-effort; component may not be present"
                        )]
                        {
                            app.umount(&Id::Accounts(AccountsId::TransactionForm)).ok();
                        }
                        match app.mount(
                            Id::Accounts(AccountsId::TransactionForm),
                            Box::new(forms::TransactionForm::new_edit(&tx)),
                            vec![],
                        ) {
                            Ok(()) => {
                                #[expect(
                                    clippy::unused_result_ok,
                                    reason = "focus is best-effort; form is still visible if active() fails"
                                )]
                                {
                                    app.active(&Id::Accounts(AccountsId::TransactionForm)).ok();
                                }
                                self.form_mounted = true;
                            }
                            Err(e) => {
                                eprintln!("failed to mount transaction form: {e}");
                                self.pending_form = PendingForm::None;
                            }
                        }
                    }
                    None => {
                        // Transaction no longer available — abort silently.
                        self.pending_form = PendingForm::None;
                    }
                }
            }
            PendingForm::None if self.form_mounted => {
                #[expect(
                    clippy::unused_result_ok,
                    reason = "unmount errors are non-fatal; component may already be absent"
                )]
                {
                    app.umount(&Id::Accounts(AccountsId::TransactionForm)).ok();
                }
                self.form_mounted = false;
            }
            PendingForm::None | PendingForm::Add | PendingForm::Edit => {}
        }

        if self.loading {
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new("Loading accounts\u{2026}"),
                area,
            );
            return;
        }

        if self.list_dirty {
            self.list_dirty = false;
            #[expect(
                clippy::unused_result_ok,
                reason = "umount errors are non-fatal; component may already be absent"
            )]
            {
                app.umount(&Id::Accounts(AccountsId::TransactionList)).ok();
            }
            if let Err(e) = app.mount(
                Id::Accounts(AccountsId::TransactionList),
                Box::new(list::TransactionList::new(
                    self.transactions.clone(),
                    None,
                    bc_models::Decimal::ZERO,
                    String::new(),
                )),
                vec![],
            ) {
                eprintln!("failed to re-mount transaction list: {e}");
            }
            // Restore focus to the list after remount unless detail or form is in front.
            if !self.detail_visible && !self.form_mounted {
                #[expect(
                    clippy::unused_result_ok,
                    reason = "focus restore is best-effort after re-mount"
                )]
                {
                    app.active(&Id::Accounts(AccountsId::TransactionList)).ok();
                }
            }
        }

        if self.detail_dirty {
            self.detail_dirty = false;
            let tx = self
                .selected_transaction
                .as_ref()
                .and_then(|id| self.transactions.iter().find(|t| t.id() == id))
                .cloned();
            #[expect(
                clippy::unused_result_ok,
                reason = "umount errors are non-fatal; component may already be absent"
            )]
            {
                app.umount(&Id::Accounts(AccountsId::TransactionDetail))
                    .ok();
            }
            if let Err(e) = app.mount(
                Id::Accounts(AccountsId::TransactionDetail),
                Box::new(detail::TransactionDetail::new(tx)),
                vec![],
            ) {
                eprintln!("failed to re-mount transaction detail: {e}");
            }
        }

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

        // Render the form overlay on top if mounted.
        if self.form_mounted {
            app.view(&Id::Accounts(AccountsId::TransactionForm), frame, area);
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
                    key: "l / Enter".into(),
                    action: "Open detail panel".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "h / ←".into(),
                    action: "Focus sidebar".into(),
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
                    key: "d".into(),
                    action: "Void transaction".into(),
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

    #[test]
    fn open_edit_returns_mode_change_insert() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        );
        let mut screen = AccountsScreen::new(ctx);
        // The transaction ID is now passed in the message; always enters Insert mode.
        let id = bc_models::TransactionId::new();
        let result = screen.handle(Msg::Accounts(AccountsMsg::OpenEditTransaction(id)));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Insert)));
    }

    #[test]
    fn open_add_returns_mode_change_insert() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        );
        let mut screen = AccountsScreen::new(ctx);
        let result = screen.handle(Msg::Accounts(AccountsMsg::OpenAddTransaction));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Insert)));
    }

    #[test]
    fn form_cancelled_returns_mode_change_normal() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        );
        let mut screen = AccountsScreen::new(ctx);
        let result = screen.handle(Msg::Accounts(AccountsMsg::FormCancelled));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Normal)));
    }

    #[test]
    fn form_submitted_returns_mode_change_normal() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        );
        let mut screen = AccountsScreen::new(ctx);
        // Invalid field values are logged to stderr; the mode change still returns.
        let result = screen.handle(Msg::Accounts(AccountsMsg::FormSubmitted {
            date: String::new(),
            payee: String::new(),
            amount: String::new(),
            account: String::new(),
        }));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Normal)));
    }

    #[test]
    fn non_accounts_msg_returns_none() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        );
        let mut screen = AccountsScreen::new(ctx);
        pretty_assertions::assert_eq!(screen.handle(Msg::AppQuit), None);
    }
}
