//! Seed a SQLite database with a realistic 6-month dataset for E2E tests.
//!
//! Creates a full account hierarchy, account-anchored budgets with allocations,
//! and ~79 transactions covering 6 historical months plus the current month.
//!
//! Usage:
//!   bc-seed [--db-path <PATH>] [--force].

#![expect(
    clippy::print_stdout,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    reason = "test fixture seeding binary; not a public library"
)]

use std::path::PathBuf;

use bc_core::AccountService;
use bc_core::BudgetService;
use bc_core::TransactionService;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::Amount;
use bc_models::BudgetWindow;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Period;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::RolloverPolicy;
use bc_models::Transaction;
use bc_models::TransactionId;
use bc_models::TransactionStatus;
use clap::Parser;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal_macros::dec;

#[derive(Parser)]
#[command(
    name = "bc-seed",
    about = "Seed a SQLite database with realistic test fixture data"
)]
/// CLI arguments for the seed binary.
struct Args {
    /// Path where the database file will be written.
    #[arg(long, default_value = "./borrow-checker-test.db")]
    db_path: PathBuf,

    /// Overwrite the database file if it already exists.
    #[arg(long)]
    force: bool,
}

/// Constructs an AUD [`Amount`] from a decimal value.
fn aud(value: Decimal) -> Amount {
    Amount::new(value, CommodityCode::new("AUD"))
}

/// Returns the first day of the calendar month `months_ago` months before
/// the current wall-clock date (intercepted by libfaketime in CI).
fn month_start(months_ago: i64) -> Date {
    let today = jiff::Zoned::now().date();
    let approx = today.saturating_sub(jiff::Span::new().months(months_ago));
    BudgetWindow::this_month(approx).start
}

/// Returns a specific day within the month that is `months_ago` months before today.
///
/// `day` is 1-based.
fn month_day(months_ago: i64, day: i8) -> Date {
    month_start(months_ago).saturating_add(jiff::Span::new().days(i64::from(day) - 1))
}

