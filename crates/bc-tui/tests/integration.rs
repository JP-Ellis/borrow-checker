//! Integration tests for the borrow-checker TUI.
//!
//! Verifies that `TuiContext::open` succeeds and that each top-level screen
//! can be mounted and unmounted against a real (temporary) SQLite database.

mod common;

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use bc_tui::context::TuiContext;
    use bc_tui::id::AccountsId;
    use bc_tui::id::BudgetId;
    use bc_tui::id::Id;
    use bc_tui::id::ReportsId;
    use bc_tui::msg::Msg;
    use bc_tui::screen::Screen as _;
    use bc_tui::screen::accounts::AccountsScreen;
    use bc_tui::screen::budget::BudgetScreen;
    use bc_tui::screen::reports::ReportsScreen;
    use tuirealm::application::Application;
    use tuirealm::event::NoUserEvent;
    use tuirealm::listener::EventListenerCfg;

    fn make_app() -> Application<Id, Msg, NoUserEvent> {
        Application::init(EventListenerCfg::default())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn context_opens_with_empty_database() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        TuiContext::open(&dir.path().join("test.db"))
            .await
            .expect("TuiContext::open should succeed on a fresh database");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accounts_screen_mounts_and_unmounts() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            TuiContext::open(&dir.path().join("test.db"))
                .await
                .expect("open ctx"),
        );
        let mut app = make_app();
        let mut screen = AccountsScreen::new(Arc::clone(&ctx));
        tokio::task::block_in_place(|| {
            screen.mount(&mut app).expect("mount");
        });
        pretty_assertions::assert_eq!(app.mounted(&Id::Accounts(AccountsId::Sidebar)), true);
        pretty_assertions::assert_eq!(
            app.mounted(&Id::Accounts(AccountsId::TransactionList)),
            true
        );
        pretty_assertions::assert_eq!(
            app.mounted(&Id::Accounts(AccountsId::TransactionDetail)),
            true
        );
        screen.unmount(&mut app);
        pretty_assertions::assert_eq!(app.mounted(&Id::Accounts(AccountsId::Sidebar)), false);
        pretty_assertions::assert_eq!(
            app.mounted(&Id::Accounts(AccountsId::TransactionList)),
            false
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn budget_screen_mounts_and_unmounts() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            TuiContext::open(&dir.path().join("test.db"))
                .await
                .expect("open ctx"),
        );
        let mut app = make_app();
        let mut screen = BudgetScreen::new(Arc::clone(&ctx));
        tokio::task::block_in_place(|| {
            screen.mount(&mut app).expect("mount");
        });
        pretty_assertions::assert_eq!(app.mounted(&Id::Budget(BudgetId::Sidebar)), true);
        pretty_assertions::assert_eq!(app.mounted(&Id::Budget(BudgetId::Detail)), true);
        screen.unmount(&mut app);
        pretty_assertions::assert_eq!(app.mounted(&Id::Budget(BudgetId::Sidebar)), false);
        pretty_assertions::assert_eq!(app.mounted(&Id::Budget(BudgetId::Detail)), false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_screen_mounts_and_unmounts() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            TuiContext::open(&dir.path().join("test.db"))
                .await
                .expect("open ctx"),
        );
        let mut app = make_app();
        let mut screen = ReportsScreen::new(Arc::clone(&ctx));
        tokio::task::block_in_place(|| {
            screen.mount(&mut app).expect("mount");
        });
        pretty_assertions::assert_eq!(app.mounted(&Id::Reports(ReportsId::View)), true);
        screen.unmount(&mut app);
        pretty_assertions::assert_eq!(app.mounted(&Id::Reports(ReportsId::View)), false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seeded_database_has_accounts_and_transactions() {
        let (ctx, _dir) = super::common::seeded_context().await.expect("seed context");
        let accounts = ctx.accounts.list_active().await.expect("list accounts");
        pretty_assertions::assert_eq!(accounts.len() >= 10, true);
        let txns = ctx.transactions.list().await.expect("list transactions");
        pretty_assertions::assert_eq!(txns.len() >= 20, true);
    }
}
