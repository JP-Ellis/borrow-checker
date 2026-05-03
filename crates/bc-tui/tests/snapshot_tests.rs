//! Snapshot tests for `bc-tui` components.
//!
//! Renders each public TUI component to a [`tuirealm::ratatui::backend::TestBackend`]
//! buffer and asserts the text output against a stored snapshot.  Run with
//! `INSTA_UPDATE=new cargo nextest run -p bc-tui --test snapshot_tests` to
//! (re-)generate snapshot files.

// MARK: render helper

/// Render `component` into a `width × height` [`tuirealm::ratatui::backend::TestBackend`]
/// buffer and return the visible text as a trimmed multi-line string.
///
/// Trailing whitespace is stripped from each row so that snapshot files remain
/// compact and free of invisible differences.
///
/// # Arguments
///
/// * `component` - Any type implementing [`tuirealm::MockComponent`].
/// * `width`     - Terminal width in columns.
/// * `height`    - Terminal height in rows.
///
/// # Returns
///
/// A `String` where each line corresponds to one row of the rendered buffer,
/// joined by `'\n'`.
#[expect(
    clippy::expect_used,
    reason = "test helper: panicking on terminal/draw failure is correct behaviour in tests"
)]
fn render<C: tuirealm::MockComponent>(component: &mut C, width: u16, height: u16) -> String {
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;
    use tuirealm::ratatui::layout::Position;
    use tuirealm::ratatui::layout::Rect;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| {
            component.view(frame, Rect::new(0, 0, width, height));
        })
        .expect("draw component");
    let buffer = terminal.backend().buffer().clone();
    let area = *buffer.area();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer.cell(Position { x, y }).map_or(" ", |c| c.symbol()))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// MARK: transaction helpers

/// Build a minimal [`bc_models::Transaction`] with the given description,
/// payee, and date.
///
/// # Arguments
///
/// * `desc`     - Transaction description (may be empty).
/// * `payee`    - Payee name; stored as `Some(payee)` when non-empty, `None` otherwise.
/// * `date_ymd` - `(year, month, day)` tuple for the transaction date.
///
/// # Returns
///
/// A new `Transaction` with `TransactionStatus::Cleared` and a freshly
/// generated ID.
fn make_transaction(desc: &str, payee: &str, date_ymd: (i16, i8, i8)) -> bc_models::Transaction {
    bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(jiff::civil::date(date_ymd.0, date_ymd.1, date_ymd.2))
        .maybe_payee(if payee.is_empty() {
            None
        } else {
            Some(payee.to_owned())
        })
        .description(desc.to_owned())
        .status(bc_models::TransactionStatus::Cleared)
        .created_at(jiff::Timestamp::UNIX_EPOCH)
        .build()
}

// MARK: account helpers

/// Build a minimal [`bc_models::Account`] with the given name and type.
///
/// # Arguments
///
/// * `name`         - Account display name.
/// * `account_type` - The [`bc_models::AccountType`] classification.
///
/// # Returns
///
/// A new `Account` with a freshly generated ID and no parent.
fn make_account(name: &str, account_type: bc_models::AccountType) -> bc_models::Account {
    bc_models::Account::builder()
        .name(name)
        .account_type(account_type)
        .build()
}

// MARK: snapshot tests

#[cfg(test)]
mod tests {
    use bc_tui::screen::accounts::list::TransactionList;
    use bc_tui::screen::accounts::sidebar::AccountSidebar;

    use super::make_account;
    use super::make_transaction;
    use super::render;

    // MARK: TransactionList

    /// Snapshot: `TransactionList` rendered with no transactions.
    #[test]
    fn transaction_list_empty() {
        let mut component = TransactionList::new(vec![]);
        let output = render(&mut component, 60, 15);
        insta::assert_snapshot!(output);
    }

    /// Snapshot: `TransactionList` rendered with three hardcoded transactions.
    ///
    /// UUIDs in the output (if any) are redacted to `<uuid>` so snapshots remain
    /// stable across test runs.
    #[test]
    fn transaction_list_with_data() {
        let txns = vec![
            make_transaction("Groceries", "Supermarket", (2026, 1, 10)),
            make_transaction("Rent", "Landlord", (2026, 1, 1)),
            make_transaction("Coffee", "Café", (2026, 1, 15)),
        ];
        let mut component = TransactionList::new(txns);
        let output = render(&mut component, 60, 15);
        insta::with_settings!({
            filters => [("[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}", "<uuid>")]
        }, {
            insta::assert_snapshot!(output);
        });
    }

    // MARK: AccountSidebar

    /// Snapshot: `AccountSidebar` rendered with no accounts.
    #[test]
    fn account_sidebar_empty() {
        let mut component = AccountSidebar::new(vec![]);
        let output = render(&mut component, 25, 20);
        insta::assert_snapshot!(output);
    }

    /// Snapshot: `AccountSidebar` rendered with three root accounts.
    #[test]
    fn account_sidebar_with_accounts() {
        let accounts = vec![
            make_account("Assets", bc_models::AccountType::Asset),
            make_account("Liabilities", bc_models::AccountType::Liability),
            make_account("Expenses", bc_models::AccountType::Expense),
        ];
        let mut component = AccountSidebar::new(accounts);
        let output = render(&mut component, 25, 20);
        insta::assert_snapshot!(output);
    }
}
