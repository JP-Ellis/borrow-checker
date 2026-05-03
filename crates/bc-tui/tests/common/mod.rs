//! Shared test fixtures for bc-tui integration tests.
//!
//! Place this file at `tests/common/mod.rs` (not `tests/common.rs`) so that
//! Cargo does not treat it as a standalone test binary. Other test files pull
//! it in with `mod common;`.
//!
//! For creating a standalone test database for manual QA (CLI, TUI, GUI),
//! use the `scripts/seed-test-db.rs` cargo script:
//! ```sh
//! cargo +nightly -Zscript scripts/seed-test-db.rs --db-path ./test.db
//! ```

use std::sync::Arc;

use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Transaction;
use bc_models::TransactionId;
use bc_models::TransactionStatus;
use bc_tui::context::TuiContext;
use jiff::Timestamp;
use jiff::civil::date;
use rust_decimal_macros::dec;

/// Builds an AUD [`Amount`] from a [`Decimal`] value.
fn aud(value: Decimal) -> Amount {
    Amount::new(value, CommodityCode::new("AUD"))
}

/// Populates `ctx` with a realistic dataset covering all account types,
/// transaction statuses, and posting structures used by the app.
///
/// # Errors
///
/// Returns an error if any account or transaction creation fails.
#[expect(
    clippy::too_many_lines,
    reason = "25 transactions are needed for a realistic fixture; extracting sub-functions would obscure the dataset"
)]
pub async fn populate_test_db(ctx: &TuiContext) -> anyhow::Result<()> {
    // --- Accounts -------------------------------------------------------

    let checking_id = ctx
        .accounts
        .create()
        .name("Checking")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let savings_id = ctx
        .accounts
        .create()
        .name("Savings")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let credit_card_id = ctx
        .accounts
        .create()
        .name("CreditCard")
        .account_type(AccountType::Liability)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let salary_id = ctx
        .accounts
        .create()
        .name("Salary")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let interest_id = ctx
        .accounts
        .create()
        .name("Interest")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let groceries_id = ctx
        .accounts
        .create()
        .name("Groceries")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let dining_id = ctx
        .accounts
        .create()
        .name("Dining")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let utilities_id = ctx
        .accounts
        .create()
        .name("Utilities")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let transport_id = ctx
        .accounts
        .create()
        .name("Transport")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let subscriptions_id = ctx
        .accounts
        .create()
        .name("Subscriptions")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    // --- Transactions ---------------------------------------------------

    // 1. Paycheck — January (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 15))
                .payee("Employer Ltd")
                .description("January paycheck")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(5000.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(salary_id.clone())
                        .amount(aud(dec!(-5000.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 2. Paycheck — February (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 15))
                .payee("Employer Ltd")
                .description("February paycheck")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(5000.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(salary_id.clone())
                        .amount(aud(dec!(-5000.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 3. Credit card payment — checking → credit card liability (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 20))
                .payee("Visa")
                .description("Credit card payment")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(800.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-800.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 4. Grocery purchase #1 (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 5))
                .payee("Woolworths")
                .description("Weekly groceries")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(groceries_id.clone())
                        .amount(aud(dec!(120.50)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-120.50)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 5. Grocery purchase #2 (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 3))
                .payee("Coles")
                .description("Fortnightly groceries")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(groceries_id.clone())
                        .amount(aud(dec!(95.80)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-95.80)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 6. Dining #1 (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 8))
                .payee("The Local Bistro")
                .description("Dinner with friends")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(dining_id.clone())
                        .amount(aud(dec!(75.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-75.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 7. Dining #2 (Pending)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 10))
                .payee("Sushi Train")
                .description("Lunch")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(dining_id.clone())
                        .amount(aud(dec!(42.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-42.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 8. Dining #3 (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 20))
                .payee("Pizza Palace")
                .description("Family dinner")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(dining_id.clone())
                        .amount(aud(dec!(88.50)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-88.50)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 9. Utilities — electricity (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 12))
                .payee("AGL Energy")
                .description("January electricity bill")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(utilities_id.clone())
                        .amount(aud(dec!(210.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-210.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 10. Utilities — internet (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 12))
                .payee("Telstra")
                .description("February internet bill")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(utilities_id.clone())
                        .amount(aud(dec!(89.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-89.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 11. Transport — transit top-up (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 18))
                .payee("Opal Card")
                .description("Monthly transit top-up")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(transport_id.clone())
                        .amount(aud(dec!(50.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-50.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 12. Transport — ride share (Pending)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 18))
                .payee("Uber")
                .description("Ride to airport")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(transport_id.clone())
                        .amount(aud(dec!(35.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-35.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 13. Subscription — streaming (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 3))
                .payee("Netflix")
                .description("January streaming subscription")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(subscriptions_id.clone())
                        .amount(aud(dec!(22.99)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-22.99)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 14. Subscription — music (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 3))
                .payee("Spotify")
                .description("February music subscription")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(subscriptions_id.clone())
                        .amount(aud(dec!(12.99)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-12.99)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 15. Savings transfer — January (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 25))
                .description("Monthly savings transfer")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(savings_id.clone())
                        .amount(aud(dec!(1000.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-1000.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 16. Interest income — January (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 31))
                .payee("Bank")
                .description("January savings interest")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(savings_id.clone())
                        .amount(aud(dec!(8.75)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(interest_id.clone())
                        .amount(aud(dec!(-8.75)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 17. Extra grocery run (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 22))
                .payee("IGA")
                .description("Top-up groceries")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(groceries_id.clone())
                        .amount(aud(dec!(45.30)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-45.30)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 18. Dining — coffee (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 28))
                .payee("The Coffee Club")
                .description("Morning coffee")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(dining_id.clone())
                        .amount(aud(dec!(6.50)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-6.50)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 19. Transport — petrol (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 5))
                .payee("BP Service Station")
                .description("Petrol fill-up")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(transport_id.clone())
                        .amount(aud(dec!(85.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-85.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 20. Utilities — gas bill (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 14))
                .payee("Origin Energy")
                .description("February gas bill")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(utilities_id.clone())
                        .amount(aud(dec!(130.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-130.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 21. Subscription — cloud storage (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 10))
                .payee("iCloud")
                .description("iCloud storage subscription")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(subscriptions_id.clone())
                        .amount(aud(dec!(4.49)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-4.49)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 22. Savings transfer — February (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 25))
                .description("February savings transfer")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(savings_id.clone())
                        .amount(aud(dec!(1200.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(-1200.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 23. Interest income — February (Cleared)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 28))
                .payee("Bank")
                .description("February savings interest")
                .status(TransactionStatus::Cleared)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(savings_id.clone())
                        .amount(aud(dec!(12.50)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(interest_id.clone())
                        .amount(aud(dec!(-12.50)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 24. Grocery purchase — pending (Pending)
    ctx.transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 2, 27))
                .payee("Harris Farm")
                .description("Organic groceries")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(groceries_id.clone())
                        .amount(aud(dec!(67.20)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(credit_card_id.clone())
                        .amount(aud(dec!(-67.20)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    // 25. Voided transaction — duplicate entry that gets voided.
    let voided_id = ctx
        .transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(date(2026, 1, 15))
                .payee("Employer Ltd")
                .description("Duplicate paycheck — to be voided")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking_id.clone())
                        .amount(aud(dec!(5000.00)))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(salary_id.clone())
                        .amount(aud(dec!(-5000.00)))
                        .build(),
                ])
                .build(),
        )
        .await?;

    ctx.transactions.void(&voided_id).await?;

    Ok(())
}

/// Creates a [`TuiContext`] backed by a temporary database pre-populated
/// with test data.
///
/// Returns both the context and the temp directory handle. The caller must
/// keep the `TempDir` alive for the duration of the test — dropping it deletes
/// the on-disk database.
///
/// # Errors
///
/// Returns an error if the temp directory or database cannot be created, or if
/// [`populate_test_db`] fails.
pub async fn seeded_context() -> anyhow::Result<(Arc<TuiContext>, assert_fs::TempDir)> {
    let dir = assert_fs::TempDir::new()?;
    let ctx = Arc::new(TuiContext::open(&dir.path().join("test.db")).await?);
    populate_test_db(&ctx).await?;
    Ok((ctx, dir))
}