/// Constructs a [`Posting`] with a new random ID.
fn posting(account_id: &AccountId, amount: Amount) -> Posting {
    Posting::builder()
        .id(PostingId::new())
        .account_id(account_id.clone())
        .amount(amount)
        .build()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.db_path.exists() && !args.force {
        anyhow::bail!(
            "database already exists at '{}'. Use --force to overwrite.",
            args.db_path.display()
        );
    }

    if args.db_path.exists() && args.force {
        std::fs::remove_file(&args.db_path)?;
    }

    let pool = bc_core::open_db_at(&args.db_path).await?;
    let accounts = AccountService::new(pool.clone());
    let budgets = BudgetService::new(pool.clone());
    let transactions = TransactionService::new(pool.clone());

    // =========================================================================
    // ACCOUNTS (26 total: 5 root + 21 leaf)
    // =========================================================================

    let assets_id = accounts
        .create()
        .name("Assets")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let liabilities_id = accounts
        .create()
        .name("Liabilities")
        .account_type(AccountType::Liability)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let equity_id = accounts
        .create()
        .name("Equity")
        .account_type(AccountType::Equity)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let income_id = accounts
        .create()
        .name("Income")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let expenses_id = accounts
        .create()
        .name("Expenses")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let checking_id = accounts
        .create()
        .name("Checking")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .parent_id(&assets_id)
        .call()
        .await?;

    let savings_id = accounts
        .create()
        .name("Savings")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .parent_id(&assets_id)
        .call()
        .await?;

    let _car_id = accounts
        .create()
        .name("Car")
        .account_type(AccountType::Asset)
        .kind(AccountKind::ManualAsset)
        .parent_id(&assets_id)
        .call()
        .await?;

    let credit_card_id = accounts
        .create()
        .name("CreditCard")
        .account_type(AccountType::Liability)
        .kind(AccountKind::DepositAccount)
        .parent_id(&liabilities_id)
        .call()
        .await?;

    let car_loan_id = accounts
        .create()
        .name("CarLoan")
        .account_type(AccountType::Liability)
        .kind(AccountKind::DepositAccount)
        .parent_id(&liabilities_id)
        .call()
        .await?;

    let opening_balance_id = accounts
        .create()
        .name("OpeningBalance")
        .account_type(AccountType::Equity)
        .kind(AccountKind::DepositAccount)
        .parent_id(&equity_id)
        .call()
        .await?;

    let salary_id = accounts
        .create()
        .name("Salary")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .parent_id(&income_id)
        .call()
        .await?;

    let interest_id = accounts
        .create()
        .name("Interest")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .parent_id(&income_id)
        .call()
        .await?;

    let freelance_id = accounts
        .create()
        .name("Freelance")
        .account_type(AccountType::Income)
        .kind(AccountKind::DepositAccount)
        .parent_id(&income_id)
        .call()
        .await?;

    let groceries_id = accounts
        .create()
        .name("Groceries")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let dining_id = accounts
        .create()
        .name("Dining")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let utilities_id = accounts
        .create()
        .name("Utilities")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let electricity_id = accounts
        .create()
        .name("Electricity")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&utilities_id)
        .call()
        .await?;

    let water_id = accounts
        .create()
        .name("Water")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&utilities_id)
        .call()
        .await?;

    let drinking_water_id = accounts
        .create()
        .name("Drinking")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&water_id)
        .call()
        .await?;

    let sewer_id = accounts
        .create()
        .name("Sewer")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&water_id)
        .call()
        .await?;

    let transport_id = accounts
        .create()
        .name("Transport")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let subscriptions_id = accounts
        .create()
        .name("Subscriptions")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let healthcare_id = accounts
        .create()
        .name("Healthcare")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let entertainment_id = accounts
        .create()
        .name("Entertainment")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    let _uncategorised_id = accounts
        .create()
        .name("Uncategorised")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&expenses_id)
        .call()
        .await?;

    // =========================================================================
    // BUDGETS (7 total: one per expense leaf account)
    // =========================================================================

    let groceries_budget = budgets
        .create()
        .account_id(groceries_id.clone())
        .name("Groceries")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let utilities_budget = budgets
        .create()
        .account_id(utilities_id.clone())
        .name("Utilities")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let transport_budget = budgets
        .create()
        .account_id(transport_id.clone())
        .name("Transport")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let dining_budget = budgets
        .create()
        .account_id(dining_id.clone())
        .name("Dining")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let entertainment_budget = budgets
        .create()
        .account_id(entertainment_id.clone())
        .name("Entertainment")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let subscriptions_budget = budgets
        .create()
        .account_id(subscriptions_id.clone())
        .name("Subscriptions")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    let healthcare_budget = budgets
        .create()
        .account_id(healthcare_id.clone())
        .name("Healthcare")
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .call()
        .await?;

    // =========================================================================
    // ALLOCATIONS (49 total: 7 budgets × 7 months)
    // =========================================================================

    for months_ago in (0_i64..=6_i64).rev() {
        let period_start = month_start(months_ago);
        budgets
            .allocate(groceries_budget.id(), period_start, aud(dec!(600.00)))
            .await?;
        budgets
            .allocate(utilities_budget.id(), period_start, aud(dec!(350.00)))
            .await?;
        budgets
            .allocate(transport_budget.id(), period_start, aud(dec!(200.00)))
            .await?;
        budgets
            .allocate(dining_budget.id(), period_start, aud(dec!(300.00)))
            .await?;
        budgets
            .allocate(entertainment_budget.id(), period_start, aud(dec!(150.00)))
            .await?;
        budgets
            .allocate(subscriptions_budget.id(), period_start, aud(dec!(60.00)))
            .await?;
        budgets
            .allocate(healthcare_budget.id(), period_start, aud(dec!(200.00)))
            .await?;
    }

    // =========================================================================
    // TRANSACTIONS (~79 total across 6 historical months + current month)
    // =========================================================================

    macro_rules! txn {
        ($date:expr, $payee:expr, $desc:expr, $status:expr,
         $debit_acct:expr, $debit_amt:expr,
         $credit_acct:expr, $credit_amt:expr) => {
            transactions
                .create(
                    Transaction::builder()
                        .id(TransactionId::new())
                        .date($date)
                        .payee($payee)
                        .description($desc)
                        .status($status)
                        .created_at(Timestamp::now())
                        .postings(vec![
                            posting($debit_acct, aud($debit_amt)),
                            posting($credit_acct, aud($credit_amt)),
                        ])
                        .build(),
                )
                .await?
        };
    }

    // -------------------------------------------------------------------------
    // Opening balances (6 months ago, day 1)
    // -------------------------------------------------------------------------

    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Checking opening balance",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(3500.00),
        &opening_balance_id,
        dec!(-3500.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Savings opening balance",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(8000.00),
        &opening_balance_id,
        dec!(-8000.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Credit card opening balance",
        TransactionStatus::Cleared,
        &opening_balance_id,
        dec!(450.00),
        &credit_card_id,
        dec!(-450.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Car loan opening balance",
        TransactionStatus::Cleared,
        &opening_balance_id,
        dec!(12000.00),
        &car_loan_id,
        dec!(-12000.00)
    );

    // -------------------------------------------------------------------------
    // 6 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(6, 5),
        "Client A",
        "November freelance payment",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(800.00),
        &freelance_id,
        dec!(-800.00)
    );
    txn!(
        month_day(6, 15),
        "Employer Ltd",
        "November paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(6, 20),
        "Visa",
        "November credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(6, 25),
        "Transfer",
        "November savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(6, 30),
        "Bank",
        "November savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(9.50),
        &interest_id,
        dec!(-9.50)
    );
    txn!(
        month_day(6, 1),
        "Car Finance",
        "November car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(6, 3),
        "Woolworths",
        "November groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(6, 14),
        "Coles",
        "November fortnightly groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(6, 22),
        "IGA",
        "November grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(6, 8),
        "The Local Bistro",
        "November dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(6, 20),
        "The Coffee Club",
        "November coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(6, 12),
        "AGL Energy",
        "November electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(6, 12),
        "Telstra",
        "November internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(6, 28),
        "Origin Energy",
        "November gas bill",
        TransactionStatus::Cleared,
        &sewer_id,
        dec!(130.00),
        &checking_id,
        dec!(-130.00)
    );
    txn!(
        month_day(6, 18),
        "Opal Card",
        "November transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(6, 3),
        "Netflix",
        "November streaming subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(6, 3),
        "Spotify",
        "November music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(6, 10),
        "iCloud",
        "November cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );

    // -------------------------------------------------------------------------
    // 5 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(5, 15),
        "Employer Ltd",
        "December paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(5, 20),
        "Visa",
        "December credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(5, 25),
        "Transfer",
        "December savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(5, 31),
        "Bank",
        "December savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(10.00),
        &interest_id,
        dec!(-10.00)
    );
    txn!(
        month_day(5, 1),
        "Car Finance",
        "December car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(5, 3),
        "Woolworths",
        "December groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(5, 14),
        "Coles",
        "December fortnightly groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(5, 22),
        "IGA",
        "December grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(5, 8),
        "The Local Bistro",
        "December dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(5, 20),
        "The Coffee Club",
        "December coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(5, 22),
        "Fine Dining Co",
        "Christmas dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(210.00),
        &credit_card_id,
        dec!(-210.00)
    );
    txn!(
        month_day(5, 31),
        "NYE Restaurant",
        "New Year's Eve dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(175.00),
        &credit_card_id,
        dec!(-175.00)
    );
    txn!(
        month_day(5, 12),
        "AGL Energy",
        "December electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(5, 12),
        "Telstra",
        "December internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(5, 18),
        "Opal Card",
        "December transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(5, 3),
        "Netflix",
        "December streaming subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(5, 3),
        "Spotify",
        "December music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(5, 10),
        "iCloud",
        "December cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(5, 15),
        "Event Cinemas",
        "December cinema",
        TransactionStatus::Cleared,
        &entertainment_id,
        dec!(45.00),
        &credit_card_id,
        dec!(-45.00)
    );
    txn!(
        month_day(5, 20),
        "Live Nation",
        "December concert",
        TransactionStatus::Cleared,
        &entertainment_id,
        dec!(120.00),
        &credit_card_id,
        dec!(-120.00)
    );

    // -------------------------------------------------------------------------
    // 4 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(4, 15),
        "Employer Ltd",
        "January paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(4, 20),
        "Visa",
        "January credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(4, 25),
        "Transfer",
        "January savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(4, 31),
        "Bank",
        "January savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(10.50),
        &interest_id,
        dec!(-10.50)
    );
    txn!(
        month_day(4, 1),
        "Car Finance",
        "January car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(4, 3),
        "Woolworths",
        "January groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(190.00),
        &credit_card_id,
        dec!(-190.00)
    );
    txn!(
        month_day(4, 14),
        "Coles",
        "January fortnightly groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(4, 22),
        "IGA",
        "January grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(4, 28),
        "Harris Farm",
        "January organic groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(80.00),
        &credit_card_id,
        dec!(-80.00)
    );
    txn!(
        month_day(4, 8),
        "The Local Bistro",
        "January dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(4, 20),
        "The Coffee Club",
        "January coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(4, 12),
        "AGL Energy",
        "January electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(4, 12),
        "Telstra",
        "January internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(4, 18),
        "Opal Card",
        "January transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(4, 3),
        "Netflix",
        "January streaming subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(4, 3),
        "Spotify",
        "January music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(4, 10),
        "iCloud",
        "January cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(4, 10),
        "City Medical Centre",
        "January GP visit",
        TransactionStatus::Cleared,
        &healthcare_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(4, 11),
        "Chemist Warehouse",
        "January pharmacy",
        TransactionStatus::Cleared,
        &healthcare_id,
        dec!(32.50),
        &credit_card_id,
        dec!(-32.50)
    );

    let voided_jan_paycheck = transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(4, 15))
                .payee("Employer Ltd")
                .description("January paycheck — duplicate (to be voided)")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&checking_id, aud(dec!(5200.00))),
                    posting(&salary_id, aud(dec!(-5200.00))),
                ])
                .build(),
        )
        .await?;
    transactions.void(&voided_jan_paycheck).await?;

    // -------------------------------------------------------------------------
    // 3 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(3, 15),
        "Employer Ltd",
        "February paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(3, 20),
        "Visa",
        "February credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(3, 25),
        "Transfer",
        "February savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(3, 28),
        "Bank",
        "February savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(11.00),
        &interest_id,
        dec!(-11.00)
    );
    txn!(
        month_day(3, 1),
        "Car Finance",
        "February car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(3, 3),
        "Woolworths",
        "February groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(3, 14),
        "Coles",
        "February fortnightly groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(3, 22),
        "IGA",
        "February grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(3, 8),
        "The Local Bistro",
        "February dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(3, 20),
        "The Coffee Club",
        "February coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(3, 12),
        "AGL Energy",
        "February electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(3, 12),
        "Telstra",
        "February internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(3, 5),
        "BP Service Station",
        "February petrol",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(3, 18),
        "Opal Card",
        "February transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(3, 3),
        "Netflix",
        "February streaming subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(3, 3),
        "Spotify",
        "February music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(3, 10),
        "iCloud",
        "February cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(3, 20),
        "Client B",
        "February freelance payment",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(1200.00),
        &freelance_id,
        dec!(-1200.00)
    );

    let voided_feb_woolworths = transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(3, 3))
                .payee("Woolworths")
                .description("February groceries — duplicate (to be voided)")
                .status(TransactionStatus::Pending)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&groceries_id, aud(dec!(140.00))),
                    posting(&credit_card_id, aud(dec!(-140.00))),
                ])
                .build(),
        )
        .await?;
    transactions.void(&voided_feb_woolworths).await?;

    // -------------------------------------------------------------------------
    // 2 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(2, 15),
        "Employer Ltd",
        "March paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(2, 20),
        "Visa",
        "March credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(2, 25),
        "Transfer",
        "March savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1200.00),
        &checking_id,
        dec!(-1200.00)
    );
    txn!(
        month_day(2, 31),
        "Bank",
        "March savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(11.50),
        &interest_id,
        dec!(-11.50)
    );
    txn!(
        month_day(2, 1),
        "Car Finance",
        "March car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(2, 3),
        "Woolworths",
        "March groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(2, 14),
        "Coles",
        "March fortnightly groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(2, 22),
        "IGA",
        "March grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(2, 8),
        "The Local Bistro",
        "March dinner",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(2, 20),
        "The Coffee Club",
        "March coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(2, 12),
        "AGL Energy",
        "March electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(2, 12),
        "Telstra",
        "March internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(2, 28),
        "Origin Energy",
        "March gas bill",
        TransactionStatus::Cleared,
        &sewer_id,
        dec!(130.00),
        &checking_id,
        dec!(-130.00)
    );
    txn!(
        month_day(2, 18),
        "Opal Card",
        "March transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(2, 3),
        "Netflix",
        "March streaming subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(2, 3),
        "Spotify",
        "March music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(2, 10),
        "iCloud",
        "March cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );

    // -------------------------------------------------------------------------
    // 1 month ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(1, 15),
        "Employer Ltd",
        "April paycheck",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(1, 20),
        "Visa",
        "April credit card payment",
        TransactionStatus::Cleared,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(1, 25),
        "Transfer",
        "April savings transfer",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(1, 30),
        "Bank",
        "April savings interest",
        TransactionStatus::Cleared,
        &savings_id,
        dec!(12.00),
        &interest_id,
        dec!(-12.00)
    );
    txn!(
        month_day(1, 1),
        "Car Finance",
        "April car loan repayment",
        TransactionStatus::Cleared,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(1, 3),
        "Woolworths",
        "April groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(1, 14),
        "Coles",
        "April fortnightly groceries",
        TransactionStatus::Pending,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(1, 22),
        "IGA",
        "April grocery top-up",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(1, 8),
        "The Local Bistro",
        "April dinner",
        TransactionStatus::Pending,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(1, 20),
        "The Coffee Club",
        "April coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(1, 12),
        "AGL Energy",
        "April electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(1, 12),
        "Telstra",
        "April internet bill",
        TransactionStatus::Cleared,
        &drinking_water_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(1, 18),
        "Opal Card",
        "April transit top-up",
        TransactionStatus::Cleared,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(1, 3),
        "Netflix",
        "April streaming subscription",
        TransactionStatus::Pending,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(1, 3),
        "Spotify",
        "April music subscription",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(1, 10),
        "iCloud",
        "April cloud storage",
        TransactionStatus::Cleared,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );

    // -------------------------------------------------------------------------
    // Current month
    // -------------------------------------------------------------------------

    txn!(
        month_day(0, 1),
        "Opening Balance",
        "Opening balance adjustment",
        TransactionStatus::Cleared,
        &checking_id,
        dec!(5200),
        &salary_id,
        dec!(-5200)
    );
    txn!(
        month_day(0, 3),
        "Supermarket",
        "Groceries",
        TransactionStatus::Cleared,
        &groceries_id,
        dec!(95),
        &checking_id,
        dec!(-95)
    );
    txn!(
        month_day(0, 4),
        "The Coffee Club",
        "Coffee",
        TransactionStatus::Cleared,
        &dining_id,
        dec!(6.50),
        &credit_card_id,
        dec!(-6.50)
    );
    txn!(
        month_day(0, 2),
        "Power Company",
        "Electricity bill",
        TransactionStatus::Cleared,
        &electricity_id,
        dec!(120),
        &checking_id,
        dec!(-120)
    );

    println!("Done.");
    println!("Created database at {}", args.db_path.display());
    println!("Accounts:     26 (5 root + 21 leaf)");
    println!("Budgets:       7 (one per expense leaf account)");
    println!("Allocations:  49 (7 budgets × 7 months)");
    println!(
        "Transactions: ~79 (cleared, pending, voided across 6 historical months + current month)"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn month_start_matches_this_month_window() {
        let today = jiff::Zoned::now().date();
        assert_eq!(month_start(0), BudgetWindow::this_month(today).start);
    }

    #[test]
    fn month_start_one_matches_last_month_window() {
        let today = jiff::Zoned::now().date();
        assert_eq!(month_start(1), BudgetWindow::last_month(today).start);
    }

    #[test]
    fn month_day_offset_lands_on_correct_date() {
        let start = month_start(0);
        assert_eq!(
            month_day(0, 15),
            start.saturating_add(jiff::Span::new().days(14_i64))
        );
    }

    #[test]
    fn aud_constructs_correct_amount() {
        assert_eq!(
            aud(dec!(42.50)),
            Amount::new(dec!(42.50), CommodityCode::new("AUD"))
        );
    }
}
