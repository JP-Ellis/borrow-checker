//! Integration tests for the borrow-checker TUI.
//!
//! Verifies that `TuiContext::open` succeeds and that each top-level screen
//! can be mounted and unmounted against a real (temporary) SQLite database.

#[cfg(test)]
mod tests {
    use core::time::Duration;
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
    use tuirealm::Application;
    use tuirealm::EventListenerCfg;
    use tuirealm::NoUserEvent;

    fn make_app() -> Application<Id, Msg, NoUserEvent> {
        Application::init(EventListenerCfg::default().poll_timeout(Duration::from_millis(10)))
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
}
